//! Fast-tier integration tests for the OAuth-AS surface's rate-limit/ban
//! protection (`crate::cluster::auth::throttle_oauth_as`), shared by every
//! well-known and `/oauth/*` endpoint. Exercised through the well-known
//! endpoint (cheapest to drive) plus one cross-check on `/oauth/jwks` to
//! confirm the same helper is actually wired into more than one handler.

mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::auth::{oauth, well_known};
use deadpool_redis::redis::AsyncTypedCommands;
use harness::{
    delete_proxy, fail2login_config, mount_oidc_provider, oidc_auth_config_with_well_known,
    proxy_fixture, rate_limited_config, seed_proxy, test_state, try_redis_pool, unique_cluster,
};
use wiremock::MockServer;

macro_rules! well_known_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new().app_data(web::Data::new($state)).service(
                web::scope("/clusters")
                    .service(well_known::oauth_authorization_server)
                    .service(oauth::jwks::jwks),
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

#[actix_web::test]
async fn rate_limit_applies_to_the_well_known_endpoint() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    proxy.spec.security_config = Some(rate_limited_config(2));
    seed_proxy(&pool, &proxy).await;

    let app = well_known_app!(test_state(idp.uri()));
    let uri = format!("/clusters/{ns}/{cluster}/.well-known/oauth-authorization-server");

    for attempt in 1..=2 {
        let resp = test::call_service(&app, test::TestRequest::get().uri(&uri).to_request()).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "request {attempt} should pass"
        );
    }

    let resp = test::call_service(&app, test::TestRequest::get().uri(&uri).to_request()).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        resp.headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("60")
    );

    let mut conn = pool.get().await.expect("redis connection");
    let _ = conn
        .del(format!("proxyk8sauth:ratelimit:{ns}/{cluster}:unknown"))
        .await;
    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn a_banned_client_is_refused_on_the_oauth_as_surface() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    proxy.spec.security_config = Some(fail2login_config(3, 300));
    seed_proxy(&pool, &proxy).await;

    // Simulate an already-banned client the way `record_auth_failure` would
    // leave one: a `proxyk8sauth:ban:...` key present with a TTL.
    let mut conn = pool.get().await.expect("redis connection");
    conn.set_ex(format!("proxyk8sauth:ban:{ns}/{cluster}:unknown"), "1", 300)
        .await
        .expect("ban key should be set");

    let app = well_known_app!(test_state(idp.uri()));

    let well_known_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!(
                "/clusters/{ns}/{cluster}/.well-known/oauth-authorization-server"
            ))
            .to_request(),
    )
    .await;
    assert_eq!(well_known_resp.status(), StatusCode::TOO_MANY_REQUESTS);

    // Same helper is wired into every OAuth-AS handler: spot-check a second one.
    let jwks_resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/clusters/{ns}/{cluster}/oauth/jwks"))
            .to_request(),
    )
    .await;
    assert_eq!(jwks_resp.status(), StatusCode::TOO_MANY_REQUESTS);

    let _ = conn
        .del(format!("proxyk8sauth:ban:{ns}/{cluster}:unknown"))
        .await;
    delete_proxy(&pool, &ns, &cluster).await;
}
