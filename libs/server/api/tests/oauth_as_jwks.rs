//! Fast-tier integration tests for `GET /{ns}/{cluster}/oauth/jwks`, the
//! stateless passthrough of the upstream provider's JWKS document.

mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::auth::oauth::jwks;
use harness::{
    delete_proxy, mount_full_oidc_provider, oidc_auth_config, oidc_auth_config_with_well_known,
    proxy_fixture, seed_proxy, test_jwks, test_state, try_redis_pool, unique_cluster,
};
use wiremock::MockServer;

macro_rules! jwks_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new()
                .app_data(web::Data::new($state))
                .service(web::scope("/clusters").service(jwks::jwks)),
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

#[actix_web::test]
async fn mirrors_the_upstream_jwks_document() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = jwks_app!(test_state(idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/jwks"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body, test_jwks());

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn returns_404_when_discovery_is_not_enabled() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    // OIDC enabled, but the well-known flag is off.
    proxy.spec.auth_config = Some(oidc_auth_config(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = jwks_app!(test_state(idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/jwks"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn returns_404_when_the_proxy_is_disabled() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_full_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    proxy.spec.enabled = false;
    seed_proxy(&pool, &proxy).await;

    let app = jwks_app!(test_state(idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/jwks"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn returns_404_for_an_unknown_cluster() {
    let _pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();

    let app = jwks_app!(test_state("https://issuer.example.com".to_string()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/jwks"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn returns_503_when_the_upstream_provider_is_unreachable() {
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    // Nothing listens here: the discovery fetch itself must fail.
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known("http://127.0.0.1:1"));
    seed_proxy(&pool, &proxy).await;

    let app = jwks_app!(test_state("http://127.0.0.1:1".to_string()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/oauth/jwks"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    delete_proxy(&pool, &ns, &cluster).await;
}
