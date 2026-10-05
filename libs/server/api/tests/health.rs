//! Liveness (`/management/health`) and readiness (`/management/ready`) probes.

mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::base::{health, ready};
use harness::{test_state, try_redis_pool, unreachable_state};

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
}
