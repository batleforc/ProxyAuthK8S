//! Fast-tier integration tests for the full, successful login exchange —
//! the one piece neither `oauth_as_*.rs` nor the older callback tests cover,
//! because it needs a real signed+verifiable upstream ID token.
//!
//! Covers both cluster login mechanisms end to end:
//! - the existing, `User`-authenticated `/auth/login` -> `/auth/callback`
//!   flow (`cluster_login`/`callback_login`);
//! - the new, unauthenticated, mediated `/oauth/authorize` ->
//!   `/oauth/callback` -> `/oauth/token` Authorization Server flow.
//!
//! Both ultimately run the same ID token verification code path
//! (nonce check, signature against the provider's JWKS); this suite is what
//! actually exercises that path against a real signature, via
//! `harness::{sign_id_token, mount_full_oidc_provider, mount_token_endpoint}`.

mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::auth::{callback::callback_login, login::cluster_login, oauth};
use harness::{
    delete_proxy, mount_full_oidc_provider, mount_token_endpoint, oidc_auth_config,
    oidc_auth_config_with_well_known, proxy_fixture, seed_proxy, sign_id_token, test_state,
    try_redis_pool, unique_cluster,
};
use reqwest::Url;

/// A JWT-shaped access token whose `aud` claim names the front's configured
/// audience (`proxyauthk8s`) — needed for `/auth/login`'s `User` extractor,
/// which authenticates against the front OIDC client, not the per-cluster one.
const FRONT_TOKEN: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJwcm94eWF1dGhrOHMiLCJzdWIiOiJhbGljZS1zdWIifQ.c2lnbmF0dXJlLW5vdC12ZXJpZmllZC1pbi10aGVzZS10ZXN0cw";

// RFC 7636 Appendix B test vector.
const EXTERNAL_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const EXTERNAL_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

macro_rules! login_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new().app_data(web::Data::new($state)).service(
                web::scope("/clusters")
                    .service(cluster_login)
                    .service(callback_login)
                    .service(oauth::authorize::authorize)
                    .service(oauth::callback::callback)
                    .service(oauth::token::token),
            ),
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

fn query_param(url: &Url, key: &str) -> String {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("{key} should be present in {url}"))
        .1
        .into_owned()
}

fn location(resp: &actix_web::dev::ServiceResponse) -> Url {
    let raw = resp
        .headers()
        .get("location")
        .expect("a redirect must carry a Location header")
        .to_str()
        .expect("Location must be ASCII");
    Url::parse(raw).expect("Location must be a valid URL")
}

#[actix_web::test]
async fn existing_auth_login_callback_flow_succeeds_with_a_real_signed_id_token() {
    let pool = redis_or_skip!();
    let idp = harness_mock_server().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = login_app!(test_state(idp.uri()));

    // Step 1: authenticated login start — returns the upstream authorize URL as plain text.
    let login_req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/auth/login"))
        .insert_header(("authorization", format!("Bearer {FRONT_TOKEN}")))
        .to_request();
    let login_resp = test::call_service(&app, login_req).await;
    assert_eq!(login_resp.status(), StatusCode::OK);
    let auth_url_raw = String::from_utf8(test::read_body(login_resp).await.to_vec())
        .expect("auth_url body should be UTF-8");
    let auth_url = Url::parse(&auth_url_raw).expect("auth_url should be a valid URL");
    let nonce = query_param(&auth_url, "nonce");
    let state = query_param(&auth_url, "state");

    // Step 2: the upstream provider "authenticates" the user and calls back.
    // We stand in for it: sign a real ID token with the nonce the server chose,
    // and mount the token endpoint that will hand it back on exchange.
    let id_token = sign_id_token(&idp.uri(), "proxyauthk8s", "alice-sub", &nonce);
    mount_token_endpoint(&idp, &id_token).await;

    let callback_req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/auth/callback?code=upstream-code&state={state}"
        ))
        .to_request();
    let callback_resp = test::call_service(&app, callback_req).await;
    assert_eq!(callback_resp.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(callback_resp).await;
    assert_eq!(body["access_token"], "upstream-access-token");
    assert_eq!(body["refresh_token"], "upstream-refresh-token");
    assert_eq!(body["id_token"], id_token);
    assert_eq!(body["subject"], "alice-sub");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn mediated_oauth_flow_succeeds_with_a_real_signed_id_token() {
    let pool = redis_or_skip!();
    let idp = harness_mock_server().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = login_app!(test_state(idp.uri()));

    // Step 1: unauthenticated start — the external client's own PKCE
    // challenge/state/redirect_uri, none of which need prior registration.
    let authorize_req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/authorize?response_type=code&client_id=my-cli\
             &redirect_uri=http%3A%2F%2Flocalhost%3A12345%2Fcallback&state=external-state\
             &code_challenge={EXTERNAL_CHALLENGE}&code_challenge_method=S256"
        ))
        .to_request();
    let authorize_resp = test::call_service(&app, authorize_req).await;
    assert_eq!(authorize_resp.status(), StatusCode::FOUND);
    let upstream_auth_url = location(&authorize_resp);
    let nonce = query_param(&upstream_auth_url, "nonce");
    let correlation_id = query_param(&upstream_auth_url, "state");

    // Step 2: the upstream provider calls back to the proxy's own /oauth/callback.
    let id_token = sign_id_token(&idp.uri(), "proxyauthk8s", "alice-sub", &nonce);
    mount_token_endpoint(&idp, &id_token).await;

    let callback_req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/callback?code=upstream-code&state={correlation_id}"
        ))
        .to_request();
    let callback_resp = test::call_service(&app, callback_req).await;
    assert_eq!(callback_resp.status(), StatusCode::FOUND);
    let client_redirect = location(&callback_resp);
    assert_eq!(
        client_redirect.origin().unicode_serialization(),
        "http://localhost:12345"
    );
    assert_eq!(client_redirect.path(), "/callback");
    assert_eq!(query_param(&client_redirect, "state"), "external-state");
    let proxy_code = query_param(&client_redirect, "code");

    // Step 3: the external client redeems the proxy-minted code, proving
    // possession of its own PKCE verifier.
    let token_req = test::TestRequest::post()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/token"))
        .set_form([
            ("grant_type", "authorization_code"),
            ("code", proxy_code.as_str()),
            ("redirect_uri", "http://localhost:12345/callback"),
            ("code_verifier", EXTERNAL_VERIFIER),
        ])
        .to_request();
    let token_resp = test::call_service(&app, token_req).await;
    assert_eq!(token_resp.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(token_resp).await;
    assert_eq!(body["access_token"], "upstream-access-token");
    assert_eq!(body["refresh_token"], "upstream-refresh-token");
    assert_eq!(body["id_token"], id_token);
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 3600);

    delete_proxy(&pool, &ns, &cluster).await;
}

async fn harness_mock_server() -> wiremock::MockServer {
    wiremock::MockServer::start().await
}
