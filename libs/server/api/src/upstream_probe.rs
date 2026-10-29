//! The trust probe (RFC 0014 §5.4, decisions 33, 41, 45): on the replica
//! holding the leader Lease, a `SelfSubjectReview` per trusted cluster
//! (effective, or preparing the switch) with a probe identity — in the
//! `X-Remote-*` headers on the connection presenting the `FrontProxy`
//! certificate, or in a JWT the proxy signs for `Issuer`, followed by an
//! unprefixed one the apiserver must refuse. The answer goes to
//! [`common::upstream_trust`], where the controller of the same process
//! reads it.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::State;
use common::upstream_trust::{self, Trust};
use crd::ProxyKubeApi;
use crd::upstream_auth::{UpstreamAuth, UpstreamAuthMode};
use kube::ResourceExt as _;
use tracing::{debug, warn};

use crate::cluster::redirect::kube_redirect::{ca_only_client, probe_client};
use crate::cluster::redirect::upstream_identity::{Upstream, UpstreamIdentity};

/// Between two rounds. The RFC asks for 5 minutes and one probe per change;
/// without a change feed, a shorter round stands in for the second.
const ROUND: Duration = Duration::from_secs(60);
/// A probe's total budget: the upstream client only bounds the connect.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// The `extra` key the probe asserts, for a header the apiserver must read.
const PROBE_EXTRA: &str = "proxyauth.weebo.si.rs/probe";

/// Whether `proxy` is probed: `FrontProxy` or `Issuer`, or `Passthrough`
/// with both prefixes (preparing one of them; `issuer_key` says which).
fn probed(proxy: &ProxyKubeApi) -> Option<&UpstreamAuth> {
    let upstream = proxy.spec.upstream_auth.as_ref()?;
    let prefixed = upstream
        .user_prefix
        .as_deref()
        .is_some_and(|p| !p.is_empty())
        && upstream
            .group_prefix
            .as_deref()
            .is_some_and(|p| !p.is_empty());
    match upstream.mode {
        UpstreamAuthMode::FrontProxy | UpstreamAuthMode::Issuer => Some(upstream),
        UpstreamAuthMode::Passthrough if prefixed => Some(upstream),
        UpstreamAuthMode::Passthrough => None,
    }
}

/// The identity the probe asserts under `upstream`'s prefixes.
fn probe_identity(upstream: &UpstreamAuth) -> UpstreamIdentity {
    let user_prefix = upstream.user_prefix.as_deref().unwrap_or_default();
    let group_prefix = upstream.group_prefix.as_deref().unwrap_or_default();
    UpstreamIdentity {
        username: format!("{user_prefix}proxyauth-probe"),
        groups: vec![format!("{group_prefix}proxyauth-probe")],
        extra: BTreeMap::from([(PROBE_EXTRA.to_string(), vec!["1".to_string()])]),
    }
}

/// What the `SelfSubjectReview` answer says about the asserted identity.
fn judge(status: u16, body: &serde_json::Value, asserted: &UpstreamIdentity) -> Trust {
    match status {
        401 => Trust::Rejected,
        200..=299 => {
            let user = body.pointer("/status/userInfo");
            let field = |name: &str| user.and_then(|user| user.get(name));
            let seen = field("username")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let has_group = field("groups")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|groups| {
                    groups
                        .iter()
                        .any(|g| g.as_str() == asserted.groups.first().map(String::as_str))
                });
            // Only what was asserted must come back: the `Issuer` probe
            // asserts no `extra`, the apiserver maps it per key.
            let has_extra = asserted.extra.is_empty()
                || field("extra")
                    .and_then(|extra| extra.get(PROBE_EXTRA))
                    .is_some_and(serde_json::Value::is_array);
            if seen == asserted.username && has_group && has_extra {
                Trust::Trusted
            } else {
                Trust::NotHonoured(format!(
                    "{seen}{}{}",
                    if has_group {
                        ""
                    } else {
                        ", without the asserted group"
                    },
                    if has_extra {
                        ""
                    } else {
                        ", without the asserted extra"
                    }
                ))
            }
        }
        other => Trust::Unknown(format!("the apiserver answered {other}")),
    }
}

/// A `SelfSubjectReview` on `client` with `headers`: its status and body.
async fn review(
    client: &reqwest::Client,
    base_url: &str,
    headers: Vec<(String, String)>,
) -> Result<(u16, serde_json::Value), String> {
    let mut request = client
        .post(format!(
            "{base_url}/apis/authentication.k8s.io/v1/selfsubjectreviews"
        ))
        .timeout(PROBE_TIMEOUT)
        .json(&serde_json::json!({
            "apiVersion": "authentication.k8s.io/v1",
            "kind": "SelfSubjectReview",
        }));
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    Ok((status, response.json().await.unwrap_or_default()))
}

