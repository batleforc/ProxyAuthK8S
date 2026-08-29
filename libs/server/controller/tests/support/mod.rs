//! Shared harness for the controller's integration tier.
//!
//! The target cluster is a `wiremock` server (the reachability probe is a plain
//! GET, so a mock answers it exactly like a real apiserver would) and Redis is a
//! real server, because Redis is what the reconcile loop actually writes to and
//! what the request path reads back.
//!
//! Redis location comes from `TEST_REDIS_URL` (default `redis://127.0.0.1:6379`).
//! When it is unreachable the Redis-backed tests print a warning and return —
//! unless `REQUIRE_TEST_REDIS` is set, which turns that into a failure, exactly
//! like the `api` crate's harness.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use common::{State, oidc_conf::OidcConf};
use crd::{ProxyKubeApi, ProxyKubeApiSpec, certificate::CertSource, service::Service};
use deadpool_redis::{
    Config, Pool, Runtime,
    redis::{AsyncTypedCommands, RedisResult},
};

/// A port nothing listens on, for the degraded-Redis path.
pub const UNREACHABLE_REDIS_URL: &str = "redis://127.0.0.1:1";

static CLUSTER_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn redis_url() -> String {
    std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string())
}

/// Connect to the test Redis, or explain why the caller should give up.
///
/// Returns `None` when Redis is unreachable and `REQUIRE_TEST_REDIS` is unset.
pub async fn try_redis_pool() -> Option<Pool> {
    let url = redis_url();
    let pool = Config::from_url(&url)
        .create_pool(Some(Runtime::Tokio1))
        .expect("redis pool should build");
    let reachable: RedisResult<()> = match pool.get().await {
        Ok(mut conn) => conn.ping().await.map(|_: String| ()),
        Err(err) => {
            report_unavailable_redis(&url, &err.to_string());
            return None;
        }
    };

    match reachable {
        Ok(()) => Some(pool),
        Err(err) => {
            report_unavailable_redis(&url, &err.to_string());
            None
        }
    }
}

fn report_unavailable_redis(url: &str, error: &str) {
    let message =
        format!("test Redis at {url} is unreachable ({error}); set TEST_REDIS_URL to override");
    assert!(
        std::env::var("REQUIRE_TEST_REDIS").is_err(),
        "REQUIRE_TEST_REDIS is set but {message}"
    );
    eprintln!("SKIPPED: {message}");
}

/// Building a kube client (and reqwest's rustls stack) reaches for the crypto
/// provider the server installs in `main`; without it they panic.
pub fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// A `State` on the real test Redis whose Kubernetes client points at a port
/// nothing listens on.
///
/// Every fixture here pre-sets the status the reconcile will compute, so the
/// "patch only if it changed" branch is not taken and the client is never used
/// — a regression that starts patching unconditionally surfaces as a connection
/// error rather than as an extra silent round-trip. The successful patch is
/// covered by the envtest tier, against a real apiserver.
pub fn state_with_redis(redis_url: &str) -> State {
    install_crypto_provider();

    let kube_config = kube::Config::new("http://127.0.0.1:1".parse().expect("static uri parses"));
    State::from_parts(
        kube::Client::try_from(kube_config).expect("kube client builds"),
        common::redis_pool::RedisPool::from_url(redis_url).expect("redis pool builds"),
        OidcConf {
            client_id: "proxyauthk8s".to_string(),
            client_secret: None,
            issuer_url: "https://oidc.example.com".to_string(),
            scopes: "openid".to_string(),
            audience: "proxyauthk8s".to_string(),
            accept_authorized_party: false,
            redirect_url: None,
        },
        "https://proxy.example.com".to_string(),
        "https://front.example.com".to_string(),
    )
}

/// A cluster name unique to this test run, so tests never share Redis keys.
pub fn unique_cluster() -> String {
    let index = CLUSTER_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("reconcile-{}-{}", std::process::id(), index)
}

/// A proxy pointing at `upstream_url`, with no auth or security configuration.
pub fn proxy_fixture(ns: &str, cluster: &str, upstream_url: &str) -> ProxyKubeApi {
    let mut proxy = ProxyKubeApi::new(
        cluster,
        ProxyKubeApiSpec {
            enabled: true,
            cert: CertSource::Insecure(true),
            client_cert: None,
            service: Service::ExternalService {
                url: upstream_url.to_string(),
            },
            auth_config: None,
            security_config: None,
            expose_via_dashboard: false,
            dashboard_group: None,
            proxy_group: None,
            virtual_apis: Vec::new(),
        },
    );
    proxy.metadata.namespace = Some(ns.to_string());
    proxy
}
