//! Access-token validation against a wiremock OIDC provider.
//!
//! Complements `oidc_user.rs` (which covers claim mapping) with the rejection
//! paths that decide whether a bearer token is trusted at all:
//! - expired / revoked / unknown tokens (userinfo or introspection says no);
//! - tokens minted for another audience (the confused-deputy case);
//! - a provider whose discovery document names a different issuer;
//! - a broken `/userinfo` response;
//! - and the matching accept paths, so a regression that rejects everything
//!   is caught as well.
//!
//! Every test starts its own provider under a unique issuer URL (see
//! [`Provider`]) and mints a unique token, so a process-wide discovery or validated-token cache can never
//! leak an earlier test's verdict into a later one. No environment variable is
//! set: tests run in the default `Enforce` audience mode.
//!
//! None of these tests need Redis: they call the validation functions directly
//! or go through the `User` extractor with a `State` whose Redis is
//! unreachable (the OIDC path never touches it).

mod harness;

use std::sync::atomic::{AtomicUsize, Ordering};

use actix_web::{App, HttpResponse, http::StatusCode, test, web};
use api::model::user::{User, UserAuthError};
use base64::Engine;
use common::oidc_conf::OidcConf;
use harness::{oidc_auth_config, proxy_fixture, unreachable_state};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CLIENT_ID: &str = "proxyauthk8s";

static TOKEN_COUNTER: AtomicUsize = AtomicUsize::new(0);
static REALM_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A wiremock OIDC provider served under a per-test realm path.
///
/// `wiremock` recycles `MockServer`s (and so their ports) across tests in one
/// process, while discovery / introspection-endpoint lookups are cached per
/// issuer URL. A unique `/realm-N` path segment makes the issuer URL unique per
/// test, so a cached answer from an earlier test can never be served here.
struct Provider {
    server: MockServer,
    issuer: String,
}

impl Provider {
    async fn start() -> Self {
        let server = MockServer::start().await;
        let realm = format!(
            "realm-{}-{}",
            std::process::id(),
            REALM_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let issuer = format!("{}/{realm}", server.uri());
        Self { server, issuer }
    }

    /// The issuer URL (server base plus realm).
    fn uri(&self) -> String {
        self.issuer.clone()
    }

    /// `suffix` (e.g. `/userinfo`) under this provider's realm, for matchers.
    fn path(&self, suffix: &str) -> String {
        let realm_path = self
            .issuer
            .strip_prefix(&self.server.uri())
            .expect("issuer starts with the server uri");
        format!("{realm_path}{suffix}")
    }
}

impl std::ops::Deref for Provider {
    type Target = MockServer;

    fn deref(&self) -> &MockServer {
        &self.server
    }
}

/// Build a JWT-shaped access token carrying `claims` plus a unique `jti`.
///
/// The signature is never verified by the proxy (`/userinfo` is the authority
/// on validity), so a fixed placeholder is enough. The `jti` makes every token
/// distinct, which keeps any token-keyed cache from short-circuiting a test.
fn jwt(mut claims: Value) -> String {
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let jti = format!(
        "jti-{}-{}",
        std::process::id(),
        TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    claims["jti"] = json!(jti);
    format!(
        "{}.{}.{}",
        b64.encode(br#"{"alg":"RS256","typ":"JWT"}"#),
        b64.encode(claims.to_string().as_bytes()),
        b64.encode(b"signature-not-verified-in-these-tests")
    )
}

/// A unique opaque (non-JWT) token.
fn opaque_token() -> String {
    format!(
        "opaque-{}-{}",
        std::process::id(),
        TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs()
}

fn oidc_conf(issuer_url: &str) -> OidcConf {
    OidcConf {
        client_id: CLIENT_ID.to_string(),
        client_secret: Some("secret".to_string()),
        issuer_url: issuer_url.to_string(),
        scopes: "openid email profile groups".to_string(),
        audience: CLIENT_ID.to_string(),
        accept_authorized_party: false,
        redirect_url: None,
    }
}

/// Mount the discovery document (and its `jwks_uri`).
///
/// `issuer_override` lets a test advertise an issuer that differs from the URL
/// the document is served from; `introspection` adds an RFC 7662 endpoint.
async fn mount_discovery(server: &Provider, issuer_override: Option<&str>, introspection: bool) {
    let base = server.uri();
    let issuer = issuer_override.map_or_else(|| base.clone(), str::to_string);
    let mut doc = json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "userinfo_endpoint": format!("{base}/userinfo"),
        "jwks_uri": format!("{base}/jwks"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "scopes_supported": ["openid", "email", "profile", "groups"],
    });
    if introspection {
        doc["introspection_endpoint"] = json!(format!("{base}/introspect"));
    }
    Mock::given(method("GET"))
        .and(path(server.path("/.well-known/openid-configuration")))
        .respond_with(ResponseTemplate::new(200).set_body_json(doc))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(server.path("/jwks")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "keys": [] })))
        .mount(server)
        .await;
}

