//! The reconcile loop against a real Redis and a wiremock target cluster.
//!
//! This is the fast tier: it covers everything the reconcile body does that is
//! not an apiserver call — the computed status, the Redis cache and its index,
//! the success/backoff requeue, and the cleanup path. The status patch itself
//! needs a real apiserver and lives in the envtest tier
//! (`libs/server/api/tests/envtest_reconcile.rs`).

mod support;

use std::sync::Arc;
use std::time::Duration;

use common::State;
use controller::error::ControllerError;
use controller::proxy_kube_api::{
    REDIS_PREFIX, cleanup::clean_proxy_kube_api, reconcile::reconcile_proxy_kube_api,
};
use crd::{ProxyKubeApi, status::ProxyKubeApiStatus};
use deadpool_redis::{Pool, redis::AsyncTypedCommands};
use kube::runtime::controller::Action;
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

/// The status a reachable proxy reconciles to.
fn healthy_status(proxy: &ProxyKubeApi) -> ProxyKubeApiStatus {
    ProxyKubeApiStatus::new(true, Some(format!("/clusters/{}", proxy.to_path())), None)
}

/// The status a proxy whose target answered but is not usable reconciles to.
fn unreachable_status() -> ProxyKubeApiStatus {
    ProxyKubeApiStatus::new(
        false,
        None,
        Some("Target service is not reachable".to_string()),
    )
}

/// Reconcile `proxy` with its status pre-set to `status`, so the "patch only if
/// it changed" branch is not taken and no apiserver is needed.
async fn reconcile_without_patch(
    proxy: &ProxyKubeApi,
    status: ProxyKubeApiStatus,
    state: &State,
) -> Result<Action, ControllerError> {
    let mut proxy = proxy.clone();
    proxy.status = Some(status);
    reconcile_proxy_kube_api(&proxy, Arc::new(state.clone())).await
}

async fn get(pool: &Pool, key: &str) -> Option<String> {
    pool.get()
        .await
        .expect("redis connection")
        .get(key)
        .await
        .expect("GET should succeed")
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
    let _ = conn.del(&id).await;
    let _ = conn.del(format!("requeue_retry:{id}")).await;
    let _ = conn.srem(format!("{REDIS_PREFIX}:index"), &id).await;
}

#[tokio::test]
async fn a_reachable_cluster_is_cached_indexed_and_requeued_after_an_hour() {
    let pool = redis_or_skip!();
    let server = MockServer::start().await;
    mount_probe(&server, 200).await;

    let proxy = support::proxy_fixture("default", &support::unique_cluster(), &server.uri());
    let state = support::state_with_redis(&support::redis_url());
    cleanup_keys(&pool, &proxy).await;

    let action = reconcile_without_patch(&proxy, healthy_status(&proxy), &state)
        .await
        .expect("a reachable cluster reconciles");

    assert_eq!(action, Action::requeue(Duration::from_secs(60 * 60)));

    // The cache is what the request path reads, so the object must be there,
    // carrying the freshly computed status rather than the one it came in with.
    let id = proxy.to_identifier();
    let cached: serde_json::Value = serde_json::from_str(
        &get(&pool, &id)
            .await
            .expect("the proxy should have been cached"),
    )
    .expect("the cached value should be JSON");
    assert_eq!(cached["status"]["exposed"], true);
    assert_eq!(
        cached["status"]["path"],
        format!("/clusters/{}", proxy.to_path())
    );
    assert!(cached["status"]["error"].is_null());

    // The dashboard lists clusters from the index, not a KEYS scan.
    assert!(indexed(&pool, &id).await, "the proxy should be indexed");

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

    let action = reconcile_without_patch(&proxy, unreachable_status(), &state)
        .await
        .expect("an unreachable cluster still reconciles");

    // First consecutive failure: the base backoff delay.
    assert_eq!(action, Action::requeue(Duration::from_secs(5 * 60)));

    let id = proxy.to_identifier();
    let cached: serde_json::Value =
        serde_json::from_str(&get(&pool, &id).await.expect("the proxy should be cached"))
            .expect("the cached value should be JSON");
    assert_eq!(cached["status"]["exposed"], false);
    assert_eq!(cached["status"]["error"], "Target service is not reachable");
    assert!(indexed(&pool, &id).await);

    cleanup_keys(&pool, &proxy).await;
}

#[tokio::test]
async fn consecutive_failures_back_off_and_a_recovery_resets_the_counter() {
    let pool = redis_or_skip!();
    let failing = MockServer::start().await;
    mount_probe(&failing, 500).await;

    let proxy = support::proxy_fixture("default", &support::unique_cluster(), &failing.uri());
    let state = support::state_with_redis(&support::redis_url());
    cleanup_keys(&pool, &proxy).await;

    // Three consecutive failures double the delay each time.
    for expected_seconds in [5 * 60, 10 * 60, 20 * 60] {
        let action = reconcile_without_patch(&proxy, unreachable_status(), &state)
            .await
            .expect("an unreachable cluster still reconciles");
        assert_eq!(
            action,
            Action::requeue(Duration::from_secs(expected_seconds))
        );
    }

    let retry_key = format!("requeue_retry:{}", proxy.to_identifier());
    assert_eq!(
        get(&pool, &retry_key).await.as_deref(),
        Some("3"),
        "the retry counter should have counted every failure"
    );

    // The same cluster comes back: the counter is dropped so the next outage
    // starts from the base delay again instead of inheriting the old backoff.
    let healthy = MockServer::start().await;
    mount_probe(&healthy, 200).await;
    let recovered = support::proxy_fixture(
        "default",
        proxy
            .metadata
            .name
            .as_deref()
            .expect("the fixture is named"),
        &healthy.uri(),
    );

    let action = reconcile_without_patch(&recovered, healthy_status(&recovered), &state)
        .await
        .expect("a recovered cluster reconciles");

    assert_eq!(action, Action::requeue(Duration::from_secs(60 * 60)));
    assert!(
        get(&pool, &retry_key).await.is_none(),
        "the retry counter should have been cleared on recovery"
    );

    cleanup_keys(&pool, &proxy).await;
}

