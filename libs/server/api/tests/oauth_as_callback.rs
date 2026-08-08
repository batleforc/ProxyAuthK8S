//! Fast-tier integration tests for `GET /{ns}/{cluster}/oauth/callback`'s
//! failure paths.
//!
//! The happy path (exchanging a real upstream code and verifying a signed ID
//! token) lives in `oidc_login_success.rs`, which needs the full
//! `authorize` -> `callback` -> `token` chain to set up realistic pending
//! state; `oauth_as_token.rs` covers what happens downstream of a successful
//! exchange (seeded directly as an `IssuedCode`). This file drives
//! `authorize` for real (to get genuine pending state) and then breaks the
//! upstream exchange in specific ways to hit `callback`'s own error branches.

mod harness;

use actix_web::{http::StatusCode, test, web, App};
use api::cluster::auth::oauth::{authorize, callback};
use harness::{
    delete_proxy, mount_full_oidc_provider, mount_token_endpoint, oidc_auth_config,
    oidc_auth_config_with_well_known, proxy_fixture, seed_proxy, sign_id_token, test_state,
    try_redis_pool, unique_cluster,
};
use reqwest::Url;
use wiremock::MockServer;

const EXTERNAL_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

macro_rules! callback_app {
    ($state:expr) => {
        test::init_service(
            App::new().app_data(web::Data::new($state)).service(
                web::scope("/clusters")
                    .service(authorize::authorize)
                    .service(callback::callback),
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

fn location(resp: &actix_web::dev::ServiceResponse) -> Url {
    let raw = resp
        .headers()
        .get("location")
        .expect("a redirect must carry a Location header")
        .to_str()
        .expect("Location must be ASCII");
    Url::parse(raw).expect("Location must be a valid URL")
}

fn query_param(url: &Url, key: &str) -> String {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("{key} should be present in {url}"))
        .1
        .into_owned()
}

macro_rules! authorize_query {
    () => {
        format!(
            "?response_type=code&client_id=my-cli\
             &redirect_uri=http%3A%2F%2Flocalhost%3A12345%2Fcallback&state=external-state\
             &code_challenge={EXTERNAL_CHALLENGE}&code_challenge_method=S256"
        )
    };
}

#[actix_web::test]
async fn rejects_an_unknown_state() {
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known("https://issuer.example.com"));
    seed_proxy(&pool, &proxy).await;

    let app = callback_app!(test_state("https://issuer.example.com".to_string()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/callback?code=upstream-code&state=never-issued"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn returns_404_when_discovery_is_not_enabled() {
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config("https://issuer.example.com"));
    seed_proxy(&pool, &proxy).await;

    let app = callback_app!(test_state("https://issuer.example.com".to_string()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/callback?code=upstream-code&state=whatever"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn redirects_with_access_denied_when_the_upstream_token_exchange_fails() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;
    // Deliberately no `/token` mock: the upstream exchange itself must fail.

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = callback_app!(test_state(idp.uri()));
    let authorize_req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/authorize{}",
            authorize_query!()
        ))
        .to_request();
    let authorize_resp = test::call_service(&app, authorize_req).await;
    assert_eq!(authorize_resp.status(), StatusCode::FOUND);
    let upstream_auth_url = location(&authorize_resp);
    let correlation_id = query_param(&upstream_auth_url, "state");

    let callback_req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/callback?code=upstream-code&state={correlation_id}"
        ))
        .to_request();
    let callback_resp = test::call_service(&app, callback_req).await;

    assert_eq!(callback_resp.status(), StatusCode::FOUND);
    let redirect = location(&callback_resp);
    assert_eq!(redirect.origin().unicode_serialization(), "http://localhost:12345");
    assert_eq!(query_param(&redirect, "error"), "access_denied");
    assert_eq!(query_param(&redirect, "state"), "external-state");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn redirects_with_a_server_error_when_the_id_token_is_missing() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;
    {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        // A token response with no `id_token` at all.
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "upstream-access-token",
                "token_type": "Bearer",
                "expires_in": 3600,
            })))
            .mount(&idp)
            .await;
    }

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = callback_app!(test_state(idp.uri()));
    let authorize_req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/authorize{}",
            authorize_query!()
        ))
        .to_request();
    let authorize_resp = test::call_service(&app, authorize_req).await;
    let upstream_auth_url = location(&authorize_resp);
    let correlation_id = query_param(&upstream_auth_url, "state");

    let callback_req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/callback?code=upstream-code&state={correlation_id}"
        ))
        .to_request();
    let callback_resp = test::call_service(&app, callback_req).await;

    assert_eq!(callback_resp.status(), StatusCode::FOUND);
    let redirect = location(&callback_resp);
    assert_eq!(query_param(&redirect, "error"), "server_error");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn redirects_with_a_server_error_when_the_id_token_nonce_is_wrong() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = callback_app!(test_state(idp.uri()));
    let authorize_req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/authorize{}",
            authorize_query!()
        ))
        .to_request();
    let authorize_resp = test::call_service(&app, authorize_req).await;
    let upstream_auth_url = location(&authorize_resp);
    let correlation_id = query_param(&upstream_auth_url, "state");

    // Signed with a nonce that does not match the one `/oauth/authorize` sent upstream.
    let id_token = sign_id_token(&idp.uri(), "proxyauthk8s", "alice-sub", "wrong-nonce");
    mount_token_endpoint(&idp, &id_token).await;

    let callback_req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/callback?code=upstream-code&state={correlation_id}"
        ))
        .to_request();
    let callback_resp = test::call_service(&app, callback_req).await;

    assert_eq!(callback_resp.status(), StatusCode::FOUND);
    let redirect = location(&callback_resp);
    assert_eq!(query_param(&redirect, "error"), "server_error");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn state_is_single_use_and_cannot_be_replayed() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = callback_app!(test_state(idp.uri()));
    let authorize_req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/authorize{}",
            authorize_query!()
        ))
        .to_request();
    let authorize_resp = test::call_service(&app, authorize_req).await;
    let upstream_auth_url = location(&authorize_resp);
    let nonce = query_param(&upstream_auth_url, "nonce");
    let correlation_id = query_param(&upstream_auth_url, "state");

    let id_token = sign_id_token(&idp.uri(), "proxyauthk8s", "alice-sub", &nonce);
    mount_token_endpoint(&idp, &id_token).await;

    let callback_uri = format!(
        "/clusters/{ns}/{cluster}/oauth/callback?code=upstream-code&state={correlation_id}"
    );
    let first = test::call_service(&app, test::TestRequest::get().uri(&callback_uri).to_request()).await;
    assert_eq!(first.status(), StatusCode::FOUND);

    let second = test::call_service(&app, test::TestRequest::get().uri(&callback_uri).to_request()).await;
    assert_eq!(second.status(), StatusCode::BAD_REQUEST);

    delete_proxy(&pool, &ns, &cluster).await;
}