/// `/userinfo` accepts exactly `token` and resolves it to alice.
async fn mount_userinfo_ok(server: &Provider, token: &str) {
    Mock::given(method("GET"))
        .and(path(server.path("/userinfo")))
        .and(header("authorization", format!("Bearer {token}").as_str()))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({
                    "sub": "alice-sub",
                    "preferred_username": "alice",
                    "email": "alice@example.com",
                    "groups": ["platform"],
                })),
        )
        .mount(server)
        .await;
}

/// `/userinfo` rejects every token the way a provider rejects an expired one.
async fn mount_userinfo_unauthorized(server: &Provider) {
    Mock::given(method("GET"))
        .and(path(server.path("/userinfo")))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header(
                    "www-authenticate",
                    r#"Bearer error="invalid_token", error_description="The access token expired""#,
                )
                .set_body_json(json!({ "error": "invalid_token" })),
        )
        .mount(server)
        .await;
}

async fn mount_introspection(server: &Provider, body: Value) {
    Mock::given(method("POST"))
        .and(path(server.path("/introspect")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(body),
        )
        .mount(server)
        .await;
}

fn assert_audience_rejected(result: &Result<Option<User>, UserAuthError>) {
    assert!(
        matches!(result, Err(UserAuthError::AudienceValidation(_))),
        "expected an audience rejection, got {result:?}"
    );
}

fn assert_accepted(result: Result<Option<User>, UserAuthError>) -> User {
    let user = result
        .expect("token should be accepted")
        .expect("user should be present");
    assert_eq!(user.username, "alice");
    assert_eq!(user.groups, vec!["platform"]);
    user
}

// ---------------------------------------------------------------------------
// Valid token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn accepts_an_unexpired_token_whose_aud_names_the_service() {
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    let token = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub", "exp": now() + 3600 }));
    mount_userinfo_ok(&server, &token).await;

    let result = User::get_user_info_from_oidc_token(token, oidc_conf(&server.uri())).await;

    let user = assert_accepted(result);
    assert_eq!(user.email, "alice@example.com");
}