#[tokio::test]
async fn an_invalid_configuration_is_reported_without_probing_the_cluster() {
    use crd::authentication_configuration::{
        AuthenticationConfiguration, OidcProvider, ValidateAgainst,
    };

    let pool = redis_or_skip!();
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
        },
        disable_validation: false,
        validate_against: ValidateAgainst::OidcProvider,
    });
    let state = support::state_with_redis(&support::redis_url());
    cleanup_keys(&pool, &proxy).await;

    let expected_status = ProxyKubeApiStatus::new(
        false,
        None,
        Some(
            "Failed to validate proxy configuration: validate_against is set to OidcProvider \
             but the OIDC provider is not enabled"
                .to_string(),
        ),
    );
    let action = reconcile_without_patch(&proxy, expected_status, &state)
        .await
        .expect("an invalid proxy still reconciles");

    assert_eq!(action, Action::requeue(Duration::from_secs(5 * 60)));
    assert!(
        server
            .received_requests()
            .await
            .expect("the mock records requests")
            .is_empty(),
        "an invalid configuration should not be probed"
    );

    let cached: serde_json::Value = serde_json::from_str(
        &get(&pool, &proxy.to_identifier())
            .await
            .expect("the proxy should be cached"),
    )
    .expect("the cached value should be JSON");
    assert_eq!(cached["status"]["exposed"], false);
    assert!(
        cached["status"]["error"]
            .as_str()
            .expect("an error should be recorded")
            .starts_with("Failed to validate proxy configuration:")
    );

    cleanup_keys(&pool, &proxy).await;
}

#[tokio::test]
async fn a_failed_cache_write_is_surfaced_instead_of_reported_as_success() {
    // Redis is the source of truth the dashboard and the proxy read from.
    // Swallowing this would report success and drop the cluster for a full
    // success-requeue interval — an hour of invisible downtime.
    let server = MockServer::start().await;
    mount_probe(&server, 200).await;

    let proxy = support::proxy_fixture("default", &support::unique_cluster(), &server.uri());
    let state = support::state_with_redis(support::UNREACHABLE_REDIS_URL);

    let error = reconcile_without_patch(&proxy, healthy_status(&proxy), &state)
        .await
        .expect_err("an unwritable cache should fail the reconcile");

    assert!(
        matches!(error, ControllerError::Redis(_)),
        "expected a Redis error, got {error:?}"
    );
}

#[tokio::test]
async fn a_changed_status_is_patched_to_the_apiserver() {
    // The complement of the fixtures above, which all pre-set the status the
    // reconcile computes so the patch is skipped: when it *does* differ, the
    // apiserver is called. Here that call fails (nothing listens), which is
    // what proves it was attempted; the envtest tier covers it succeeding.
    let pool = redis_or_skip!();
    let server = MockServer::start().await;
    mount_probe(&server, 200).await;

    let proxy = support::proxy_fixture("default", &support::unique_cluster(), &server.uri());
    let state = support::state_with_redis(&support::redis_url());
    cleanup_keys(&pool, &proxy).await;

    // Status left as `None`, so it cannot equal the computed one.
    let error = reconcile_proxy_kube_api(&proxy, Arc::new(state))
        .await
        .expect_err("the patch should have been attempted and failed");

    assert!(
        matches!(error, ControllerError::Kube(_)),
        "expected a Kubernetes error, got {error:?}"
    );
    // The cache write happens before the patch, so it still landed.
    assert!(get(&pool, &proxy.to_identifier()).await.is_some());

    cleanup_keys(&pool, &proxy).await;
}

#[tokio::test]
async fn cleanup_drops_the_cached_cluster_and_its_index_entry() {
    let pool = redis_or_skip!();
    let server = MockServer::start().await;
    mount_probe(&server, 200).await;

    let proxy = support::proxy_fixture("default", &support::unique_cluster(), &server.uri());
    let state = support::state_with_redis(&support::redis_url());
    cleanup_keys(&pool, &proxy).await;

    reconcile_without_patch(&proxy, healthy_status(&proxy), &state)
        .await
        .expect("a reachable cluster reconciles");
    let id = proxy.to_identifier();
    assert!(get(&pool, &id).await.is_some());
    assert!(indexed(&pool, &id).await);

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
async fn cleanup_survives_an_unreachable_redis() {
    // Deletion must not wedge the finalizer: a cache that cannot be reached is
    // logged and the object is allowed to go away.
    let proxy = support::proxy_fixture("default", &support::unique_cluster(), "http://unused");
    let state = support::state_with_redis(support::UNREACHABLE_REDIS_URL);

    let action = clean_proxy_kube_api(&proxy, Arc::new(state))
        .await
        .expect("cleanup should not block deletion on Redis");
    assert_eq!(action, Action::await_change());
}
