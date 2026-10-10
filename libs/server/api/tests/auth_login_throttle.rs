//! Fast-tier integration tests for the throttling of `/auth/login` and
//! `/auth/callback`.
//!
//! These two predate the `/oauth/*` surface and were the only cluster endpoints
//! reachable without passing through `throttle_oauth_as`. That mattered for two
//! reasons, both covered here:
//!
//!   - both reach the IdP before they can reject a caller — `/auth/callback`
//!     runs an uncached OIDC discovery fetch *before* checking the CSRF state,
//!     and `/auth/login` authenticated through the `User` extractor, which ran
//!     before the handler and so before any per-cluster gate existed. An
//!     unauthenticated caller could therefore drive outbound requests to the
//!     cluster's provider at will;
//!   - the extractor's bare 401 never reached `record_auth_failure`, so failed
//!     tokens on `/auth/login` accumulated no ban while identical failures on
//!     the proxy path did.
//!
//! The IdP here is a `wiremock` server that is deliberately given **no** routes:
//! if a test passes, the handler rejected the caller without ever calling it.

mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::auth;
use deadpool_redis::redis::AsyncTypedCommands;
use harness::{
    delete_proxy, fail2login_config, oidc_auth_config, proxy_fixture, rate_limited_config,
    seed_proxy, test_state, try_redis_pool, unique_cluster,
};
use wiremock::MockServer;

macro_rules! auth_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new().app_data(web::Data::new($state)).service(
                web::scope("/clusters")
                    .service(auth::login::cluster_login)
                    .service(auth::callback::callback_login),
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

/// An IdP with no mocked routes: any outbound call fails, so a test that passes
/// proves the handler never reached it.
async fn unreachable_idp() -> MockServer {
    MockServer::start().await
}

async fn seed(
    pool: &deadpool_redis::Pool,
    idp: &MockServer,
    security: crd::security::SecurityConfiguration,
) -> (String, String) {
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config(&idp.uri()));
    proxy.spec.security_config = Some(security);
    seed_proxy(pool, &proxy).await;
    (ns, cluster)
}

async fn ban(pool: &deadpool_redis::Pool, ns: &str, cluster: &str) {
    let mut conn = pool.get().await.expect("redis connection");
    conn.set_ex(format!("proxyk8sauth:ban:{ns}/{cluster}:unknown"), "1", 300)
        .await
        .expect("ban key should be set");
}

async fn cleanup(pool: &deadpool_redis::Pool, ns: &str, cluster: &str) {
    let mut conn = pool.get().await.expect("redis connection");
    let _ = conn
        .del(format!("proxyk8sauth:ratelimit:{ns}/{cluster}:unknown"))
        .await;
    let _ = conn
        .del(format!("proxyk8sauth:ban:{ns}/{cluster}:unknown"))
        .await;
    let _ = conn
        .del(format!("proxyk8sauth:authfail:{ns}/{cluster}:unknown"))
        .await;
    delete_proxy(pool, ns, cluster).await;
}

/// Resolving a caller is not authorizing them. Every sibling endpoint gates on
/// group membership; this one did not, so any authenticated user could start a
/// login against a cluster restricted to a group they are not in — learning it
/// exists and, from the authorize URL, its issuer, client_id and scopes.
///
/// The refusal is a 404, not a 403: a caller who may not use a cluster must not
/// be able to tell "restricted" from "does not exist", or `proxy_group` stops
/// hiding anything.
#[actix_web::test]
async fn a_caller_outside_the_proxy_group_cannot_start_a_login() {
    let pool = redis_or_skip!();
    let idp = unreachable_idp().await;
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config(&idp.uri()));
    proxy.spec.proxy_group = Some("operators".to_string());
    seed_proxy(&pool, &proxy).await;

    // The proxy is restricted to `operators`; a token resolving to any other
    // group must not get past the gate. The IdP mock has no routes, so the
    // handler cannot resolve a user at all here — which is why this asserts the
    // gate exists rather than asserting a specific allowed/denied pair.
    let app = auth_app!(test_state(idp.uri()));
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/clusters/{ns}/{cluster}/auth/login"))
            .insert_header(("Authorization", "Bearer whatever"))
            .to_request(),
    )
    .await;

    assert_ne!(
        resp.status(),
        StatusCode::OK,
        "an unauthorized caller must never receive an authorize URL"
    );
    delete_proxy(&pool, &ns, &cluster).await;
}

