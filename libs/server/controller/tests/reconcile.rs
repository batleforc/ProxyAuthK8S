//! The reconcile loop against a real Redis and a wiremock target cluster.
//!
//! This is the fast tier: it covers everything the reconcile does that is not
//! an apiserver call — the evaluation (`Ready` outcome, advisories, status
//! fields), the Redis cache and its index, and the cleanup path. The status
//! publication needs a real apiserver and lives in the envtest tier
//! (`libs/server/api/tests/envtest_reconcile.rs`).
#![expect(
    clippy::expect_used,
    reason = "test harness: a panic is a failed test, with its message"
)]

mod support;

use std::sync::Arc;

use controller::error::ControllerError;
use controller::proxy_kube_api::{
    REDIS_PREFIX,
    cleanup::clean_proxy_kube_api,
    reconcile::{cache, evaluate, reconcile_proxy_kube_api},
};
use controller::reason::ReasonCode;
use crd::{ProxyKubeApi, status::ProxyKubeApiStatus};
use deadpool_redis::{Pool, redis::AsyncTypedCommands};
use kube::runtime::controller::Action;
use weebo_kit_api::Condition;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Skip the test when the test Redis is unavailable (unless CI demands it).
macro_rules! redis_or_skip {
    () => {
        match support::try_redis_pool().await {
            Some(pool) => pool,
            None => return,
        }
    };
}

/// Answer the reachability probe — a plain GET on the service URL — with
/// `status_code`.
async fn mount_probe(server: &MockServer, status_code: u16) {
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(status_code))
        .mount(server)
        .await;
}

async fn get(pool: &Pool, key: &str) -> Option<String> {
    pool.get()
        .await
        .expect("redis connection")
        .get(key)
        .await
        .expect("GET should succeed")
}

async fn cached(pool: &Pool, proxy: &ProxyKubeApi) -> serde_json::Value {
    serde_json::from_str(
        &get(pool, &proxy.to_identifier())
            .await
            .expect("the proxy should have been cached"),
    )
    .expect("the cached value should be JSON")
}

async fn indexed(pool: &Pool, id: &str) -> bool {
    pool.get()
        .await
        .expect("redis connection")
        .sismember(format!("{REDIS_PREFIX}:index"), id)
        .await
        .expect("SISMEMBER should succeed")
}

async fn cleanup_keys(pool: &Pool, proxy: &ProxyKubeApi) {
    let id = proxy.to_identifier();
    let mut conn = pool.get().await.expect("redis connection");
    // Best effort: a leftover key only affects a later run of this same test.
    drop(conn.del(&id).await);
    drop(conn.srem(format!("{REDIS_PREFIX}:index"), &id).await);
}

#[tokio::test]
async fn a_reachable_cluster_is_ready_and_cached_without_its_conditions() {
    let pool = redis_or_skip!();
    let server = MockServer::start().await;
    mount_probe(&server, 200).await;

    let mut proxy = support::proxy_fixture("default", &support::unique_cluster(), &server.uri());
    // What the apiserver holds: the conditions of a previous reconcile.
    proxy.status = Some(ProxyKubeApiStatus {
        conditions: vec![Condition::default()],
        observed_generation: Some(1),
        ..ProxyKubeApiStatus::default()
    });
    let state = support::state_with_redis(&support::redis_url());
    cleanup_keys(&pool, &proxy).await;

    let evaluation = evaluate(&proxy, Arc::new(state.clone())).await;
    assert!(evaluation.ready.is_ok(), "{evaluation:?}");
    assert!(evaluation.steady());
    assert!(evaluation.advisories.is_empty());
    cache(&proxy, &evaluation.status, &state)
        .await
        .expect("the cache write succeeds");

    // The cache is what the request path reads: the freshly evaluated
    // fields, and none of the apiserver's conditions (decision 18).
    let cached = cached(&pool, &proxy).await;
    assert_eq!(cached["status"]["exposed"], true);
    assert_eq!(
        cached["status"]["path"],
        format!("/clusters/{}", proxy.to_path())
    );
    assert!(cached["status"]["error"].is_null());
    assert!(cached["status"].get("conditions").is_none(), "{cached}");
    assert!(
        cached["status"].get("observedGeneration").is_none(),
        "{cached}"
    );

    // The dashboard lists clusters from the index, not a KEYS scan.
    assert!(indexed(&pool, &proxy.to_identifier()).await);

    cleanup_keys(&pool, &proxy).await;
}

