//! Lot 6.4: rate limiting and fail2login, against a real Redis.
//!
//! The counters are shared state, so these exercise the Redis plumbing; the
//! policy itself (which limit applies, how long a ban lasts) is unit-tested on
//! `SecurityConfiguration`.

mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::redirect;
use harness::{
    delete_proxy, fail2login_config, mount_oidc_provider, oidc_auth_config, proxy_fixture,
    rate_limited_config, seed_proxy, test_state, try_redis_pool, unique_cluster,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A JWT-shaped access token whose `aud` claim names `oidc_auth_config`'s
/// `client_id` (`proxyauthk8s`) — needed so `ensure_token_audience` (which
/// fails closed under the default Enforce mode on an opaque token) doesn't
/// reject it before it can count as a "valid" authentication. Signature is
/// not verified here.
const VALID_TOKEN: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJwcm94eWF1dGhrOHMiLCJzdWIiOiJhbGljZS1zdWIifQ.c2lnbmF0dXJlLW5vdC12ZXJpZmllZC1pbi10aGVzZS10ZXN0cw";

macro_rules! proxy_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new()
                .app_data(web::Data::new($state))
                .service(web::scope("/clusters").service(redirect::get_redirect)),
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
async fn refuses_requests_past_the_rate_limit() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.security_config = Some(rate_limited_config(3));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let uri = format!("/clusters/{ns}/{cluster}/api/v1/pods");

    for attempt in 1..=3 {
        let req = test::TestRequest::get().uri(&uri).to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "request {attempt} should pass"
        );
    }

    let req = test::TestRequest::get().uri(&uri).to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        resp.headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("60")
    );

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["kind"], "Status");
    assert_eq!(body["reason"], "TooManyRequests");
    assert_eq!(body["code"], 429);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn a_cluster_without_rate_limiting_is_never_throttled() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream.uri())).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let uri = format!("/clusters/{ns}/{cluster}/api/v1/pods");

    for _ in 0..10 {
        let req = test::TestRequest::get().uri(&uri).to_request();
        assert_eq!(test::call_service(&app, req).await.status(), StatusCode::OK);
    }

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn bans_a_client_after_repeated_authentication_failures() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    mount_oidc_provider(&upstream, "alice", &["dev"]).await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    proxy.spec.security_config = Some(fail2login_config(3, 300));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let uri = format!("/clusters/{ns}/{cluster}/api/v1/pods");

    // No token at all: three failures, then the ban.
    for attempt in 1..=3 {
        let req = test::TestRequest::get().uri(&uri).to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "failure {attempt} should be a 401"
        );
    }

    let req = test::TestRequest::get().uri(&uri).to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    // Even a valid token is refused while the ban is in force.
    let req = test::TestRequest::get()
        .uri(&uri)
        .insert_header(("authorization", format!("Bearer {VALID_TOKEN}")))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    // Clean up the counters this test left behind.
    let mut conn = pool.get().await.expect("redis connection");
    use deadpool_redis::redis::AsyncTypedCommands;
    let _ = conn
        .del(format!("proxyk8sauth:ban:{ns}/{cluster}:unknown"))
        .await;
    let _ = conn
        .del(format!("proxyk8sauth:fail2login:{ns}/{cluster}:unknown"))
        .await;

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn a_successful_authentication_clears_the_failure_counter() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    mount_oidc_provider(&upstream, "alice", &["dev"]).await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    proxy.spec.security_config = Some(fail2login_config(3, 300));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let uri = format!("/clusters/{ns}/{cluster}/api/v1/pods");

    // Two failures, then a success, then two more failures: without the reset
    // the fifth request would be banned.
    for _ in 0..2 {
        let req = test::TestRequest::get().uri(&uri).to_request();
        assert_eq!(
            test::call_service(&app, req).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    let req = test::TestRequest::get()
        .uri(&uri)
        .insert_header(("authorization", format!("Bearer {VALID_TOKEN}")))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), StatusCode::OK);

    for _ in 0..2 {
        let req = test::TestRequest::get().uri(&uri).to_request();
        assert_eq!(
            test::call_service(&app, req).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    let req = test::TestRequest::get()
        .uri(&uri)
        .insert_header(("authorization", format!("Bearer {VALID_TOKEN}")))
        .to_request();
    assert_eq!(
        test::call_service(&app, req).await.status(),
        StatusCode::OK,
        "the counter should have been reset by the successful authentication"
    );

    let mut conn = pool.get().await.expect("redis connection");
    use deadpool_redis::redis::AsyncTypedCommands;
    let _ = conn
        .del(format!("proxyk8sauth:fail2login:{ns}/{cluster}:unknown"))
        .await;

    delete_proxy(&pool, &ns, &cluster).await;
}
