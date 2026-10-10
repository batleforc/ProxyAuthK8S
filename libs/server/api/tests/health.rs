//! Liveness (`/management/health`) and readiness (`/management/ready`) probes.

mod harness;

use std::sync::atomic::Ordering;
use std::time::Duration;

use actix_web::{App, http::StatusCode, test, web};
use api::base::{health, ready};
use harness::{mount_full_oidc_provider, test_state, try_redis_pool, unreachable_state};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

macro_rules! probe_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new()
                .app_data(web::Data::new($state))
                .service(web::scope("/management").service(health).service(ready)),
        )
        .await
    };
}

#[actix_web::test]
async fn liveness_ignores_redis() {
    let app = probe_app!(unreachable_state("http://127.0.0.1:1".to_string()));
    let res = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/management/health")
            .to_request(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[actix_web::test]
async fn readiness_is_503_when_redis_is_down() {
    let app = probe_app!(unreachable_state("http://127.0.0.1:1".to_string()));
    let res = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/management/ready")
            .to_request(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = test::read_body_json(res).await;
    assert_eq!(body["redis"], "unavailable");
    assert_eq!(body["oidc"], "ok");
}

#[actix_web::test]
async fn readiness_is_200_when_redis_answers() {
    if try_redis_pool().await.is_none() {
        return;
    }
    let app = probe_app!(test_state("http://127.0.0.1:1".to_string()));
    let res = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/management/ready")
            .to_request(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(res).await;
    assert_eq!(body["redis"], "ok");
    assert_eq!(body["oidc"], "ok");
}

#[actix_web::test]
async fn readiness_is_503_while_oidc_discovery_is_pending() {
    // Redis up or not, a pod whose boot-time discovery has not succeeded yet
    // must not receive traffic.
    let state = test_state("http://127.0.0.1:1".to_string());
    state.oidc_ready.store(false, Ordering::Release);
    let app = probe_app!(state);
    let res = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/management/ready")
            .to_request(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = test::read_body_json(res).await;
    assert_eq!(body["oidc"], "pending");
}

#[actix_web::test]
async fn liveness_stays_ok_while_oidc_discovery_is_pending() {
    let state = unreachable_state("http://127.0.0.1:1".to_string());
    state.oidc_ready.store(false, Ordering::Release);
    let app = probe_app!(state);
    let res = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/management/health")
            .to_request(),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn oidc_discovery_retries_until_the_idp_is_back() {
    let idp = MockServer::start().await;
    // The IdP is down for the first two attempts, then serves discovery.
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .with_priority(1)
        .expect(2)
        .mount(&idp)
        .await;
    mount_full_oidc_provider(&idp, "alice", &[]).await;

    let state = unreachable_state(idp.uri());
    state.oidc_ready.store(false, Ordering::Release);
    tokio::time::timeout(
        Duration::from_secs(10),
        state.discover_oidc_until_ready(Duration::from_millis(10), Duration::from_millis(50)),
    )
    .await
    .expect("discovery should succeed once the IdP answers");
    assert!(state.is_oidc_ready());
}

#[tokio::test]
async fn oidc_discovery_keeps_retrying_while_the_idp_is_down() {
    let idp = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&idp)
        .await;

    let state = unreachable_state(idp.uri());
    state.oidc_ready.store(false, Ordering::Release);
    let outcome = tokio::time::timeout(
        Duration::from_millis(300),
        state.discover_oidc_until_ready(Duration::from_millis(10), Duration::from_millis(20)),
    )
    .await;
    assert!(outcome.is_err(), "discovery must not give up on its own");
    assert!(!state.is_oidc_ready());
    let attempts = idp.received_requests().await.unwrap_or_default().len();
    assert!(attempts >= 3, "expected several attempts, got {attempts}");
}