#[tokio::test]
async fn a_probe_that_fails_is_still_cached_so_the_cluster_stays_visible() {
    // A cluster that is down must not disappear from the dashboard — it is
    // cached with an error status, which is what makes the failure visible.
    let pool = redis_or_skip!();
    let server = MockServer::start().await;
    mount_probe(&server, 500).await;

    let proxy = support::proxy_fixture("default", &support::unique_cluster(), &server.uri());
    let state = support::state_with_redis(&support::redis_url());
    cleanup_keys(&pool, &proxy).await;

    let evaluation = evaluate(&proxy, Arc::new(state.clone())).await;
    let failure = evaluation.ready.as_ref().expect_err("the target is down");
    assert_eq!(failure.reason, ReasonCode::TargetUnreachable);
    assert!(!evaluation.steady(), "a down target backs off");
    cache(&proxy, &evaluation.status, &state)
        .await
        .expect("the cache write succeeds");

    let cached = cached(&pool, &proxy).await;
    assert_eq!(cached["status"]["exposed"], false);
    assert_eq!(cached["status"]["error"], "Target service is not reachable");
    assert!(indexed(&pool, &proxy.to_identifier()).await);

    cleanup_keys(&pool, &proxy).await;
}

#[tokio::test]
async fn an_invalid_configuration_is_reported_without_probing_the_cluster() {
    use crd::authentication_configuration::{
        AuthenticationConfiguration, OidcProvider, ValidateAgainst,
    };

    // No mock mounted: validation must fail before any probe goes out.
    let server = MockServer::start().await;

    let mut proxy = support::proxy_fixture("default", &support::unique_cluster(), &server.uri());
    // `validate_against: OidcProvider` with the provider disabled is exactly
    // what the CRD's CEL rule refuses; `validate()` has to agree.
    proxy.spec.auth_config = Some(AuthenticationConfiguration {
        jwt: Vec::new(),
        oidc_provider: OidcProvider {
            enabled: false,
            issuer_url: String::new(),
            client_id: "proxyauthk8s".to_string(),
            client_secret: None,
            extra_scope: String::new(),
            audience: String::new(),
            accept_authorized_party: false,
            expose_oauth_authorization_server: false,
            config_from: None,
        },
        disable_validation: false,
        validate_against: ValidateAgainst::OidcProvider,
    });
    let state = support::state_with_redis(&support::redis_url());

    let evaluation = evaluate(&proxy, Arc::new(state)).await;
    let failure = evaluation.ready.as_ref().expect_err("the spec is invalid");
    assert_eq!(failure.reason, ReasonCode::InvalidSpec);
    assert_eq!(
        failure.message,
        "Failed to validate proxy configuration: validate_against is set to OidcProvider \
         but the OIDC provider is not enabled"
    );
    assert_eq!(
        evaluation.status.error.as_deref(),
        Some(failure.message.as_str())
    );
    // Only a change of the CR can fix it, and a change reconciles by itself.
    assert!(evaluation.steady());
    assert!(
        server
            .received_requests()
            .await
            .expect("the mock records requests")
            .is_empty(),
        "an invalid configuration should not be probed"
    );
}

#[tokio::test]
async fn a_failed_cache_write_is_surfaced_instead_of_reported_as_success() {
    // Redis is the source of truth the dashboard and the proxy read from.
    // Swallowing this would report success and drop the cluster until the
    // next reconcile.
    let proxy = support::proxy_fixture("default", &support::unique_cluster(), "http://unused");
    let state = support::state_with_redis(support::UNREACHABLE_REDIS_URL);

    let error = cache(&proxy, &ProxyKubeApiStatus::new(true, None, None), &state)
        .await
        .expect_err("an unwritable cache should fail the reconcile");
    assert!(
        matches!(error, ControllerError::Redis(_)),
        "expected a Redis error, got {error:?}"
    );
    assert_eq!(
        error.reason(),
        Some((ReasonCode::CacheWriteFailed, "Reconcile"))
    );
}

