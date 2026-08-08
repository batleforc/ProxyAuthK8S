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
            expose_oauth_authorization_server: false,
        },
        disable_validation: false,
        validate_against: ValidateAgainst::OidcProvider,
    }
}

/// An `AuthenticationConfiguration` that also exposes the well-known
/// OAuth authorization server discovery document.
pub fn oidc_auth_config_with_well_known(issuer_url: &str) -> AuthenticationConfiguration {
    let mut config = oidc_auth_config(issuer_url);
    config.oidc_provider.expose_oauth_authorization_server = true;
    config
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

/// A fixed RSA-2048 test keypair, generated once with `openssl genrsa` purely
/// for signing test ID tokens. Not used for anything but these tests.
const TEST_RSA_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCVWDLfDvI6z8rf
b25gM0mJDih3rqsYac61YHG7vHAgzQxpGsY14+tTiEn9/9jE3mXzmya7HcVnuazT
61Ca2YeFBTMIRY1zH5K5Mh/mJIMMWOzE57ZC8bQhx1XpJv5v7j6s5NUzwwxcSg+J
u98gphEwYp7m2TElLnoFufeDv98THz/gz3mbKY8m71FU88mnqiQ56FTuR6G0Gotd
0wiBuH+ec67hmtE1TtezajOiBBushf13+4PQSZtAQbwBJn4StggtAHAgmiJD67sR
jKUPSXd8heG3XyVi0tTQlTXqzPn3VH8WZzTQEExlApjbIRA6CX48kwAv3/DXRwLI
zmn+5pbvAgMBAAECggEAIQdWiNpnW/ZkqbGdOY1eL/9/l6h7knSkEJz5yklMixSO
MBiJyZVUkC7OHmyc5j1BUvT3Rd65r8zymhOqyfRd8l9KAARR2iobavXY9C8TBIIO
KyYLuxZ1fhr1txC2qM6J8fbR6Ba0/xwp/44bNL9FgevttKRIKC71MZsFUI/4p+Ok
4SX/jCLf9dMfnSUxeEcNgkp0XzjC0ybF/Fp3HOiHZLpYLetKTPZ+OT53w6/SF5T7
zR4C0syvwA1xcbVXECgaEuf6UigHQep3AuD5Cm45VCGq1hJoTnqdffCKiXKexMwE
lAEOPLJmFRQt1eWuJYM5udFuXzdXynqSSAY//aT/sQKBgQDFbMqbErHw61jQur2i
gyZ+/2gMrgNwJvezv0v6+hjDNlpycUbk8uAjRByE8LFgNsD8x/UO1wwQ6OhSKzkH
uhEv0XIHDOyKvytFoR5A4kXOeQi0z/yZYROeHQi/IHB4wIHNa5DeaE0rDpsZ9Jzg
P3Nv6ejOArXCkTz5gRZgxvzztQKBgQDBp4HfPLkb3w18mx2gYyLb2j3A7USd2/QE
QAZ6uvUcbtmroxlisxrkn2dhk1bwmW3Cy+ZiBmzduvrbfbUR7SEmTVHFafiGxogi
ZtvCQbpLZW1wTviZxJVIkOTx09fkGVkd1PyFANlxHEpEDDd1+MuxVETziKlVNHDj
nPvlHj/OkwKBgQCrH5+GRvAh6X00f4j8Ij3t+qhPxU2Jmt092mSbiMiJ/MTtSa6v
qK4LI3Cs8oxs30jsUs3hLRlyVs942ao3PlrDXgI+hj9KDGYPlpZIm1jynQqk31sN
/40nkfcQ46dZo1NfoQsTHMk2txRNrS+FWLpQmSmH1+WAXq/BfNjOzexXuQKBgFEp
bjHsljxLKLDfpfQReIuiFR2lk4uBouyhFNYdQxtujgX0bnBCVnQZJs/rW5WtCCaL
JHxS6w+nDPou3lOsCaeu4iWV+1YpIOciKtpoh7aPxOU8A88WZ+ao63s66RGtWf85
w7fOmlNgovOQFzJ3Wo9wnRFgZm/ScbnDkoL9QYrHAoGBAMCFrAVPhkeCbhEE3d72
2GtWI9pLBSDdjHcFsKPHSH4OLV+xnId2YtnqnWxxCbrEXwZrKwHQ//0JY68hwYk6
MMbXBdtd9gV1rhXUavlYXcfj0yKGz0sG9D6nXTc4t/40qaLR3138VpTHQaMJ69oy
ek6nnHwctKRm0DIafA6KY8tj
-----END PRIVATE KEY-----
";

const TEST_RSA_KID: &str = "test-key-1";

/// The JWKS matching [`TEST_RSA_PRIVATE_KEY_PEM`], as the upstream provider
/// would publish at its `jwks_uri`.
pub fn test_jwks() -> serde_json::Value {
    serde_json::json!({
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": TEST_RSA_KID,
            "n": "lVgy3w7yOs_K329uYDNJiQ4od66rGGnOtWBxu7xwIM0MaRrGNePrU4hJ_f_YxN5l85smux3FZ7ms0-tQmtmHhQUzCEWNcx-SuTIf5iSDDFjsxOe2QvG0IcdV6Sb-b-4-rOTVM8MMXEoPibvfIKYRMGKe5tkxJS56Bbn3g7_fEx8_4M95mymPJu9RVPPJp6okOehU7kehtBqLXdMIgbh_nnOu4ZrRNU7Xs2ozogQbrIX9d_uD0EmbQEG8ASZ-ErYILQBwIJoiQ-u7EYylD0l3fIXht18lYtLU0JU16sz591R_Fmc00BBMZQKY2yEQOgl-PJMAL9_w10cCyM5p_uaW7w",
            "e": "AQAB",
        }]
    })
}

