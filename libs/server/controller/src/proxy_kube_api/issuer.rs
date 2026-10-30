//! The controller's side of an `Issuer` cluster (RFC 0014 §5.3): it owns the
//! signing keys, `proxyauth-issuer-<UID>` in the proxy's namespace — first
//! key, rotation, revocation, purge — and sweeps the Secrets of clusters that
//! are gone. The proxy replicas only read them. In the effective mode a
//! problem is `Ready=False` (the proxy then answers `503`); while a
//! `Passthrough` cluster with `issuer_key` prepares the switch it is the
//! advisory `IssuerNotReady`.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use common::State;
use crd::ProxyKubeApi;
use crd::service::Service;
use crd::upstream_auth::front_proxy::{MANAGED_BY_LABEL, MANAGED_BY_VALUE};
use crd::upstream_auth::issuer_key::{
    self, CLUSTER_ANNOTATION, CLUSTER_UID_LABEL, IssuerKey, IssuerKeySet, KEYS_KEY,
    ORPHANED_ANNOTATION, PROCESSED_ANNOTATION, Plan, ROTATE_ANNOTATION, Rotation,
};
use crd::upstream_auth::{UpstreamAuthMode, UpstreamAuthStatus};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use kube::api::{DeleteParams, ListParams, PostParams, Preconditions};
use kube::{Api, ResourceExt};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use tracing::{info, warn};
use weebo_kit_domain::Advisory;

use super::front_proxy::{judge_trust, record_trust, refuse_as};
use super::reconcile::Evaluation;
use crate::reason::ReasonCode;

/// How often the leader looks for orphaned key Secrets.
const SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// An orphan is deleted this long after it was first marked.
const ORPHAN_GRACE_SECONDS: i64 = 24 * 3600;

fn now() -> i64 {
    k8s_openapi::jiff::Timestamp::now().as_second()
}

fn rfc3339(at: i64) -> Option<String> {
    k8s_openapi::jiff::Timestamp::from_second(at)
        .ok()
        .map(|t| t.to_string())
}

