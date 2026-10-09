//! `User::get_user_info_from_oidc_token` against a wiremock OIDC provider.

use api::model::user::User;
use common::oidc_conf::OidcConf;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A JWT-shaped access token whose `aud` claim names the configured audience
/// (`proxyauthk8s`). `/userinfo` proves the token is valid; the audience check
/// that follows reads this `aud`, so a realistic signed token must carry it —
/// an opaque string would be (correctly) rejected by the default enforce mode.
/// Header/payload/signature are base64url; the signature is never verified here.
const VALID_TOKEN: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJwcm94eWF1dGhrOHMiLCJzdWIiOiJhbGljZS1zdWIifQ.c2lnbmF0dXJlLW5vdC12ZXJpZmllZC1pbi10aGVzZS10ZXN0cw";

/// A cache with nothing in it.
///
/// Each test gets its own, because these tests assert on what the provider is
/// actually asked for — a shared cache would let one test's discovery satisfy
/// another's fetch and quietly stop exercising the path under test.
fn fresh_cache() -> common::discovery_cache::DiscoveryCache {
    common::discovery_cache::DiscoveryCache::new()
}

fn oidc_conf(issuer_url: &str) -> OidcConf {
    OidcConf {
        client_id: "proxyauthk8s".to_string(),
        client_secret: Some("secret".to_string()),
        issuer_url: issuer_url.to_string(),
        scopes: "openid email profile groups".to_string(),
        audience: "proxyauthk8s".to_string(),
        accept_authorized_party: false,
        redirect_url: None,
    }
}

/// Minimal discovery document accepted by `CoreProviderMetadata`.
///
/// Discovery is two round-trips: the well-known document, then the `jwks_uri`
/// it points at. Both have to be mounted or discovery fails with a 404.
async fn mount_discovery(server: &MockServer) {
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
            "scopes_supported": ["openid", "email", "profile", "groups"],
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "keys": [] })))
        .mount(server)
        .await;
}

async fn mount_userinfo(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/userinfo"))
        .and(header(
            "authorization",
            format!("Bearer {VALID_TOKEN}").as_str(),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(body),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn resolves_a_user_from_the_userinfo_endpoint() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;
    mount_userinfo(
        &server,
        json!({
            "sub": "alice-sub",
            "preferred_username": "alice",
            "email": "alice@example.com",
            "groups": ["dev-alice", "platform"],
        }),
    )
    .await;

    let user = User::get_user_info_from_oidc_token(
        VALID_TOKEN.to_string(),
        oidc_conf(&server.uri()),
        &fresh_cache(),
    )
    .await
    .expect("user info should resolve")
    .expect("user should be present");

    assert_eq!(user.username, "alice");
    assert_eq!(user.email, "alice@example.com");
    assert_eq!(user.groups, vec!["dev-alice", "platform"]);
    assert!(user.is_in_group("platform"));
    assert!(!user.is_in_group("admins"));
}

#[tokio::test]
async fn tolerates_missing_optional_claims() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;
    mount_userinfo(&server, json!({ "sub": "alice-sub", "groups": [] })).await;

    let user = User::get_user_info_from_oidc_token(
        VALID_TOKEN.to_string(),
        oidc_conf(&server.uri()),
        &fresh_cache(),
    )
    .await
    .expect("user info should resolve")
    .expect("user should be present");

    assert_eq!(user.username, "");
    assert_eq!(user.email, "");
    assert!(user.groups.is_empty());
}

#[tokio::test]
async fn resolves_a_user_from_a_response_without_the_groups_claim() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;
    // Providers tag `groups` as `omitempty` (Dex and Authentik both do), so a
    // user in no group gets a payload with no `groups` key. That must resolve to
    // a group-less user rather than a parse failure — the group-restriction
    // check downstream is what refuses access, and an empty list matches nothing.
    mount_userinfo(
        &server,
        json!({ "sub": "alice-sub", "preferred_username": "alice" }),
    )
    .await;

    let user = User::get_user_info_from_oidc_token(
        VALID_TOKEN.to_string(),
        oidc_conf(&server.uri()),
        &fresh_cache(),
    )
    .await
    .expect("user info should resolve")
    .expect("user should be present");

    assert_eq!(user.username, "alice");
    assert!(user.groups.is_empty());
    assert!(!user.is_in_group("admins"));
}

#[tokio::test]
async fn rejects_an_unknown_token() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;
    Mock::given(method("GET"))
        .and(path("/userinfo"))
        .respond_with(ResponseTemplate::new(401).set_body_string("invalid_token"))
        .mount(&server)
        .await;

    let result = User::get_user_info_from_oidc_token(
        "expired-token".to_string(),
        oidc_conf(&server.uri()),
        &fresh_cache(),
    )
    .await;

    assert!(result.is_err(), "expected an error, got {result:?}");
}

#[tokio::test]
async fn surfaces_a_provider_error() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;
    Mock::given(method("GET"))
        .and(path("/userinfo"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let result = User::get_user_info_from_oidc_token(
        VALID_TOKEN.to_string(),
        oidc_conf(&server.uri()),
        &fresh_cache(),
    )
    .await;

    assert!(result.is_err(), "expected an error, got {result:?}");
}

#[tokio::test]
async fn fails_when_discovery_is_unavailable() {
    let server = MockServer::start().await;
    // No discovery document mounted at all.
    let result = User::get_user_info_from_oidc_token(
        VALID_TOKEN.to_string(),
        oidc_conf(&server.uri()),
        &fresh_cache(),
    )
    .await;

    assert!(result.is_err(), "expected an error, got {result:?}");
}