#[tokio::test]
async fn the_status_is_published_on_every_reconcile() {
    // No equality guard (decision 17): even a CR whose status already says
    // what this reconcile finds is published. Here the publication fails
    // (nothing listens), which is what proves it was attempted; the envtest
    // tier proves the apiserver writes nothing for an identical patch.
    let pool = redis_or_skip!();
    let server = MockServer::start().await;
    mount_probe(&server, 200).await;

    let mut proxy = support::proxy_fixture("default", &support::unique_cluster(), &server.uri());
    proxy.status = Some(ProxyKubeApiStatus::new(
        true,
        Some(format!("/clusters/{}", proxy.to_path())),
        None,
    ));
    let state = support::state_with_redis(&support::redis_url());
    cleanup_keys(&pool, &proxy).await;

    let error = reconcile_proxy_kube_api(&proxy, Arc::new(state))
        .await
        .expect_err("the publication should have been attempted and failed");
    assert!(
        matches!(error, ControllerError::Kube(_)),
        "expected a Kubernetes error, got {error:?}"
    );
    assert_eq!(
        error.reason(),
        Some((ReasonCode::StatusWriteFailed, "Reconcile"))
    );
    // The cache write happens before the publication, so it still landed.
    assert!(get(&pool, &proxy.to_identifier()).await.is_some());

    cleanup_keys(&pool, &proxy).await;
}

#[tokio::test]
async fn cleanup_drops_the_cached_cluster_and_its_index_entry() {
    let pool = redis_or_skip!();
    let proxy = support::proxy_fixture("default", &support::unique_cluster(), "http://unused");
    let state = support::state_with_redis(&support::redis_url());
    cleanup_keys(&pool, &proxy).await;

    cache(&proxy, &ProxyKubeApiStatus::new(true, None, None), &state)
        .await
        .expect("the cache write succeeds");
    let id = proxy.to_identifier();
    assert!(get(&pool, &id).await.is_some());
    assert!(indexed(&pool, &id).await);

    // The `Deleted` Event aimed at the closed apiserver is logged, never
    // returned.
    let action = clean_proxy_kube_api(&proxy, Arc::new(state))
        .await
        .expect("cleanup succeeds");

    // Nothing to requeue: the object is going away.
    assert_eq!(action, Action::await_change());
    assert!(get(&pool, &id).await.is_none(), "the cache should be gone");
    assert!(
        !indexed(&pool, &id).await,
        "a deleted cluster must leave the index, or the dashboard keeps listing it"
    );
}

#[tokio::test]
async fn cleanup_of_an_unknown_cluster_is_not_an_error() {
    // The finalizer can run twice (a retried delete, a controller restart);
    // the second pass has nothing left to remove and must still succeed.
    let _pool = redis_or_skip!();
    let proxy = support::proxy_fixture("default", &support::unique_cluster(), "http://unused");
    let state = support::state_with_redis(&support::redis_url());

    let action = clean_proxy_kube_api(&proxy, Arc::new(state))
        .await
        .expect("cleaning an already-clean proxy succeeds");
    assert_eq!(action, Action::await_change());
}

#[tokio::test]
async fn cleanup_fails_closed_when_redis_is_unreachable() {
    // The proxy routes from Redis and the key has no TTL: letting the object go
    // while its key stays would leave a deleted cluster routable (RFC 0011, T6).
    // The error keeps the finalizer, and the cleanup runs again.
    let proxy = support::proxy_fixture("default", &support::unique_cluster(), "http://unused");
    let state = support::state_with_redis(support::UNREACHABLE_REDIS_URL);

    let error = clean_proxy_kube_api(&proxy, Arc::new(state))
        .await
        .expect_err("cleanup must not release the finalizer while Redis is down");
    assert!(
        matches!(error, ControllerError::Redis(_)),
        "expected a Redis error, got {error:?}"
    );
}

