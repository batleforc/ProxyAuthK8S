//! Shared harness for the fast integration tier.
//!
//! Upstream Kubernetes API and OIDC provider are faked with `wiremock`; Redis
//! is a real server because `State` caches cluster configuration there and that
//! path is what we want to exercise.
//!
//! Redis location comes from `TEST_REDIS_URL` (default `redis://127.0.0.1:6379`).
//! When it is unreachable the Redis-backed tests print a warning and return —
//! unless `REQUIRE_TEST_REDIS` is set, which turns that into a failure. CI sets
//! it so a missing service container can never silently green the suite.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use common::{oidc_conf::OidcConf, State};
use crd::{
    authentication_configuration::{AuthenticationConfiguration, OidcProvider, ValidateAgainst},
    certificate::CertSource,
    security::SecurityConfiguration,
    service::Service,
    ProxyKubeApi, ProxyKubeApiSpec,
};
use deadpool_redis::{
    redis::{AsyncTypedCommands, RedisResult},
    Config, Pool, Runtime,
};

pub const REDIS_PREFIX: &str = crd::REDIS_PREFIX;

static CLUSTER_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Redis URL under test.
pub fn redis_url() -> String {
    std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string())
}

pub fn redis_pool(url: &str) -> Pool {
    Config::from_url(url)
        .create_pool(Some(Runtime::Tokio1))
        .expect("redis pool should build")
}

/// A pool pointed at a port nothing listens on, to exercise the degraded path.
pub fn unreachable_redis_pool() -> Pool {
    redis_pool(UNREACHABLE_REDIS_URL)
}

/// A `State` whose Redis is unreachable, to exercise the degraded path.
pub fn unreachable_state(oidc_issuer_url: String) -> State {
    install_crypto_provider();

    let kube_config = kube::Config::new(
        "http://127.0.0.1:1"
            .parse()
            .expect("static uri should parse"),
    );
    State::from_parts(
        kube::Client::try_from(kube_config).expect("kube client should build"),
        common::redis_pool::RedisPool::from_url(UNREACHABLE_REDIS_URL).expect("pool should build"),
        OidcConf {
            client_id: "proxyauthk8s".to_string(),
            client_secret: None,
            issuer_url: oidc_issuer_url,
            scopes: "openid".to_string(),
            audience: "proxyauthk8s".to_string(),
            accept_authorized_party: false,
            redirect_url: None,
        },
        "https://proxy.example.com".to_string(),
        "https://front.example.com".to_string(),
    )
}

const UNREACHABLE_REDIS_URL: &str = "redis://127.0.0.1:1";