/// Sign a minimal, valid OIDC ID token with [`TEST_RSA_PRIVATE_KEY_PEM`].
///
/// Deliberately omits `at_hash`: both `callback.rs` handlers only check it
/// when present, and computing a spec-correct hash here would just duplicate
/// that (unchanged, already-shipped) verification logic instead of testing
/// anything new.
pub fn sign_id_token(issuer: &str, audience: &str, subject: &str, nonce: &str) -> String {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after the epoch")
        .as_secs();
    let claims = serde_json::json!({
        "iss": issuer,
        "sub": subject,
        "aud": audience,
        "iat": now,
        "exp": now + 300,
        "nonce": nonce,
    });
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(TEST_RSA_KID.to_string());
    let key = EncodingKey::from_rsa_pem(TEST_RSA_PRIVATE_KEY_PEM.as_bytes())
        .expect("test RSA key should parse");
    encode(&header, &claims, &key).expect("test id_token should sign")
}

/// Mount a full, signature-capable OIDC provider: discovery, a real JWKS
/// (unlike [`mount_oidc_provider`]'s empty one), and `/userinfo`.
///
/// Does not mount `/token`: the response depends on a nonce/id_token only
/// known once the flow under test has started (see `sign_id_token`), so
/// callers mount it themselves, per-test, once they have that value.
pub async fn mount_full_oidc_provider(server: &wiremock::MockServer, username: &str, groups: &[&str]) {
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
        .respond_with(ResponseTemplate::new(200).set_body_json(test_jwks()))
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

/// Mount `/token`, returning `id_token` alongside a fixed access/refresh
/// token pair. Mount this only after `id_token` has been signed with the
/// nonce the flow under test actually generated.
pub async fn mount_token_endpoint(server: &wiremock::MockServer, id_token: &str) {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "upstream-access-token",
            "token_type": "Bearer",
            "refresh_token": "upstream-refresh-token",
            "id_token": id_token,
            "expires_in": 3600,
        })))
        .mount(server)
        .await;
}
