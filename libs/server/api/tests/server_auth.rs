//! RFC 0010 §6.5, in process: the CLI's SSO session to the proxy, against a
//! wiremock standing in for the proxy's own IdP. Login, callback and token
//! end to end; the refresh, and T12 (a refused refresh counts toward the
//! unscoped ban, an unavailable IdP does not).
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers: a panic is a failed test, with its message"
)]

mod harness;

use axum::http::StatusCode;
use harness::{memory_state, memory_store, mount_full_oidc_provider, sign_id_token, test};
use reqwest::Url;
use serde_json::{Value, json};
use tokio::sync::{Mutex, MutexGuard};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// RFC 7636 appendix B.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const LOOPBACK: &str = "http://127.0.0.1:18000/callback";

fn location(response: &axum::response::Response) -> Url {
    let raw = response
        .headers()
        .get("location")
        .expect("a redirect carries a Location")
        .to_str()
        .expect("an ASCII Location");
    Url::parse(raw).expect("a URL Location")
}

fn param(url: &Url, key: &str) -> String {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("{key} in {url}"))
        .1
        .into_owned()
}

fn encode(value: &str) -> String {
    serde_urlencoded::to_string([("v", value)])
        .expect("encodes")
        .trim_start_matches("v=")
        .to_string()
}

/// A login from the CLI, for `redirect_uri`.
fn login_uri(redirect_uri: &str) -> String {
    format!(
        "/api/v1/auth/login?redirect_uri={}&state=cli-state&code_challenge={CHALLENGE}",
        encode(redirect_uri)
    )
}

#[tokio::test]
async fn a_non_loopback_redirect_is_refused_before_the_idp() {
    let idp = MockServer::start().await;
    let app = test::init_service(harness::app(memory_state(&memory_store(), idp.uri()))).await;
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&login_uri("https://evil.example.com/steal"))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(idp.received_requests().await.expect("recorded").is_empty());
}

/// The whole session: the IdP calls the proxy back, the CLI gets a
/// single-use code on its loopback, never a token in a URL, and redeems it
/// with its verifier once.
#[tokio::test]
async fn the_cli_gets_its_session_through_a_single_use_code() {
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;
    let store = memory_store();
    let app = test::init_service(harness::app(memory_state(&store, idp.uri()))).await;

    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&login_uri(LOOPBACK))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let authorize = location(&response);
    assert_eq!(
        param(&authorize, "redirect_uri"),
        "https://proxy.example.com/api/v1/auth/callback"
    );
    assert_eq!(param(&authorize, "client_id"), "proxyauthk8s");
    let state = param(&authorize, "state");
    let nonce = param(&authorize, "nonce");
    let id_token = sign_id_token(&idp.uri(), "proxyauthk8s", "alice-sub", &nonce);
    harness::mount_token_endpoint(&idp, &id_token).await;

    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!(
                "/api/v1/auth/callback?code=idp-code&state={state}"
            ))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let back = location(&response);
    assert_eq!(back.host_str(), Some("127.0.0.1"));
    assert_eq!(param(&back, "state"), "cli-state");
    assert!(!back.as_str().contains("upstream-access-token"));
    let code = param(&back, "code");

    let redeem = |verifier: &'static str| {
        test::TestRequest::post()
            .uri("/api/v1/auth/token")
            .set_form([
                ("code", code.as_str()),
                ("redirect_uri", LOOPBACK),
                ("code_verifier", verifier),
            ])
            .to_request()
    };
    let response = test::call_service(&app, redeem(VERIFIER)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let session: Value = test::read_body_json(response).await;
    assert_eq!(session["access_token"], "upstream-access-token");
    assert_eq!(session["refresh_token"], "upstream-refresh-token");
    assert_eq!(session["expires_in"], 3600);

    // Single use: a replay finds nothing.
    let response = test::call_service(&app, redeem(VERIFIER)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// A code is only released to the holder of the verifier.
#[tokio::test]
async fn a_wrong_verifier_never_gets_the_session() {
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;
    let app = test::init_service(harness::app(memory_state(&memory_store(), idp.uri()))).await;
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&login_uri(LOOPBACK))
            .to_request(),
    )
    .await;
    let authorize = location(&response);
    let id_token = sign_id_token(
        &idp.uri(),
        "proxyauthk8s",
        "alice-sub",
        &param(&authorize, "nonce"),
    );
    harness::mount_token_endpoint(&idp, &id_token).await;
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!(
                "/api/v1/auth/callback?code=idp-code&state={}",
                param(&authorize, "state")
            ))
            .to_request(),
    )
    .await;
    let code = param(&location(&response), "code");
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/v1/auth/token")
            .set_form([
                ("code", code.as_str()),
                ("redirect_uri", LOOPBACK),
                ("code_verifier", &"x".repeat(43)),
            ])
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// Serialises the tests that set `UNSCOPED_*` (a plain `cargo test` runs them
/// as threads of one process).
static ENV_LOCK: Mutex<()> = Mutex::const_new(());

