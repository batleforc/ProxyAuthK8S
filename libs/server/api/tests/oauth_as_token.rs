//! Fast-tier integration tests for `POST /{ns}/{cluster}/oauth/token`, the
//! token endpoint of the mediated OAuth Authorization Server flow.
//!
//! The upstream exchange itself (`/oauth/callback` minting an `IssuedCode`)
//! needs a signed upstream ID token and isn't exercised here; these tests
//! seed an `IssuedCode` directly, the same shape `/oauth/callback` would
//! have written, and exercise the token endpoint's own contract: PKCE
//! verification, `redirect_uri` binding, and single-use redemption.

mod harness;

use actix_web::{test, web, App};
use api::cluster::auth::oauth::{model::IssuedCode, token};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use deadpool_redis::redis::AsyncTypedCommands;
use harness::{
    delete_proxy, oidc_auth_config_with_well_known, proxy_fixture, seed_proxy, test_state,
    try_redis_pool, unique_cluster,
};
use sha2::{Digest, Sha256};

const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

fn s256_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

macro_rules! token_app {
    ($state:expr) => {
        test::init_service(
            App::new()
                .app_data(web::Data::new($state))
                .service(web::scope("/clusters").service(token::token)),
        )
        .await
    };
}

macro_rules! redis_or_skip {
    () => {
        match try_redis_pool().await {
            Some(pool) => pool,
            None => return,
        }
    };
}

async fn seed_code(pool: &deadpool_redis::Pool, ns: &str, cluster: &str, code: &str, issued: &IssuedCode) {
    let mut conn = pool.get().await.expect("redis connection");
    conn.set_ex(
        format!("oauth_as_code:{ns}/{cluster}/{code}"),
        serde_json::to_string(issued).expect("issued code should serialize"),
        60,
    )
    .await
    .expect("issued code should be cached");
}

fn issued_code(redirect_uri: &str) -> IssuedCode {
    IssuedCode {
        redirect_uri: redirect_uri.to_string(),
        code_challenge: s256_challenge(VERIFIER),
        access_token: "upstream-access-token".to_string(),
        refresh_token: Some("upstream-refresh-token".to_string()),
        id_token: "upstream-id-token".to_string(),
        scope: "openid email profile groups".to_string(),
        expires_in: Some(3600),
    }
}

#[actix_web::test]
async fn exchanges_a_valid_code_for_the_upstream_tokens() {
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known("https://issuer.example.com"));
    seed_proxy(&pool, &proxy).await;
    seed_code(
        &pool,
        &ns,
        &cluster,
        "the-code",
        &issued_code("http://localhost:12345/callback"),
    )
    .await;

    let app = token_app!(test_state("https://issuer.example.com".to_string()));
    let req = test::TestRequest::post()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/token"))
        .set_form([
            ("grant_type", "authorization_code"),
            ("code", "the-code"),
            ("redirect_uri", "http://localhost:12345/callback"),
            ("code_verifier", VERIFIER),
        ])
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["access_token"], "upstream-access-token");
    assert_eq!(body["refresh_token"], "upstream-refresh-token");
    assert_eq!(body["id_token"], "upstream-id-token");
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 3600);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn a_code_can_only_be_redeemed_once() {
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known("https://issuer.example.com"));
    seed_proxy(&pool, &proxy).await;
    seed_code(
        &pool,
        &ns,
        &cluster,
        "the-code",
        &issued_code("http://localhost:12345/callback"),
    )
    .await;

    let app = token_app!(test_state("https://issuer.example.com".to_string()));
    let form = || {
        [
            ("grant_type", "authorization_code"),
            ("code", "the-code"),
            ("redirect_uri", "http://localhost:12345/callback"),
            ("code_verifier", VERIFIER),
        ]
    };
    let first = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/clusters/{ns}/{cluster}/oauth/token"))
            .set_form(form())
            .to_request(),
    )
    .await;
    assert_eq!(first.status(), 200);

    let second = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/clusters/{ns}/{cluster}/oauth/token"))
            .set_form(form())
            .to_request(),
    )
    .await;
    assert_eq!(second.status(), 400);
    let body: serde_json::Value = test::read_body_json(second).await;
    assert_eq!(body["error"], "invalid_grant");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn rejects_a_mismatched_pkce_verifier() {
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known("https://issuer.example.com"));
    seed_proxy(&pool, &proxy).await;
    seed_code(
        &pool,
        &ns,
        &cluster,
        "the-code",
        &issued_code("http://localhost:12345/callback"),
    )
    .await;

    let app = token_app!(test_state("https://issuer.example.com".to_string()));
    let req = test::TestRequest::post()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/token"))
        .set_form([
            ("grant_type", "authorization_code"),
            ("code", "the-code"),
            ("redirect_uri", "http://localhost:12345/callback"),
            ("code_verifier", "wrong-verifier-wrong-verifier-wrong-verifi"),
        ])
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "invalid_grant");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn rejects_a_mismatched_redirect_uri() {
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known("https://issuer.example.com"));
    seed_proxy(&pool, &proxy).await;
    seed_code(
        &pool,
        &ns,
        &cluster,
        "the-code",
        &issued_code("http://localhost:12345/callback"),
    )
    .await;

    let app = token_app!(test_state("https://issuer.example.com".to_string()));
    let req = test::TestRequest::post()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/token"))
        .set_form([
            ("grant_type", "authorization_code"),
            ("code", "the-code"),
            ("redirect_uri", "http://localhost:9999/other"),
            ("code_verifier", VERIFIER),
        ])
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "invalid_grant");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn rejects_an_unknown_code() {
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known("https://issuer.example.com"));
    seed_proxy(&pool, &proxy).await;

    let app = token_app!(test_state("https://issuer.example.com".to_string()));
    let req = test::TestRequest::post()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/token"))
        .set_form([
            ("grant_type", "authorization_code"),
            ("code", "never-issued"),
            ("redirect_uri", "http://localhost:12345/callback"),
            ("code_verifier", VERIFIER),
        ])
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "invalid_grant");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn rejects_an_unsupported_grant_type() {
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known("https://issuer.example.com"));
    seed_proxy(&pool, &proxy).await;

    let app = token_app!(test_state("https://issuer.example.com".to_string()));
    let req = test::TestRequest::post()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/token"))
        .set_form([
            ("grant_type", "client_credentials"),
            ("code", "the-code"),
            ("redirect_uri", "http://localhost:12345/callback"),
            ("code_verifier", VERIFIER),
        ])
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["error"], "unsupported_grant_type");

    delete_proxy(&pool, &ns, &cluster).await;
}