/// Connect to the test Redis, or explain why the caller should give up.
///
/// Returns `None` when Redis is unreachable and `REQUIRE_TEST_REDIS` is unset.
pub async fn try_redis_pool() -> Option<Pool> {
    let url = redis_url();
    let pool = redis_pool(&url);
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

/// A cluster identifier unique to this test run, so tests never share Redis keys.
pub fn unique_cluster() -> (String, String) {
    let index = CLUSTER_COUNTER.fetch_add(1, Ordering::Relaxed);
    (
        "default".to_string(),
        format!("test-cluster-{}-{}", std::process::id(), index),
    )
}

/// The server installs the rustls provider in `main`; tests have to do it too
/// or `build_tls_config` panics on the first proxied request.
pub fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Build a `State` whose Kubernetes client is never contacted: every fixture
/// uses `Service::ExternalService` and `CertSource::Insecure`.
pub fn test_state(oidc_issuer_url: String) -> State {
    install_crypto_provider();

    // `State` owns its own pool abstraction; the raw deadpool pool the tests use
    // to seed and inspect keys stays separate.
    let redis = common::redis_pool::RedisPool::from_url(&redis_url())
        .expect("state redis pool should build");

    let kube_config = kube::Config::new(
        "http://127.0.0.1:1"
            .parse()
            .expect("static uri should parse"),
    );
    let client = kube::Client::try_from(kube_config).expect("kube client should build");

    State::from_parts(
        client,
        redis,
        OidcConf {
            client_id: "proxyauthk8s".to_string(),
            client_secret: Some("secret".to_string()),
            issuer_url: oidc_issuer_url,
            scopes: "openid email profile groups".to_string(),
            audience: "proxyauthk8s".to_string(),
            accept_authorized_party: false,
            redirect_url: None,
        },
        "https://proxy.example.com".to_string(),
        "https://front.example.com".to_string(),
    )
}

/// A proxy pointing at `upstream_url`, with token validation disabled.
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

/// An `AuthenticationConfiguration` that forces token validation against OIDC.
pub fn oidc_auth_config(issuer_url: &str) -> AuthenticationConfiguration {
    AuthenticationConfiguration {
        jwt: Vec::new(),
        oidc_provider: OidcProvider {
            enabled: true,
            issuer_url: issuer_url.to_string(),
            client_id: "proxyauthk8s".to_string(),
            client_secret: Some("secret".to_string()),
            extra_scope: "groups".to_string(),
            audience: String::new(),
            accept_authorized_party: false,
        },
        disable_validation: false,
        validate_against: ValidateAgainst::OidcProvider,
    }
}

pub fn security_config(paths: Vec<(&str, bool)>) -> SecurityConfiguration {
    use crd::security::{AllowedPathConfiguration, AllowedPathConfigurationEnum};

    SecurityConfiguration {
        enabled: true,
        allowed_resources: paths
            .into_iter()
            .map(|(path, parametised)| {
                AllowedPathConfigurationEnum::Path(AllowedPathConfiguration {
                    path: path.to_string(),
                    parametised,
                })
            })
            .collect(),
        ..SecurityConfiguration::default()
    }
}

/// A security configuration that rate limits every caller to `per_minute`.
pub fn rate_limited_config(per_minute: u32) -> SecurityConfiguration {
    use crd::security::RateLimitingConfiguration;

    SecurityConfiguration {
        rate_limiting: RateLimitingConfiguration {
            enabled: true,
            max_requests_per_minute: per_minute,
        },
        ..SecurityConfiguration::default()
    }
}

/// A security configuration that bans a caller after `max_failed_logins`.
pub fn fail2login_config(max_failed_logins: u32, ban_duration: u32) -> SecurityConfiguration {
    use crd::security::Fail2LoginEqualBanConfiguration;

    SecurityConfiguration {
        fail2login_equal_ban: Fail2LoginEqualBanConfiguration {
            enabled: true,
            max_failed_logins,
            ban_duration,
            exponential_backoff: false,
        },
        ..SecurityConfiguration::default()
    }
}

/// Enable a virtual API on a proxy fixture.
pub fn with_virtual_api(proxy: &mut ProxyKubeApi, kind: crd::virtual_api::VirtualApiKind) {
    proxy
        .spec
        .virtual_apis
        .push(crd::virtual_api::VirtualApiConfiguration::new(kind));
}

/// Mount a working OIDC provider on `server`, resolving `TEST_TOKEN` to `user`.
///
/// The same wiremock server can also stand in for the upstream Kubernetes API:
/// the OIDC paths (`/.well-known/...`, `/jwks`, `/userinfo`) never collide with
/// the `/api/...` and `/apis/...` prefixes a Kubernetes client uses.
pub async fn mount_oidc_provider(server: &wiremock::MockServer, username: &str, groups: &[&str]) {
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    let issuer = server.uri();

    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{issuer}/token"),
            "userinfo_endpoint": format!("{issuer}/userinfo"),
            "jwks_uri": format!("{issuer}/jwks"),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "keys": [] })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/userinfo"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({
                    "sub": format!("{username}-sub"),
                    "preferred_username": username,
                    "email": format!("{username}@example.com"),
                    "groups": groups,
                })),
        )
        .mount(server)
        .await;
}

/// Cache a proxy the way the controller does, so `redirect()` can find it.
pub async fn seed_proxy(pool: &Pool, proxy: &ProxyKubeApi) {
    use common::traits::ObjectRedis;

    let key = format!(
        "{}:{}/{}",
        REDIS_PREFIX,
        proxy.metadata.namespace.as_deref().unwrap_or_default(),
        proxy.metadata.name.as_deref().unwrap_or_default()
    );

    let mut conn = pool.get().await.expect("redis connection");
    conn.set_ex(&key, proxy.to_json(), 300)
        .await
        .expect("proxy should be cached");
    // The controller keeps this index in sync in production; the dashboard
    // listing reads through it instead of scanning with `KEYS`.
    conn.sadd(format!("{}:index", REDIS_PREFIX), &key)
        .await
        .expect("proxy should be indexed");
}

pub async fn delete_proxy(pool: &Pool, ns: &str, cluster: &str) {
    let key = format!("{}:{}/{}", REDIS_PREFIX, ns, cluster);
    let mut conn = pool.get().await.expect("redis connection");
    let _ = conn.del(&key).await;
    let _ = conn.srem(format!("{}:index", REDIS_PREFIX), &key).await;
}
