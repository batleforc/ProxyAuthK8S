//! End-to-end integration test for the primary user journey this project
//! exists for: a user logs in to a cluster the same way the `kubectl-proxyauth`
//! extension does — `/auth/login` -> `/auth/callback`, storing the resulting
//! `id_token` as the cluster credential (see `libs/cli/cli/src/login/sso.rs`)
//! — then uses that exact token to fetch a Secret through the proxy.
//!
//! The CLI-side counterpart, which exercises the extension's own network code
//! (loopback listener, browser round-trip) against a mocked server instead of
//! this server-side login+proxy chain, is
//! `libs/cli/cli/src/login/sso.rs::tests::sso_cluster_login_completes_the_full_browser_round_trip`.
//!
//! Neither half was covered together before this test: `oidc_login_success.rs`
//! stops at token issuance, and `proxy_security.rs`/`proxy_redirect.rs`
//! exercise the proxy path with either no token or an unsigned placeholder
//! whose signature is never checked. This chains a real signed+verified login
//! all the way to an actual Kubernetes resource response.

mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::auth::{callback::callback_login, login::cluster_login};
use api::cluster::redirect;
use harness::{
    delete_proxy, mount_full_oidc_provider, mount_token_endpoint, oidc_auth_config,
    proxy_fixture, seed_proxy, sign_id_token, test_state, try_redis_pool, unique_cluster,
};
use reqwest::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A JWT-shaped access token whose `aud` claim names the front's configured
/// audience (`proxyauthk8s`) — needed for `/auth/login`'s `User` extractor,
/// which authenticates against the front OIDC client, not the per-cluster one.
const FRONT_TOKEN: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJwcm94eWF1dGhrOHMiLCJzdWIiOiJhbGljZS1zdWIifQ.c2lnbmF0dXJlLW5vdC12ZXJpZmllZC1pbi10aGVzZS10ZXN0cw";

macro_rules! full_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new().app_data(web::Data::new($state)).service(
                web::scope("/clusters")
                    .service(cluster_login)
                    .service(callback_login)
                    .service(redirect::get_redirect),
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

#[actix_web::test]
async fn a_user_logs_in_through_the_extension_flow_and_reads_a_secret() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    // The same wiremock server plays both the OIDC provider and the upstream
    // Kubernetes API, exactly like `proxy_security.rs` — their paths never
    // collide (see `harness::mount_oidc_provider`'s doc comment).
    mount_full_oidc_provider(&upstream, "alice", &["dev"]).await;

    let secret_body = serde_json::json!({
        "kind": "Secret",
        "apiVersion": "v1",
        "metadata": { "name": "db-credentials", "namespace": "dev" },
        "type": "Opaque",
        "data": { "password": "c3VwZXItc2VjcmV0" },
    });
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/dev/secrets/db-credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(secret_body.clone()))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = full_app!(test_state(upstream.uri()));

    // Step 1 (extension): start the login the same way `kubectl-proxyauth
    // login --cluster-name` does — `sso_cluster_login` calls `/auth/login`
    // authenticated with the already-stored server token.
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

    // Step 2: the browser completes the login at the real IdP; stand in for
    // it with a genuinely signed ID token, exactly like `oidc_login_success.rs`.
    let id_token = sign_id_token(&upstream.uri(), "proxyauthk8s", "alice-sub", &nonce);
    mount_token_endpoint(&upstream, &id_token).await;

    // Step 3 (extension): exchange the code for the cluster's id_token — the
    // response `sso_cluster_login` reads `.id_token` off of and stores as the
    // cluster credential (`libs/cli/cli/src/login/sso.rs`).
    let callback_req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/auth/callback?code=upstream-code&state={state}"
        ))
        .to_request();
    let callback_resp = test::call_service(&app, callback_req).await;
    assert_eq!(callback_resp.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(callback_resp).await;
    let cluster_token = body["id_token"]
        .as_str()
        .expect("the callback should return an id_token")
        .to_string();
    assert_eq!(cluster_token, id_token);

    // Step 4 (kubectl, via the exec-credential plugin's `get-token`): present
    // that exact stored token as the Bearer credential for a real request —
    // fetching a Secret's content, the scenario this whole flow exists for.
    let secret_req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/dev/secrets/db-credentials"
        ))
        .insert_header(("authorization", format!("Bearer {cluster_token}")))
        .to_request();
    let secret_resp = test::call_service(&app, secret_req).await;
    assert_eq!(secret_resp.status(), StatusCode::OK);
    let secret: serde_json::Value = test::read_body_json(secret_resp).await;
    assert_eq!(secret, secret_body);
    assert_eq!(secret["data"]["password"], "c3VwZXItc2VjcmV0");

    delete_proxy(&pool, &ns, &cluster).await;
}
