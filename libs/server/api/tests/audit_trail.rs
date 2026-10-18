//! RFC 0008: the audit lines of the proxy path, the logins, the mediated
//! OAuth server and the bans, read from the in-process capture
//! (`audit::capture`). Each test reads only the lines of its own cluster.

mod harness;

use api::cluster::auth::oauth::model::IssuedCode;
use api::cluster::redirect::audit::capture;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use deadpool_redis::redis::AsyncTypedCommands;
use harness::test;
use harness::{
    delete_proxy, fail2login_config, oidc_auth_config, oidc_auth_config_with_well_known,
    proxy_fixture, seed_proxy, test_state, try_redis_pool, unique_cluster,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use wiremock::{Mock, MockServer, ResponseTemplate};

const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

macro_rules! redis_or_skip {
    () => {
        match try_redis_pool().await {
            Some(pool) => pool,
            None => return,
        }
    };
}

/// The captured lines of `ns/cluster`, `event` only.
fn lines(ns: &str, cluster: &str, event: &str) -> Vec<Value> {
    capture::lines()
        .into_iter()
        .filter(|line| line["ns"] == ns && line["cluster"] == cluster && line["event"] == event)
        .collect()
}

async fn upstream_answering_200() -> MockServer {
    let upstream = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_string("upstream"))
        .mount(&upstream)
        .await;
    upstream
}

/// A proxied request: one `request` line with the HTTP method, the
/// Kubernetes verb and the path without its query.
#[tokio::test]
async fn a_proxied_request_writes_one_request_line() {
    capture::start();
    let pool = redis_or_skip!();
    let upstream = upstream_answering_200().await;
    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream.uri())).await;

    let app = test::init_service(harness::app(test_state(upstream.uri()))).await;
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods?labelSelector=secret"
        ))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 200);

    let requests = lines(&ns, &cluster, "request");
    assert_eq!(requests.len(), 1, "{requests:?}");
    let line = &requests[0];
    assert_eq!(line["audit_schema"], 1);
    assert_eq!(line["outcome"], "allowed");
    assert_eq!(line["http_method"], "GET");
    assert_eq!(line["verb"], "GET");
    assert_eq!(line["k8s_verb"], "list");
    assert_eq!(line["path"], "/api/v1/namespaces/dev/pods");
    assert_eq!(line["status"], 200);
    assert!(
        !line.to_string().contains("secret"),
        "the query leaked: {line}"
    );
    delete_proxy(&pool, &ns, &cluster).await;
}

/// A verb refused by the policy: a `policy_decision` line and a `request`
/// line in 403, both `denied`.
#[tokio::test]
async fn a_refused_verb_writes_a_policy_decision() {
    use crd::security::{AllowedPathConfiguration, AllowedPathConfigurationEnum, Verb};
    capture::start();
    let pool = redis_or_skip!();
    let upstream = upstream_answering_200().await;
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    let mut config = harness::security_config(vec![]);
    config
        .allowed_resources
        .push(AllowedPathConfigurationEnum::Path(
            AllowedPathConfiguration {
                path: "/api/v1/namespaces/dev/pods/**".to_string(),
                parametised: true,
                allowed_ports: None,
                exec: None,
                verbs: Some(Verb::READ.to_vec()),
            },
        ));
    proxy.spec.security_config = Some(config);
    seed_proxy(&pool, &proxy).await;

    let app = test::init_service(harness::app(test_state(upstream.uri()))).await;
    let req = test::TestRequest::delete()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods/web"
        ))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 403);

    let decisions = lines(&ns, &cluster, "policy_decision");
    assert_eq!(decisions.len(), 1, "{decisions:?}");
    assert_eq!(decisions[0]["policy"], "verb");
    assert_eq!(decisions[0]["decision"], "denied");
    assert_eq!(decisions[0]["outcome"], "denied");
    assert_eq!(decisions[0]["k8s_verb"], "delete");
    let requests = lines(&ns, &cluster, "request");
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0]["status"], 403);
    assert_eq!(requests[0]["outcome"], "denied");
    delete_proxy(&pool, &ns, &cluster).await;
}

/// Failures past the `fail2login` threshold: one `client_banned` line with
/// the failures and the ban length.
#[tokio::test]
async fn a_ban_writes_a_client_banned_line() {
    capture::start();
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    harness::mount_oidc_provider(&upstream, "alice", &["dev"]).await;
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    proxy.spec.security_config = Some(fail2login_config(2, 300));
    seed_proxy(&pool, &proxy).await;

    let app = test::init_service(harness::app(test_state(upstream.uri()))).await;
    for _ in 0..2 {
        let req = test::TestRequest::get()
            .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
            .to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 401);
    }

    let bans = lines(&ns, &cluster, "client_banned");
    assert_eq!(bans.len(), 1, "{bans:?}");
    assert_eq!(bans[0]["outcome"], "denied");
    assert_eq!(bans[0]["failures"], 2);
    assert_eq!(bans[0]["ban_seconds"], 300);
    assert!(bans[0]["client_ip"].is_string());

    let mut conn = pool.get().await.expect("redis connection");
    let _ = conn
        .del(format!("proxyk8sauth:ban:{ns}/{cluster}:unknown"))
        .await;
    let _ = conn
        .del(format!("proxyk8sauth:fail2login:{ns}/{cluster}:unknown"))
        .await;
    delete_proxy(&pool, &ns, &cluster).await;
}