struct BanAfterOneFailure<'a> {
    _lock: MutexGuard<'a, ()>,
}

impl BanAfterOneFailure<'_> {
    async fn new() -> Self {
        let lock = ENV_LOCK.lock().await;
        // SAFETY: under the environment lock.
        #[expect(unsafe_code, reason = "set_var under the environment lock")]
        unsafe {
            std::env::set_var("UNSCOPED_MAX_FAILED_LOGINS", "1");
        }
        Self { _lock: lock }
    }
}

impl Drop for BanAfterOneFailure<'_> {
    fn drop(&mut self) {
        // SAFETY: under the environment lock, still held.
        #[expect(unsafe_code, reason = "remove_var under the environment lock")]
        unsafe {
            std::env::remove_var("UNSCOPED_MAX_FAILED_LOGINS");
        }
    }
}

async fn refresh(app: &axum::Router, from: &str) -> (StatusCode, Value) {
    let response = test::call_service(
        app,
        test::TestRequest::post()
            .uri("/api/v1/auth/refresh")
            .insert_header(("x-forwarded-for", from))
            .set_form([("refresh_token", "the-refresh-token")])
            .to_request(),
    )
    .await;
    let status = response.status();
    let body = serde_json::from_slice(&test::read_body(response).await).unwrap_or(Value::Null);
    (status, body)
}

#[tokio::test]
async fn a_refresh_renews_the_session() {
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=the-refresh-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "renewed",
            "token_type": "Bearer",
            "expires_in": 300,
        })))
        .expect(1)
        .mount(&idp)
        .await;
    let app = test::init_service(harness::app(memory_state(&memory_store(), idp.uri()))).await;
    let (status, session) = refresh(&app, "198.51.100.10").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["access_token"], "renewed");
    // No rotation: the CLI keeps the one it has.
    assert_eq!(session["refresh_token"], "the-refresh-token");
}

/// T12: a refresh token the IdP refuses counts toward the unscoped ban.
#[tokio::test]
async fn a_refused_refresh_counts_toward_a_ban() {
    let _env = BanAfterOneFailure::new().await;
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
        .expect(1)
        .mount(&idp)
        .await;
    let app = test::init_service(harness::app(memory_state(&memory_store(), idp.uri()))).await;
    let (status, body) = refresh(&app, "198.51.100.11").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_grant");
    // Banned now: refused before the IdP (`expect(1)` above).
    assert_eq!(
        refresh(&app, "198.51.100.11").await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

/// An IdP that cannot answer is not the caller's fault: `503`, no ban.
#[tokio::test]
async fn an_unavailable_idp_never_counts_toward_a_ban() {
    let _env = BanAfterOneFailure::new().await;
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&idp)
        .await;
    let app = test::init_service(harness::app(memory_state(&memory_store(), idp.uri()))).await;
    for _ in 0..3 {
        assert_eq!(
            refresh(&app, "198.51.100.12").await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
