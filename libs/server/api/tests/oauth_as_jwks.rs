//! Fast-tier integration tests for `GET /{ns}/{cluster}/oauth/jwks`, the
//! stateless passthrough of the upstream provider's JWKS document.

mod harness;

use harness::{memory_state, memory_store};

use axum::http::StatusCode;
use harness::test;
use harness::{
    delete_proxy, mount_full_oidc_provider, oidc_auth_config, oidc_auth_config_with_well_known,
    proxy_fixture, seed_proxy, test_jwks, unique_cluster,
};
use wiremock::MockServer;

macro_rules! jwks_app {
    ($state:expr_2021) => {
        test::init_service(harness::app($state)).await
    };
}

#[tokio::test]
async fn mirrors_the_upstream_jwks_document() {
    let pool = memory_store();
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = jwks_app!(memory_state(&pool, idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/jwks"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body, test_jwks());

    delete_proxy(&pool, &ns, &cluster).await;
}

#[tokio::test]
async fn returns_404_when_discovery_is_not_enabled() {
    let pool = memory_store();
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    // OIDC enabled, but the well-known flag is off.
    proxy.spec.auth_config = Some(oidc_auth_config(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = jwks_app!(memory_state(&pool, idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/jwks"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[tokio::test]
async fn returns_404_when_the_proxy_is_disabled() {
    let pool = memory_store();
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    proxy.spec.enabled = false;
    seed_proxy(&pool, &proxy).await;

    let app = jwks_app!(memory_state(&pool, idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/jwks"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[tokio::test]
async fn returns_404_for_an_unknown_cluster() {
    let pool = memory_store();
    let (ns, cluster) = unique_cluster();

    let app = jwks_app!(memory_state(
        &pool,
        "https://issuer.example.com".to_string()
    ));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/jwks"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn returns_503_when_the_upstream_provider_is_unreachable() {
    let pool = memory_store();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    // Nothing listens here: the discovery fetch itself must fail.
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known("http://127.0.0.1:1"));
    seed_proxy(&pool, &proxy).await;

    let app = jwks_app!(memory_state(&pool, "http://127.0.0.1:1".to_string()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/jwks"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    delete_proxy(&pool, &ns, &cluster).await;
}
