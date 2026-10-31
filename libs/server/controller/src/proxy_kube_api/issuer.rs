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

    // The key Secret against a wiremock apiserver (RFC 0015 P3): what the
    // controller reads, writes, keeps and sweeps.
    mod against_an_apiserver {
        use super::*;
        use crd::upstream_auth::{IssuerKeySettings, UpstreamAuth};
        use serde_json::{Value, json};
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, Request, ResponseTemplate};

        const UID: &str = "u-alive";
        const SECRETS: &str = "/api/v1/namespaces/default/secrets";

        fn state(kube: &MockServer) -> State {
            drop(rustls::crypto::ring::default_provider().install_default());
            let config = kube::Config::new(kube.uri().parse().unwrap());
            State::from_parts(
                kube::Client::try_from(config).unwrap(),
                common::kv_memory::InMemoryKv::new(),
                common::oidc_conf::OidcConf {
                    client_id: "proxyauthk8s".to_string(),
                    client_secret: None,
                    issuer_url: "http://127.0.0.1:1".to_string(),
                    scopes: "openid".to_string(),
                    audience: "proxyauthk8s".to_string(),
                    accept_authorized_party: false,
                    redirect_url: None,
                },
                "https://proxy.example.com".to_string(),
                "https://front.example.com".to_string(),
            )
        }

        fn issuing(issuer_url: &str) -> ProxyKubeApi {
            let mut proxy = ProxyKubeApi::new(
                "issuing",
                serde_json::from_value(json!({
                    "cert": { "SystemRoots": true },
                    "service": { "ExternalService": { "url": "https://k:6443" } },
                }))
                .unwrap(),
            );
            proxy.metadata.namespace = Some("default".into());
            proxy.metadata.uid = Some(UID.into());
            proxy.spec.upstream_auth = Some(UpstreamAuth {
                mode: UpstreamAuthMode::Issuer,
                user_prefix: Some("proxyauth:".into()),
                group_prefix: Some("proxyauth:".into()),
                issuer_key: Some(IssuerKeySettings {
                    issuer_url: issuer_url.into(),
                    token_ttl_seconds: 300,
                    rotation_days: 30,
                }),
                token_exchange: None,
            });
            proxy
        }

        const ISSUER_URL: &str = "https://proxy.example.com/issuer/default/issuing";

        fn secret_json(uid: &str, keys: &IssuerKeySet, annotations: Value) -> Value {
            json!({
                "apiVersion": "v1",
                "kind": "Secret",
                "metadata": {
                    "name": format!("proxyauth-issuer-{uid}"),
                    "namespace": "default",
                    "uid": format!("secret-of-{uid}"),
                    "resourceVersion": "42",
                    "labels": issuer_key::labels(uid),
                    "annotations": annotations,
                },
                "data": { KEYS_KEY: STANDARD.encode(serde_json::to_vec(keys).unwrap()) },
            })
        }

        async fn serve_secret(kube: &MockServer, body: Value) {
            Mock::given(method("GET"))
                .and(path(format!("{SECRETS}/proxyauth-issuer-{UID}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(kube)
                .await;
        }

        fn not_found() -> ResponseTemplate {
            ResponseTemplate::new(404).set_body_json(json!({
                "kind": "Status", "apiVersion": "v1", "status": "Failure",
                "reason": "NotFound", "code": 404,
            }))
        }

        /// Echo the written object back, as the apiserver does.
        fn echo(status: u16) -> impl Fn(&Request) -> ResponseTemplate {
            move |request: &Request| {
                ResponseTemplate::new(status).set_body_bytes(request.body.clone())
            }
        }

        fn written(kube_requests: &[Request], verb: &str) -> Vec<Value> {
            kube_requests
                .iter()
                .filter(|r| r.method.as_str() == verb)
                .map(|r| serde_json::from_slice(&r.body).unwrap())
                .collect()
        }

        fn ready() -> Evaluation {
            Evaluation {
                ready: Ok("ok".into()),
                advisories: Vec::new(),
                status: crd::status::ProxyKubeApiStatus::default(),
            }
        }

        /// The first key: one Secret, labelled for this cluster only, its
        /// key active at once and named in the status.
        #[tokio::test]
        async fn the_first_key_secret_is_created_labelled_for_its_cluster() {
            let kube = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(format!("{SECRETS}/proxyauth-issuer-{UID}")))
                .respond_with(not_found())
                .mount(&kube)
                .await;
            Mock::given(method("POST"))
                .and(path(SECRETS))
                .respond_with(echo(201))
                .expect(1)
                .mount(&kube)
                .await;
            let mut evaluation = ready();
            evaluate(&issuing(ISSUER_URL), &state(&kube), &mut evaluation).await;

            let created = written(&kube.received_requests().await.unwrap(), "POST");
            let secret: Secret = serde_json::from_value(created[0].clone()).unwrap();
            assert!(issuer_key::owned(&secret, UID));
            assert_eq!(
                secret
                    .annotations()
                    .get(CLUSTER_ANNOTATION)
                    .map(String::as_str),
                Some("default/issuing")
            );
            let keys = issuer_key::read(&secret, UID).unwrap();
            let current = keys.current(now()).expect("active at once");
            assert!(evaluation.ready.is_ok());
            let status = evaluation.status.upstream_auth.expect("status");
            assert_eq!(status.key_id.as_deref(), Some(current.kid.as_str()));
            assert_eq!(
                status.jwks_uri.as_deref(),
                Some(&*format!("{ISSUER_URL}/jwks"))
            );
        }

        /// A set in force is left alone: no write on every reconcile.
        #[tokio::test]
        async fn a_current_key_set_is_kept_without_a_write() {
            let at = now();
            let keys = IssuerKeySet {
                keys: vec![new_key(at, at).unwrap()],
            };
            let kube = MockServer::start().await;
            serve_secret(&kube, secret_json(UID, &keys, json!({}))).await;
            Mock::given(method("PUT"))
                .respond_with(echo(200))
                .expect(0)
                .mount(&kube)
                .await;
            let kept = ensure_keys(&issuing(ISSUER_URL), &state(&kube))
                .await
                .unwrap();
            assert_eq!(kept, keys);
        }

        /// A key past `rotation_days` gets a successor, written with the
        /// `resourceVersion` read: two overlapping leaders never both add one.
        #[tokio::test]
        async fn a_rotation_is_written_with_the_resource_version_read() {
            let at = now();
            let old = at - 31 * 86_400;
            let keys = IssuerKeySet {
                keys: vec![new_key(old, old).unwrap()],
            };
            let kube = MockServer::start().await;
            serve_secret(&kube, secret_json(UID, &keys, json!({}))).await;
            Mock::given(method("PUT"))
                .and(path(format!("{SECRETS}/proxyauth-issuer-{UID}")))
                .respond_with(echo(200))
                .expect(1)
                .mount(&kube)
                .await;
            let rotated = ensure_keys(&issuing(ISSUER_URL), &state(&kube))
                .await
                .unwrap();
            assert_eq!(rotated.keys.len(), 2);
            let replaced = written(&kube.received_requests().await.unwrap(), "PUT");
            assert_eq!(replaced[0]["metadata"]["resourceVersion"], "42");
        }

        /// T18: a Secret of the right name but another cluster's labels is
        /// neither used nor overwritten, and the cluster is not Ready.
        #[tokio::test]
        async fn a_key_secret_of_another_cluster_is_refused() {
            let at = now();
            let keys = IssuerKeySet {
                keys: vec![new_key(at, at).unwrap()],
            };
            let kube = MockServer::start().await;
            let mut foreign = secret_json("u-other", &keys, json!({}));
            foreign["metadata"]["name"] = json!(format!("proxyauth-issuer-{UID}"));
            serve_secret(&kube, foreign).await;
            Mock::given(method("PUT"))
                .respond_with(echo(200))
                .expect(0)
                .mount(&kube)
                .await;
            let mut evaluation = ready();
            evaluate(&issuing(ISSUER_URL), &state(&kube), &mut evaluation).await;
            let failure = evaluation.ready.expect_err("not Ready");
            assert_eq!(failure.reason, ReasonCode::IssuerKeyUnavailable);
        }

        /// An `issuer_url` that is not this cluster's route is refused,
        /// whatever the keys.
        #[tokio::test]
        async fn an_issuer_url_of_another_route_is_refused() {
            let at = now();
            let keys = IssuerKeySet {
                keys: vec![new_key(at, at).unwrap()],
            };
            let kube = MockServer::start().await;
            serve_secret(&kube, secret_json(UID, &keys, json!({}))).await;
            let mut evaluation = ready();
            let proxy = issuing("https://proxy.example.com/issuer/default/another");
            evaluate(&proxy, &state(&kube), &mut evaluation).await;
            let failure = evaluation.ready.expect_err("not Ready");
            assert_eq!(failure.reason, ReasonCode::IssuerUrlMismatch);
        }

        async fn serve_proxies(kube: &MockServer, response: ResponseTemplate) {
            Mock::given(method("GET"))
                .and(path("/apis/weebo.si.rs/v1/proxykubeapis"))
                .respond_with(response)
                .mount(kube)
                .await;
        }

        /// The sweep leaves a live cluster's key, marks a new orphan, and
        /// deletes one marked a day ago with the preconditions read.
        #[tokio::test]
        async fn the_sweep_marks_then_deletes_only_the_orphans() {
            let keys = IssuerKeySet::default();
            let marked = rfc3339(now() - ORPHAN_GRACE_SECONDS - 60).unwrap();
            let kube = MockServer::start().await;
            serve_proxies(
                &kube,
                ResponseTemplate::new(200).set_body_json(json!({
                    "apiVersion": "weebo.si.rs/v1", "kind": "ProxyKubeApiList",
                    "metadata": {},
                    "items": [serde_json::to_value(issuing(ISSUER_URL)).unwrap()],
                })),
            )
            .await;
            Mock::given(method("GET"))
                .and(path(SECRETS))
                .and(query_param(
                    "labelSelector",
                    format!("{MANAGED_BY_LABEL}={MANAGED_BY_VALUE},{CLUSTER_UID_LABEL}"),
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "apiVersion": "v1", "kind": "SecretList", "metadata": {},
                    "items": [
                        secret_json(UID, &keys, json!({})),
                        secret_json("u-new-orphan", &keys, json!({})),
                        secret_json("u-old-orphan", &keys, json!({ ORPHANED_ANNOTATION: marked })),
                    ],
                })))
                .mount(&kube)
                .await;
            Mock::given(method("PUT"))
                .and(path(format!("{SECRETS}/proxyauth-issuer-u-new-orphan")))
                .respond_with(echo(200))
                .expect(1)
                .mount(&kube)
                .await;
            Mock::given(method("DELETE"))
                .and(path(format!("{SECRETS}/proxyauth-issuer-u-old-orphan")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "kind": "Status", "apiVersion": "v1", "metadata": {}, "status": "Success",
                })))
                .expect(1)
                .mount(&kube)
                .await;

            sweep(&state(&kube)).await.unwrap();
            let requests = kube.received_requests().await.unwrap();
            let mark = written(&requests, "PUT");
            assert!(mark[0]["metadata"]["annotations"][ORPHANED_ANNOTATION].is_string());
            let delete = written(&requests, "DELETE");
            assert_eq!(
                delete[0]["preconditions"],
                json!({ "uid": "secret-of-u-old-orphan", "resourceVersion": "42" })
            );
        }

        /// A failed list of the clusters sweeps nothing: an empty answer
        /// must never read as "every cluster is gone".
        #[tokio::test]
        async fn a_failed_cluster_list_sweeps_nothing() {
            let kube = MockServer::start().await;
            serve_proxies(&kube, ResponseTemplate::new(500)).await;
            Mock::given(method("PUT"))
                .respond_with(echo(200))
                .expect(0)
                .mount(&kube)
                .await;
            Mock::given(method("DELETE"))
                .respond_with(echo(200))
                .expect(0)
                .mount(&kube)
                .await;
            assert!(sweep(&state(&kube)).await.is_err());
        }
    }
}
