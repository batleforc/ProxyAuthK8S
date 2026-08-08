//! Fast-tier integration tests for the well-known OAuth authorization server
//! discovery endpoint (`GET /{ns}/{cluster}/.well-known/oauth-authorization-server`).

mod harness;

use actix_web::{http::StatusCode, test, web, App};
use api::cluster::auth::well_known;
use harness::{
    delete_proxy, mount_oidc_provider, oidc_auth_config, oidc_auth_config_with_well_known,
    proxy_fixture, seed_proxy, test_state, try_redis_pool, unique_cluster,
};
use wiremock::MockServer;

macro_rules! well_known_app {
    ($state:expr) => {
        test::init_service(
            App::new()
                .app_data(web::Data::new($state))
                .service(web::scope("/clusters").service(well_known::oauth_authorization_server)),
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
async fn serves_metadata_mirroring_the_upstream_provider_when_enabled() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let state = test_state(idp.uri());
    let app = well_known_app!(state);
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/.well-known/oauth-authorization-server"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    let issuer = format!("https://proxy.example.com/clusters/{ns}/{cluster}");
    assert_eq!(body["issuer"].as_str(), Some(issuer.as_str()));
    assert_eq!(
        body["authorization_endpoint"].as_str(),
        Some(format!("{issuer}/oauth/authorize")).as_deref()
    );
    assert_eq!(
        body["token_endpoint"].as_str(),
        Some(format!("{issuer}/oauth/token")).as_deref()
    );
    assert_eq!(
        body["jwks_uri"].as_str(),
        Some(format!("{issuer}/oauth/jwks")).as_deref()
    );
    assert_eq!(body["response_types_supported"], serde_json::json!(["code"]));
    assert_eq!(
        body["grant_types_supported"],
        serde_json::json!(["authorization_code"])
    );
    assert_eq!(
        body["code_challenge_methods_supported"],
        serde_json::json!(["S256"])
    );

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn returns_404_when_discovery_is_not_enabled() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    // OIDC enabled, but the well-known flag is off.
    proxy.spec.auth_config = Some(oidc_auth_config(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = well_known_app!(test_state(idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/.well-known/oauth-authorization-server"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn returns_404_when_the_proxy_is_disabled() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    proxy.spec.enabled = false;
    seed_proxy(&pool, &proxy).await;

    let app = well_known_app!(test_state(idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/.well-known/oauth-authorization-server"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn returns_404_for_an_unknown_cluster() {
    let _pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();

    let app = well_known_app!(test_state("https://issuer.example.com".to_string()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/.well-known/oauth-authorization-server"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
