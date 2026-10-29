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

    let user =
        User::get_user_info_from_oidc_token(VALID_TOKEN.to_string(), oidc_conf(&server.uri()))
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

    let user =
        User::get_user_info_from_oidc_token(VALID_TOKEN.to_string(), oidc_conf(&server.uri()))
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

    let user =
        User::get_user_info_from_oidc_token(VALID_TOKEN.to_string(), oidc_conf(&server.uri()))
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

    let result =
        User::get_user_info_from_oidc_token("expired-token".to_string(), oidc_conf(&server.uri()))
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

    let result =
        User::get_user_info_from_oidc_token(VALID_TOKEN.to_string(), oidc_conf(&server.uri()))
            .await;

    assert!(result.is_err(), "expected an error, got {result:?}");
}

#[tokio::test]
async fn fails_when_discovery_is_unavailable() {
    let server = MockServer::start().await;
    // No discovery document mounted at all.
    let result =
        User::get_user_info_from_oidc_token(VALID_TOKEN.to_string(), oidc_conf(&server.uri()))
            .await;

    assert!(result.is_err(), "expected an error, got {result:?}");
}

// RFC 0014: `oidc_provider.claim_mappings` over the raw `/userinfo` claims.

fn mappings(value: serde_json::Value) -> crd::authentication_configuration::ClaimMappings {
    serde_json::from_value(value).expect("claim mappings")
}

async fn mapped(
    server: &MockServer,
    mappings: &crd::authentication_configuration::ClaimMappings,
    trusted: bool,
) -> Result<Option<User>, api::model::user::UserAuthError> {
    User::get_user_info_mapped(
        VALID_TOKEN.to_string(),
        oidc_conf(&server.uri()),
        Some(api::model::user::Mapping { mappings, trusted }),
    )
    .await
}

#[tokio::test]
async fn claim_mappings_read_any_userinfo_claim() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;
    mount_userinfo(
        &server,
        json!({
            "sub": "alice-sub",
            "preferred_username": "alice",
            "email": "alice@example.com",
            "teams": ["blue"],
            "groups": ["dev"],
        }),
    )
    .await;
    let config = mappings(json!({
        "username": { "claim": "sub", "prefix": "" },
        "groups": { "expression": "claims.teams" },
        "extra": [{ "key": "example.com/team", "value_expression": "claims.teams" }],
    }));
    let user = mapped(&server, &config, false)
        .await
        .expect("mapped")
        .expect("user");
    assert_eq!(user.username, "alice-sub");
    assert_eq!(user.groups, vec!["blue"]);
    assert_eq!(user.email, "alice@example.com");
    assert_eq!(user.extra["example.com/team"], vec!["blue"]);
}

/// Decision 38: one token, one OIDC client, two clusters with different
/// mappings: each gets its own user, never the other's cached one.
#[tokio::test]
async fn a_mapping_from_one_cluster_never_serves_another() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;
    mount_userinfo(
        &server,
        json!({ "sub": "alice-sub", "preferred_username": "alice", "groups": [] }),
    )
    .await;
    let by_sub = mappings(json!({ "username": { "claim": "sub", "prefix": "" } }));
    let everyone_admin = mappings(json!({ "username": { "expression": "'admin'" } }));

    let first = mapped(&server, &everyone_admin, false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.username, "admin");
    let second = mapped(&server, &by_sub, true).await.unwrap().unwrap();
    assert_eq!(second.username, "alice-sub");
    let legacy =
        User::get_user_info_from_oidc_token(VALID_TOKEN.to_string(), oidc_conf(&server.uri()))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(legacy.username, "alice");
}

/// Decision 51: in a trusted mode, a username read from an unverified email
/// is refused; untrusted, it is accepted as before.
#[tokio::test]
async fn an_unverified_email_is_refused_as_username_in_a_trusted_mode() {
    let server = MockServer::start().await;
    mount_discovery(&server).await;
    mount_userinfo(
        &server,
        json!({ "sub": "alice-sub", "email": "alice@example.com", "email_verified": false }),
    )
    .await;
    let by_email = mappings(json!({ "username": { "claim": "email", "prefix": "" } }));
    assert!(mapped(&server, &by_email, true).await.is_err());
    let user = mapped(&server, &by_email, false).await.unwrap().unwrap();
    assert_eq!(user.username, "alice@example.com");
}