/// A new P-256 key: its PKCS#8, its public point and its RFC 7638 `kid`.
fn new_key(created_at: i64, activate_at: i64) -> Result<IssuerKey, String> {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
        .map_err(|_| "cannot generate a P-256 key".to_string())?;
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
        .map_err(|_| "cannot read the P-256 key just generated".to_string())?;
    // An uncompressed SEC1 point: 0x04, then x and y, 32 bytes each.
    let point = pair.public_key().as_ref();
    let (Some(x), Some(y)) = (point.get(1..33), point.get(33..65)) else {
        return Err("unexpected P-256 public key".to_string());
    };
    let (x, y) = (URL_SAFE_NO_PAD.encode(x), URL_SAFE_NO_PAD.encode(y));
    let thumbprint = format!(r#"{{"crv":"P-256","kty":"EC","x":"{x}","y":"{y}"}}"#);
    let kid = URL_SAFE_NO_PAD.encode(ring::digest::digest(
        &ring::digest::SHA256,
        thumbprint.as_bytes(),
    ));
    Ok(IssuerKey {
        kid,
        pkcs8: STANDARD.encode(pkcs8.as_ref()),
        x,
        y,
        created_at,
        activate_at,
        retire_at: None,
    })
}

/// The key Secret of `proxy`, as the controller writes it.
fn key_secret(
    proxy: &ProxyKubeApi,
    namespace: &str,
    name: &str,
    uid: &str,
    keys: &IssuerKeySet,
    processed: Option<&str>,
) -> Result<Secret, String> {
    let json = serde_json::to_vec(keys).map_err(|e| format!("cannot encode {KEYS_KEY}: {e}"))?;
    let mut annotations = BTreeMap::from([(CLUSTER_ANNOTATION.to_string(), proxy.to_path())]);
    if let Some(processed) = processed {
        annotations.insert(PROCESSED_ANNOTATION.to_string(), processed.to_string());
    }
    let mut secret = Secret {
        type_: Some("Opaque".to_string()),
        data: Some(BTreeMap::from([(KEYS_KEY.to_string(), ByteString(json))])),
        ..Secret::default()
    };
    secret.metadata.name = Some(name.to_string());
    secret.metadata.namespace = Some(namespace.to_string());
    secret.metadata.labels = Some(issuer_key::labels(uid));
    secret.metadata.annotations = Some(annotations);
    Ok(secret)
}

/// Read, plan and write `proxy`'s keys; the set in force afterwards. The
/// errors name the Secret, never its content.
async fn ensure_keys(proxy: &ProxyKubeApi, ctx: &State) -> Result<IssuerKeySet, String> {
    let settings = proxy
        .spec
        .upstream_auth
        .as_ref()
        .and_then(|upstream| upstream.issuer_key.as_ref())
        .ok_or("no issuer_key")?;
    let uid = proxy.uid().ok_or("the ProxyKubeApi has no UID yet")?;
    let name = issuer_key::secret_name(proxy).map_err(|e| e.to_string())?;
    let namespace = ctx.lease_namespace.as_str();
    let api: Api<Secret> = Api::namespaced(ctx.client.clone(), namespace);
    let rotation = Rotation {
        rotation_days: settings.rotation_days,
        token_ttl_seconds: settings.token_ttl_seconds,
    };
    let request = proxy
        .annotations()
        .get(ROTATE_ANNOTATION)
        .map(String::as_str);
    let at = now();
    let existing = api
        .get_opt(&name)
        .await
        .map_err(|e| format!("cannot read Secret {namespace}/{name}: {e}"))?;
    let Some(existing) = existing else {
        let plan = issuer_key::plan(
            &IssuerKeySet::default(),
            at,
            rotation,
            request,
            None,
            &mut new_key,
        )?;
        let Plan::Write { keys, processed } = plan else {
            return Err(format!("no key to write in Secret {namespace}/{name}"));
        };
        let secret = key_secret(proxy, namespace, &name, &uid, &keys, processed.as_deref())?;
        api.create(&PostParams::default(), &secret)
            .await
            .map_err(|e| format!("cannot create Secret {namespace}/{name}: {e}"))?;
        info!(cluster = %proxy.to_path(), secret = %name, "created the Issuer key");
        return Ok(keys);
    };
    let current = issuer_key::read(&existing, &uid).map_err(|e| e.to_string())?;
    let processed = existing
        .annotations()
        .get(PROCESSED_ANNOTATION)
        .map(String::as_str);
    match issuer_key::plan(&current, at, rotation, request, processed, &mut new_key)? {
        Plan::Keep => Ok(current),
        Plan::Write { keys, processed } => {
            let mut secret =
                key_secret(proxy, namespace, &name, &uid, &keys, processed.as_deref())?;
            // Optimistic concurrency: two overlapping leaders never both
            // add a key.
            secret.metadata.resource_version = existing.resource_version();
            api.replace(&name, &PostParams::default(), &secret)
                .await
                .map_err(|e| format!("cannot update Secret {namespace}/{name}: {e}"))?;
            info!(cluster = %proxy.to_path(), secret = %name, "updated the Issuer keys");
            Ok(keys)
        }
    }
}

/// The `Issuer` checks of `proxy`, folded into `evaluation`.
pub async fn evaluate(proxy: &ProxyKubeApi, ctx: &State, evaluation: &mut Evaluation) {
    let Some(settings) = proxy
        .spec
        .upstream_auth
        .as_ref()
        .and_then(|upstream| upstream.issuer_key.as_ref())
    else {
        return;
    };
    let effective = proxy.upstream_mode() == UpstreamAuthMode::Issuer;
    // Only an effective failure overrides a Ready that already failed.
    let refuse = |evaluation: &mut Evaluation, code: ReasonCode, message: String| {
        if !(effective && evaluation.ready.is_err()) {
            refuse_as(
                evaluation,
                effective,
                code,
                ReasonCode::IssuerNotReady,
                message,
            );
        }
    };
    let audience = proxy.to_identifier();
    let mut status = UpstreamAuthStatus {
        mode: proxy.upstream_mode(),
        issuer_url: Some(settings.issuer_url.clone()),
        audience: Some(audience.clone()),
        jwks_uri: Some(issuer_key::jwks_uri(&settings.issuer_url)),
        ..UpstreamAuthStatus::default()
    };
    if !issuer_key::issuer_url_matches(proxy, &settings.issuer_url) {
        refuse(
            evaluation,
            ReasonCode::IssuerUrlMismatch,
            format!(
                "issuer_key.issuer_url must end with {}, the route the proxy serves",
                issuer_key::issuer_path(proxy)
            ),
        );
    }
    if matches!(proxy.spec.service, Service::Agent(_)) {
        evaluation.advisories.push(Advisory {
            reason: ReasonCode::IssuerBehindAgent,
            message: "the apiserver behind the agent must still reach the proxy's public URL for the JWKS".to_string(),
        });
    }
    match ensure_keys(proxy, ctx).await {
        Ok(keys) => {
            let at = now();
            if let Some(key) = keys.current(at) {
                status.key_id = Some(key.kid.clone());
                status.key_rotated_at = rfc3339(key.activate_at);
                let overdue = key.activate_at.saturating_add(
                    i64::from(settings.rotation_days.saturating_add(1)).saturating_mul(86_400),
                );
                if at >= overdue {
                    evaluation.advisories.push(Advisory {
                        reason: ReasonCode::IssuerKeyRotationOverdue,
                        message: format!(
                            "the Issuer key {} has not rotated for more than {} days",
                            key.kid, settings.rotation_days
                        ),
                    });
                }
            }
        }
        Err(message) => {
            warn!(cluster = %proxy.to_path(), %message, "the Issuer key is unavailable");
            refuse(evaluation, ReasonCode::IssuerKeyUnavailable, message);
        }
    }
    let verdict = record_trust(proxy, &mut status);
    evaluation.status.upstream_auth = Some(status);
    if effective && evaluation.ready.is_err() {
        return;
    }
    judge_trust(
        verdict,
        evaluation,
        effective,
        ReasonCode::IssuerNotReady,
        (
            ReasonCode::IssuerNotTrusted,
            format!(
                "the apiserver refuses the proxy's JWT (401): declare the issuer {} with the audience {audience} in its AuthenticationConfiguration",
                settings.issuer_url
            ),
        ),
    );
}

/// What the sweep does with a key Secret whose cluster is gone.
#[derive(Debug, PartialEq, Eq)]
enum Orphan {
    Mark,
    Wait,
    Delete,
}

fn orphan_action(orphaned_since: Option<&str>, now: i64) -> Orphan {
    let since = orphaned_since
        .and_then(|value| value.parse::<k8s_openapi::jiff::Timestamp>().ok())
        .map(|t| t.as_second());
    match since {
        None => Orphan::Mark,
        Some(since) if now.saturating_sub(since) >= ORPHAN_GRACE_SECONDS => Orphan::Delete,
        Some(_) => Orphan::Wait,
    }
}

/// One sweep: only after a complete, successful list of every
/// `ProxyKubeApi` (an error sweeps nothing), mark the key Secrets of
/// clusters that are gone, then delete them 24 h later if still orphaned,
/// with preconditions so a Secret rewritten meanwhile survives.
async fn sweep(state: &State) -> Result<(), kube::Error> {
    let proxies = Api::<ProxyKubeApi>::all(state.client.clone())
        .list(&ListParams::default())
        .await?;
    let alive: HashSet<String> = proxies.items.iter().filter_map(ResourceExt::uid).collect();
    let api: Api<Secret> = Api::namespaced(state.client.clone(), &state.lease_namespace);
    let selector = format!("{MANAGED_BY_LABEL}={MANAGED_BY_VALUE},{CLUSTER_UID_LABEL}");
    let secrets = api.list(&ListParams::default().labels(&selector)).await?;
    let at = now();
    for secret in secrets.items {
        let name = secret.name_any();
        let Some(uid) = secret.labels().get(CLUSTER_UID_LABEL) else {
            continue;
        };
        if alive.contains(uid) || !name.starts_with("proxyauth-issuer-") {
            continue;
        }
        let since = secret
            .annotations()
            .get(ORPHANED_ANNOTATION)
            .map(String::as_str);
        match orphan_action(since, at) {
            Orphan::Wait => {}
            Orphan::Mark => {
                let mut marked = secret.clone();
                marked.annotations_mut().insert(
                    ORPHANED_ANNOTATION.to_string(),
                    rfc3339(at).unwrap_or_default(),
                );
                api.replace(&name, &PostParams::default(), &marked).await?;
                info!(secret = %name, "the Issuer key's cluster is gone; deleted in 24 h if it stays so");
            }
            Orphan::Delete => {
                let params = DeleteParams {
                    preconditions: Some(Preconditions {
                        uid: secret.uid(),
                        resource_version: secret.resource_version(),
                    }),
                    ..DeleteParams::default()
                };
                api.delete(&name, &params).await?;
                info!(secret = %name, "deleted the Issuer key of a cluster gone for 24 h");
            }
        }
    }
    Ok(())
}

/// The sweep, on the leader, every [`SWEEP_INTERVAL`].
pub async fn run_sweep(state: Arc<State>) {
    loop {
        if state.is_leader.load(std::sync::atomic::Ordering::Relaxed)
            && let Err(error) = sweep(&state).await
        {
            warn!(%error, "the Issuer key sweep failed; nothing swept");
        }
        tokio::time::sleep(SWEEP_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_key_is_p256_with_a_thumbprint_kid() {
        let key = new_key(10, 20).expect("a key");
        assert_eq!(URL_SAFE_NO_PAD.decode(&key.x).expect("x").len(), 32);
        assert_eq!(URL_SAFE_NO_PAD.decode(&key.y).expect("y").len(), 32);
        assert_eq!(URL_SAFE_NO_PAD.decode(&key.kid).expect("kid").len(), 32);
        assert_ne!(new_key(10, 20).expect("a key").kid, key.kid);
        let pkcs8 = STANDARD.decode(&key.pkcs8).expect("pkcs8");
        let rng = SystemRandom::new();
        assert!(EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8, &rng).is_ok());
    }

    #[test]
    fn an_orphan_is_marked_then_deleted_a_day_later() {
        let marked = rfc3339(1_000).expect("time");
        assert_eq!(orphan_action(None, 1_000), Orphan::Mark);
        assert_eq!(orphan_action(Some(&marked), 1_000 + 3_600), Orphan::Wait);
        assert_eq!(
            orphan_action(Some(&marked), 1_000 + ORPHAN_GRACE_SECONDS),
            Orphan::Delete
        );
        // An unreadable mark starts the grace again, never deletes.
        assert_eq!(orphan_action(Some("yesterday"), 1_000), Orphan::Mark);
    }
}