#[tokio::test]
async fn accepts_a_token_listing_the_service_among_several_audiences() {
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    let token = jwt(json!({ "aud": ["billing-api", CLIENT_ID], "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &token).await;

    let result = User::get_user_info_from_oidc_token(token, oidc_conf(&server.uri())).await;

    assert_accepted(result);
}

// ---------------------------------------------------------------------------
// Expired / invalid token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rejects_an_expired_token_refused_by_userinfo() {
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    mount_userinfo_unauthorized(&server).await;
    // Correct audience, but expired: the provider is the authority and says no.
    let token = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub", "exp": now() - 3600 }));

    let result = User::get_user_info_from_oidc_token(token, oidc_conf(&server.uri())).await;

    assert!(
        matches!(result, Err(UserAuthError::UserInfoResponse)),
        "expected a userinfo rejection, got {result:?}"
    );
}

#[tokio::test]
async fn rejects_a_malformed_userinfo_response() {
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    Mock::given(method("GET"))
        .and(path(server.path("/userinfo")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string("{not json"),
        )
        .mount(&server)
        .await;
    let token = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub" }));

    let result = User::get_user_info_from_oidc_token(token, oidc_conf(&server.uri())).await;

    assert!(
        matches!(result, Err(UserAuthError::UserInfoResponse)),
        "expected a userinfo rejection, got {result:?}"
    );
}

#[tokio::test]
async fn rejects_a_token_that_introspection_reports_inactive() {
    // `/userinfo` still answers (e.g. a stale provider cache), the JWT `aud` is
    // right, but introspection says the token is revoked/expired: reject.
    let server = Provider::start().await;
    mount_discovery(&server, None, true).await;
    let token = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &token).await;
    mount_introspection(&server, json!({ "active": false })).await;

    let result = User::get_user_info_from_oidc_token(token, oidc_conf(&server.uri())).await;

    assert_audience_rejected(&result);
}

// ---------------------------------------------------------------------------
// Wrong audience
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rejects_a_token_minted_for_another_audience() {
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    let token = jwt(json!({ "aud": "billing-api", "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &token).await;

    let result = User::get_user_info_from_oidc_token(token, oidc_conf(&server.uri())).await;

    assert_audience_rejected(&result);
}

#[tokio::test]
async fn rejects_a_foreign_audience_token_even_when_azp_is_this_client() {
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    let token = jwt(json!({ "aud": ["billing-api"], "azp": CLIENT_ID, "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &token).await;

    let result = User::get_user_info_from_oidc_token(token, oidc_conf(&server.uri())).await;

    assert_audience_rejected(&result);
}

#[tokio::test]
async fn accepts_azp_only_when_authorized_party_acceptance_is_enabled() {
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    let token = jwt(json!({ "aud": ["account"], "azp": CLIENT_ID, "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &token).await;
    let mut conf = oidc_conf(&server.uri());
    conf.accept_authorized_party = true;

    let result = User::get_user_info_from_oidc_token(token, conf).await;

    assert_accepted(result);
}

#[tokio::test]
async fn rejects_an_opaque_token_when_the_audience_cannot_be_determined() {
    // Valid per `/userinfo`, but opaque and no introspection endpoint: the
    // audience is unknown and the default mode fails closed.
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    let token = opaque_token();
    mount_userinfo_ok(&server, &token).await;

    let result = User::get_user_info_from_oidc_token(token, oidc_conf(&server.uri())).await;

    assert_audience_rejected(&result);
}

#[tokio::test]
async fn introspection_accepts_an_opaque_token_for_this_audience() {
    let server = Provider::start().await;
    mount_discovery(&server, None, true).await;
    let token = opaque_token();
    mount_userinfo_ok(&server, &token).await;
    mount_introspection(&server, json!({ "active": true, "aud": [CLIENT_ID] })).await;

    let result = User::get_user_info_from_oidc_token(token, oidc_conf(&server.uri())).await;

    assert_accepted(result);
}

#[tokio::test]
async fn introspection_audience_wins_over_the_jwt_claims() {
    // The JWT claims our audience, but the authoritative introspection response
    // says the token is for another resource: introspection must win.
    let server = Provider::start().await;
    mount_discovery(&server, None, true).await;
    let token = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &token).await;
    mount_introspection(
        &server,
        json!({ "active": true, "aud": "billing-api", "client_id": CLIENT_ID }),
    )
    .await;

    let result = User::get_user_info_from_oidc_token(token, oidc_conf(&server.uri())).await;

    assert_audience_rejected(&result);
}

#[tokio::test]
async fn failing_introspection_falls_back_to_the_jwt_audience() {
    let server = Provider::start().await;
    mount_discovery(&server, None, true).await;
    Mock::given(method("POST"))
        .and(path(server.path("/introspect")))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let good = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &good).await;
    let result = User::get_user_info_from_oidc_token(good, oidc_conf(&server.uri())).await;
    assert_accepted(result);

    // ...and the fallback still enforces the audience.
    let foreign = jwt(json!({ "aud": "billing-api", "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &foreign).await;
    let result = User::get_user_info_from_oidc_token(foreign, oidc_conf(&server.uri())).await;
    assert_audience_rejected(&result);
}

// ---------------------------------------------------------------------------
// Wrong issuer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rejects_a_provider_whose_discovery_names_another_issuer() {
    let server = Provider::start().await;
    mount_discovery(&server, Some("https://evil.example.com"), false).await;
    let token = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub" }));
    // If discovery were (wrongly) accepted, userinfo would succeed.
    Mock::given(method("GET"))
        .and(path(server.path("/userinfo")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({ "sub": "alice-sub", "groups": ["platform"] })),
        )
        .expect(0)
        .mount(&server)
        .await;

    let result = User::get_user_info_from_oidc_token(token, oidc_conf(&server.uri())).await;

    assert!(
        matches!(result, Err(UserAuthError::OidcCore(_))),
        "expected an issuer/discovery rejection, got {result:?}"
    );
}

#[tokio::test]
async fn rejects_when_the_issuer_url_is_not_a_url() {
    let token = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub" }));

    let result = User::get_user_info_from_oidc_token(token, oidc_conf("not a url")).await;

    assert!(
        matches!(result, Err(UserAuthError::OidcCore(_))),
        "expected an OIDC core error, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Per-cluster provider: `User::auth_against_oidc_provider`
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cluster_provider_without_audience_falls_back_to_the_client_id() {
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    let token = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &token).await;
    let mut proxy = proxy_fixture("default", "token-validation", "https://127.0.0.1:1");
    proxy.spec.auth_config = Some(oidc_auth_config(&server.uri()));

    let result =
        User::auth_against_oidc_provider(unreachable_state(server.uri()), proxy, token).await;

    assert_accepted(result);
}

#[tokio::test]
async fn cluster_provider_enforces_its_configured_audience() {
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    let mut config = oidc_auth_config(&server.uri());
    config.oidc_provider.audience = "cluster-api".to_string();
    let mut proxy = proxy_fixture("default", "token-validation", "https://127.0.0.1:1");
    proxy.spec.auth_config = Some(config);

    // aud = client id is no longer enough once a distinct audience is set.
    let client_aud = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &client_aud).await;
    let result = User::auth_against_oidc_provider(
        unreachable_state(server.uri()),
        proxy.clone(),
        client_aud,
    )
    .await;
    assert_audience_rejected(&result);

    let cluster_aud = jwt(json!({ "aud": "cluster-api", "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &cluster_aud).await;
    let result =
        User::auth_against_oidc_provider(unreachable_state(server.uri()), proxy, cluster_aud).await;
    assert_accepted(result);
}

#[tokio::test]
async fn cluster_provider_disabled_means_no_oidc_configuration() {
    let server = Provider::start().await;
    let mut config = oidc_auth_config(&server.uri());
    config.oidc_provider.enabled = false;
    let mut proxy = proxy_fixture("default", "token-validation", "https://127.0.0.1:1");
    proxy.spec.auth_config = Some(config);
    let token = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub" }));

    let result =
        User::auth_against_oidc_provider(unreachable_state(server.uri()), proxy, token).await;

    assert!(
        matches!(result, Err(UserAuthError::OidcConfigMissing)),
        "expected OidcConfigMissing, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// HTTP: the `User` extractor collapses every rejection into a 401
// ---------------------------------------------------------------------------

async fn whoami(user: User) -> HttpResponse {
    HttpResponse::Ok().body(user.username)
}

async fn call_whoami(issuer: String, token: &str) -> (StatusCode, String) {
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(unreachable_state(issuer)))
            .route("/whoami", web::get().to(whoami)),
    )
    .await;
    let req = test::TestRequest::get()
        .uri("/whoami")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let status = resp.status();
    let body = test::read_body(resp).await;
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[actix_web::test]
async fn extractor_returns_200_for_a_valid_token() {
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    let token = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub", "exp": now() + 3600 }));
    mount_userinfo_ok(&server, &token).await;

    let (status, body) = call_whoami(server.uri(), &token).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "alice");
}

#[actix_web::test]
async fn extractor_returns_401_for_an_expired_token() {
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    mount_userinfo_unauthorized(&server).await;
    let token = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub", "exp": now() - 3600 }));

    let (status, _) = call_whoami(server.uri(), &token).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
async fn extractor_returns_401_for_a_wrong_audience() {
    let server = Provider::start().await;
    mount_discovery(&server, None, false).await;
    let token = jwt(json!({ "aud": "billing-api", "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &token).await;

    let (status, body) = call_whoami(server.uri(), &token).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // The rejection reason must not leak to the caller.
    assert!(!body.contains("billing-api"), "leaked detail: {body}");
}

#[actix_web::test]
async fn extractor_returns_401_for_a_wrong_issuer() {
    let server = Provider::start().await;
    mount_discovery(&server, Some("https://evil.example.com"), false).await;
    let token = jwt(json!({ "aud": CLIENT_ID, "sub": "alice-sub" }));
    mount_userinfo_ok(&server, &token).await;

    let (status, _) = call_whoami(server.uri(), &token).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