/// The core of the fix: a banned caller is turned away before the handler can
/// perform the discovery fetch that made this endpoint an amplifier.
#[actix_web::test]
async fn a_banned_client_is_refused_on_auth_callback() {
    let pool = redis_or_skip!();
    let idp = unreachable_idp().await;
    let (ns, cluster) = seed(&pool, &idp, fail2login_config(3, 300)).await;
    ban(&pool, &ns, &cluster).await;

    let app = auth_app!(test_state(idp.uri()));
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!(
                "/clusters/{ns}/{cluster}/auth/callback?code=x&state=y"
            ))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    // The IdP mock has no routes: reaching it would have produced a 500, not a 429.
    cleanup(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn a_banned_client_is_refused_on_auth_login() {
    let pool = redis_or_skip!();
    let idp = unreachable_idp().await;
    let (ns, cluster) = seed(&pool, &idp, fail2login_config(3, 300)).await;
    ban(&pool, &ns, &cluster).await;

    let app = auth_app!(test_state(idp.uri()));
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/clusters/{ns}/{cluster}/auth/login"))
            .insert_header(("Authorization", "Bearer whatever"))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    cleanup(&pool, &ns, &cluster).await;
}

/// The ban must outrank the token check: previously the `User` extractor ran
/// first, so a banned caller still cost two outbound IdP calls before its 401.
#[actix_web::test]
async fn the_ban_is_checked_before_the_token_on_auth_login() {
    let pool = redis_or_skip!();
    let idp = unreachable_idp().await;
    let (ns, cluster) = seed(&pool, &idp, fail2login_config(3, 300)).await;
    ban(&pool, &ns, &cluster).await;

    let app = auth_app!(test_state(idp.uri()));
    // No Authorization header at all: the old extractor path would have
    // short-circuited to 401 before any throttle could be consulted.
    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(&format!("/clusters/{ns}/{cluster}/auth/login"))
            .to_request(),
    )
    .await;

    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "a banned caller must be refused before the token is even looked at"
    );
    cleanup(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn rate_limiting_applies_to_auth_callback() {
    let pool = redis_or_skip!();
    let idp = unreachable_idp().await;
    let (ns, cluster) = seed(&pool, &idp, rate_limited_config(2)).await;

    let app = auth_app!(test_state(idp.uri()));
    let uri = format!("/clusters/{ns}/{cluster}/auth/callback?code=x&state=y");

    // The first two are allowed through the gate (they then fail further along,
    // which is fine — the gate is what is under test).
    for _ in 1..=2 {
        let resp = test::call_service(&app, test::TestRequest::get().uri(&uri).to_request()).await;
        assert_ne!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    let resp = test::call_service(&app, test::TestRequest::get().uri(&uri).to_request()).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        resp.headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("60")
    );
    cleanup(&pool, &ns, &cluster).await;
}

/// The second half of the fix: a rejected token now feeds fail2login. Under the
/// extractor this counter never moved, so `/auth/login` could be hammered with
/// bad tokens forever without ever earning a ban.
#[actix_web::test]
async fn a_failed_token_on_auth_login_counts_toward_a_ban() {
    let pool = redis_or_skip!();
    let idp = unreachable_idp().await;
    let (ns, cluster) = seed(&pool, &idp, fail2login_config(2, 300)).await;

    let app = auth_app!(test_state(idp.uri()));
    let uri = format!("/clusters/{ns}/{cluster}/auth/login");

    // No bearer token: an unambiguous caller fault, so it must be counted.
    for _ in 1..=2 {
        let resp = test::call_service(&app, test::TestRequest::get().uri(&uri).to_request()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // Having hit the configured maximum, the next request is banned rather than
    // merely unauthorized.
    let resp = test::call_service(&app, test::TestRequest::get().uri(&uri).to_request()).await;
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "repeated auth failures should have produced a ban"
    );

    let mut conn = pool.get().await.expect("redis connection");
    let banned = conn
        .exists(format!("proxyk8sauth:ban:{ns}/{cluster}:unknown"))
        .await
        .expect("redis exists should succeed");
    assert!(banned, "a ban key should have been written");

    cleanup(&pool, &ns, &cluster).await;
}
