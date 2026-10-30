//! Fast-tier integration tests for the well-known OAuth authorization server
//! discovery endpoint (`GET /{ns}/{cluster}/.well-known/oauth-authorization-server`).

mod harness;

use harness::{memory_state, memory_store};

use axum::http::StatusCode;
use harness::test;
use harness::{
    delete_proxy, mount_oidc_provider, oidc_auth_config, oidc_auth_config_with_well_known,
    proxy_fixture, seed_proxy, unique_cluster,
};
use wiremock::MockServer;

macro_rules! well_known_app {
    ($state:expr_2021) => {
        test::init_service(harness::app($state)).await
    };
}

#[tokio::test]
async fn serves_metadata_mirroring_the_upstream_provider_when_enabled() {
    let pool = memory_store();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let state = memory_state(&pool, idp.uri());
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
    assert_eq!(
        body["response_types_supported"],
        serde_json::json!(["code"])
    );
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

#[tokio::test]
async fn returns_404_when_discovery_is_not_enabled() {
    let pool = memory_store();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    // OIDC enabled, but the well-known flag is off.
    proxy.spec.auth_config = Some(oidc_auth_config(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = well_known_app!(memory_state(&pool, idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/.well-known/oauth-authorization-server"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[tokio::test]
async fn returns_404_when_the_proxy_is_disabled() {
    let pool = memory_store();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    proxy.spec.enabled = false;
    seed_proxy(&pool, &proxy).await;

    let app = well_known_app!(memory_state(&pool, idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/.well-known/oauth-authorization-server"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[tokio::test]
async fn returns_404_for_an_unknown_cluster() {
    let pool = memory_store();
    let (ns, cluster) = unique_cluster();

    let app = well_known_app!(memory_state(
        &pool,
        "https://issuer.example.com".to_string()
    ));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/.well-known/oauth-authorization-server"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