/// One probe of `proxy`.
async fn probe(state: &State, proxy: &ProxyKubeApi, upstream: &UpstreamAuth) -> Trust {
    let issuer = upstream.issuer_key.is_some();
    let mut identity = probe_identity(upstream);
    let client = if issuer {
        // x509 passes before the JWT: never a client certificate here.
        identity.extra.clear();
        ca_only_client(proxy, state).await
    } else {
        probe_client(proxy, state).await
    };
    let client = match client {
        Ok(client) => client,
        Err(e) => return Trust::Unknown(e),
    };
    let base_url = match proxy
        .spec
        .service
        .url_to_call(state.client.clone(), proxy.namespace().unwrap_or_default())
        .await
    {
        Ok(url) => url.trim_end_matches('/').to_string(),
        Err(e) => return Trust::Unknown(e.to_string()),
    };
    if !issuer {
        return match review(&client, &base_url, Upstream::Headers(&identity).headers()).await {
            Ok((status, body)) => judge(status, &body, &identity),
            Err(e) => Trust::Unknown(e),
        };
    }
    let token = match crate::issuer::sign(state, proxy, &identity, None).await {
        Ok(token) => token,
        Err(e) => return Trust::Unknown(e),
    };
    let trust = match review(
        &client,
        &base_url,
        Upstream::Bearer(&identity, &token).headers(),
    )
    .await
    {
        Ok((status, body)) => judge(status, &body, &identity),
        Err(e) => return Trust::Unknown(e),
    };
    if trust != Trust::Trusted {
        return trust;
    }
    // The apiserver must check the prefix itself (decision 28): an
    // unprefixed identity from this issuer is refused, else the
    // `userValidationRules` are missing.
    let bare = UpstreamIdentity {
        username: "proxyauth-probe".to_string(),
        groups: vec!["proxyauth-probe".to_string()],
        extra: BTreeMap::new(),
    };
    let token = match crate::issuer::sign(state, proxy, &bare, None).await {
        Ok(token) => token,
        Err(e) => return Trust::Unknown(e),
    };
    match review(
        &client,
        &base_url,
        Upstream::Bearer(&bare, &token).headers(),
    )
    .await
    {
        Ok((401, _)) => Trust::Trusted,
        Ok((200..=299, _)) => Trust::Unguarded,
        Ok((other, _)) => Trust::Unknown(format!(
            "the apiserver answered {other} to an unprefixed identity"
        )),
        Err(e) => Trust::Unknown(e),
    }
}

/// The probe loop: a round per [`ROUND`], on the leader only.
pub async fn run(state: State) {
    let state = Arc::new(state);
    loop {
        if state.is_leader.load(Ordering::Relaxed) {
            match state.list_proxies().await {
                Ok(proxies) => {
                    for proxy in &proxies {
                        let Some(upstream) = probed(proxy) else {
                            continue;
                        };
                        let trust = probe(&state, proxy, upstream).await;
                        debug!(cluster = %proxy.to_path(), ?trust, "upstream trust probed");
                        upstream_trust::record(&proxy.to_path(), trust);
                    }
                }
                Err(error) => warn!(%error, "the trust probe could not list the clusters"),
            }
        }
        tokio::time::sleep(ROUND).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn asserted() -> UpstreamIdentity {
        probe_identity(&UpstreamAuth {
            mode: UpstreamAuthMode::FrontProxy,
            user_prefix: Some("p:".into()),
            group_prefix: Some("p:".into()),
            issuer_key: None,
        })
    }

    fn review(username: &str, groups: &[&str], extra: bool) -> serde_json::Value {
        let mut user = json!({ "username": username, "groups": groups });
        if extra {
            user["extra"] = json!({ PROBE_EXTRA: ["1"] });
        }
        json!({ "status": { "userInfo": user } })
    }

    #[test]
    fn the_asserted_identity_seen_whole_is_trusted() {
        let asserted = asserted();
        assert_eq!(
            judge(
                201,
                &review(
                    "p:proxyauth-probe",
                    &["p:proxyauth-probe", "system:authenticated"],
                    true
                ),
                &asserted
            ),
            Trust::Trusted
        );
    }

    /// T7: the CN of a `--client-ca-file` certificate, or a header the
    /// apiserver does not read, is seen and named.
    #[test]
    fn another_identity_or_a_lost_header_is_not_honoured() {
        let asserted = asserted();
        assert_eq!(
            judge(
                201,
                &review("proxyauth-it", &["system:authenticated"], false),
                &asserted
            ),
            Trust::NotHonoured(
                "proxyauth-it, without the asserted group, without the asserted extra".into()
            )
        );
        assert!(matches!(
            judge(
                201,
                &review("p:proxyauth-probe", &["p:proxyauth-probe"], false),
                &asserted
            ),
            Trust::NotHonoured(_)
        ));
    }

    /// Decision 45: a 401 is a broken trust; anything else, no conclusion.
    #[test]
    fn a_401_is_rejected_and_anything_else_unknown() {
        let asserted = asserted();
        assert_eq!(judge(401, &json!({}), &asserted), Trust::Rejected);
        assert!(matches!(
            judge(503, &json!({}), &asserted),
            Trust::Unknown(_)
        ));
    }
}