/// A login without a bearer token: an `auth_login` line, `denied`, with its
/// reason.
#[tokio::test]
async fn a_login_without_a_token_is_denied() {
    capture::start();
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    harness::mount_oidc_provider(&upstream, "alice", &["dev"]).await;
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = test::init_service(harness::app(test_state(upstream.uri()))).await;
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/auth/login"))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 401);

    let logins = lines(&ns, &cluster, "auth_login");
    assert_eq!(logins.len(), 1, "{logins:?}");
    assert_eq!(logins[0]["outcome"], "denied");
    assert_eq!(logins[0]["reason"], "no usable bearer token");
    assert_eq!(logins[0]["status"], 401);
    delete_proxy(&pool, &ns, &cluster).await;
}

async fn seed_code(pool: &deadpool_redis::Pool, ns: &str, cluster: &str, code: &str) {
    let issued = IssuedCode {
        subject: "alice".to_string(),
        redirect_uri: "http://localhost:12345/callback".to_string(),
        code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes())),
        access_token: "upstream-access-token".to_string(),
        refresh_token: Some("upstream-refresh-token".to_string()),
        id_token: "upstream-id-token".to_string(),
        scope: "openid".to_string(),
        expires_in: Some(3600),
    };
    let mut conn = pool.get().await.expect("redis connection");
    conn.set_ex(
        format!("oauth_as_code:{ns}/{cluster}/{code}"),
        serde_json::to_string(&issued).expect("issued code should serialize"),
        60,
    )
    .await
    .expect("issued code should be cached");
}

async fn redeem(ns: &str, cluster: &str, code: &str) -> u16 {
    let app = test::init_service(harness::app(test_state(
        "https://issuer.example.com".to_string(),
    )))
    .await;
    let req = test::TestRequest::post()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/token"))
        .set_form([
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", "http://localhost:12345/callback?ignored=1"),
            ("code_verifier", VERIFIER),
        ])
        .to_request();
    test::call_service(&app, req).await.status().as_u16()
}

/// T2 and T3: tokens handed out leave an `oauth_token_issued` line naming who
/// received them, and no token, code or verifier in any line.
#[tokio::test]
async fn issued_tokens_are_audited_without_the_tokens() {
    capture::start();
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(
        "https://issuer.example.com",
    ));
    seed_proxy(&pool, &proxy).await;
    seed_code(&pool, &ns, &cluster, "the-code").await;

    // The `redirect_uri` with a query does not match the code's: refused.
    assert_eq!(redeem(&ns, &cluster, "the-code").await, 400);
    let denied = lines(&ns, &cluster, "oauth_token_denied");
    assert_eq!(denied.len(), 1, "{denied:?}");
    assert_eq!(denied[0]["reason"], "redirect_uri does not match the code");
    assert_eq!(denied[0]["user"], "alice");
    assert_eq!(denied[0]["redirect_uri"], "http://localhost:12345/callback");

    // A code never issued.
    assert_eq!(redeem(&ns, &cluster, "unknown-code").await, 400);
    assert_eq!(lines(&ns, &cluster, "oauth_token_denied").len(), 2);

    // A good redemption.
    seed_code(&pool, &ns, &cluster, "good-code").await;
    let app = test::init_service(harness::app(test_state(
        "https://issuer.example.com".to_string(),
    )))
    .await;
    let req = test::TestRequest::post()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/token"))
        .set_form([
            ("grant_type", "authorization_code"),
            ("code", "good-code"),
            ("redirect_uri", "http://localhost:12345/callback"),
            ("code_verifier", VERIFIER),
        ])
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 200);
    let issued = lines(&ns, &cluster, "oauth_token_issued");
    assert_eq!(issued.len(), 1, "{issued:?}");
    assert_eq!(issued[0]["outcome"], "allowed");
    assert_eq!(issued[0]["user"], "alice");
    assert_eq!(issued[0]["grant_type"], "authorization_code");
    assert_eq!(issued[0]["expires_in"], 3600);

    let everything: String = capture::lines()
        .iter()
        .filter(|line| line["ns"] == ns.as_str())
        .map(Value::to_string)
        .collect();
    for secret in [
        "upstream-access-token",
        "upstream-refresh-token",
        "upstream-id-token",
        "good-code",
        "the-code",
        VERIFIER,
    ] {
        assert!(
            !everything.contains(secret),
            "{secret} leaked into the audit"
        );
    }
    delete_proxy(&pool, &ns, &cluster).await;
}