/// RFC 0012: an `Agent` cluster is ready while a gateway pod holds one of its
/// tunnels, `AgentNoTunnel` otherwise, and `status.agent` sums the pods'
/// records; a `limits.streams_per_tunnel` past the gateway's maximum is an
/// advisory.
#[tokio::test]
async fn an_agent_cluster_is_ready_through_the_gateway_presences() {
    use common::agent_gateway::{
        AgentCount, AgentRefusal, PresenceInfo, agent_refused_key, presence_info_key, presence_key,
    };
    use crd::service::{AgentBootstrap, AgentLimits, AgentService, Service};

    let _pool = redis_or_skip!();
    let mut proxy = support::proxy_fixture("default", &support::unique_cluster(), "https://unused");
    let uid = format!("{:08x}-0000-4000-8000-0000000000aa", std::process::id());
    proxy.metadata.uid = Some(uid.clone());
    proxy.spec.cert = crd::certificate::CertSource::ConfigMap {
        name: "prod-ca".to_string(),
        key: "ca.crt".to_string(),
        namespace: None,
    };
    proxy.spec.service = Service::Agent(AgentService {
        server_name: crd::service::DEFAULT_SERVER_NAME.to_string(),
        epoch: 0,
        bootstrap: AgentBootstrap::ServiceAccountToken(
            crd::service::ServiceAccountTokenBootstrap {
                subject: "system:serviceaccount:a:b".to_string(),
                audience: crd::service::DEFAULT_AUDIENCE.to_string(),
                jwks: None,
                jwks_url: Some("https://issuer.example.com/keys".to_string()),
                jwks_from: None,
            },
        ),
        limits: Some(AgentLimits {
            streams_per_tunnel: Some(100_000),
        }),
    });
    let state = support::state_with_redis(&support::redis_url());
    let key = presence_key(&uid);
    for pod in ["gw-0", "gw-1"] {
        drop(state.presence_remove(&key, pod).await);
    }

    drop(state.delete_key(&agent_refused_key(&uid)).await);
    let evaluation = evaluate(&proxy, Arc::new(state.clone())).await;
    let failure = evaluation.ready.as_ref().expect_err("no tunnel yet");
    assert_eq!(failure.reason, ReasonCode::AgentNoTunnel);
    assert_eq!(
        evaluation.status.agent.as_ref().map(|agent| agent.tunnels),
        Some(0)
    );
    assert!(
        evaluation
            .advisories
            .iter()
            .any(|advisory| advisory.reason == ReasonCode::AgentLimitCapped),
        "{evaluation:?}"
    );

    // RFC 0013: a recent protocol refusal is the cause of the missing tunnel.
    let refusal = AgentRefusal {
        reason: "protocol".to_string(),
        version: "0.7.0".to_string(),
        protocols: "2-3".to_string(),
    };
    state
        .redis_set(
            &agent_refused_key(&uid),
            &serde_json::to_string(&refusal).expect("json"),
            Some(60),
        )
        .await
        .expect("refusal");
    let evaluation = evaluate(&proxy, Arc::new(state.clone())).await;
    let failure = evaluation.ready.as_ref().expect_err("no tunnel yet");
    assert_eq!(failure.reason, ReasonCode::AgentProtocolUnsupported);
    let agent = evaluation.status.agent.as_ref().expect("status.agent");
    assert_eq!(agent.refused.as_ref(), Some(&refusal));
    assert_eq!(
        agent.proxy_version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    drop(state.delete_key(&agent_refused_key(&uid)).await);

    // gw-1 is a 0.4.0 gateway: its record has no `agents`.
    let old = r#"{"tunnels":1,"versions":["0.4.1"],"ca_fingerprints":["ab"]}"#.to_string();
    let new = serde_json::to_string(&PresenceInfo {
        tunnels: 1,
        versions: vec!["0.4.0".to_string()],
        ca_fingerprints: vec!["ab".to_string()],
        agents: vec![AgentCount {
            version: "0.4.0".to_string(),
            protocol: 1,
            tunnels: 1,
        }],
    })
    .expect("json");
    for (pod, record) in [("gw-1", old), ("gw-0", new)] {
        state
            .redis_set(&presence_info_key(&uid, pod), &record, Some(30))
            .await
            .expect("record");
        state
            .presence_refresh(&key, pod, 30)
            .await
            .expect("presence");
    }
    let evaluation = evaluate(&proxy, Arc::new(state.clone())).await;
    assert!(evaluation.ready.is_ok(), "{evaluation:?}");
    let agent = evaluation.status.agent.expect("status.agent");
    assert_eq!(agent.tunnels, 2);
    assert_eq!(agent.gateways, vec!["gw-0", "gw-1"]);
    assert_eq!(agent.versions, vec!["0.4.0", "0.4.1"]);
    assert_eq!(agent.ca_fingerprints, vec!["ab"]);
    assert_eq!(
        agent.agents,
        vec![AgentCount {
            version: "0.4.0".to_string(),
            protocol: 1,
            tunnels: 1,
        }]
    );

    for pod in ["gw-0", "gw-1"] {
        drop(state.presence_remove(&key, pod).await);
    }
}
