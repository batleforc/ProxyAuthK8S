//! Fast-tier integration tests for the cluster proxy path.
//!
//! The upstream Kubernetes API is a `wiremock` server; Redis is real (see
//! `harness`). Upgrade (exec/attach) is not covered here because it needs a raw
//! HTTP/1.1 101 exchange on a bound socket.
//!
//! That exchange has no integration coverage anywhere — the envtest tier does
//! not touch it either, despite what this comment used to claim. What is covered
//! are the two functions on that path which encode a security contract rather
//! than an I/O shape: the request-smuggling guard and the request serializer,
//! unit-tested in `kube_redirect/upgrade.rs` itself.

mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::redirect;
use harness::{
    delete_proxy, proxy_fixture, seed_proxy, test_state, try_redis_pool, unique_cluster,
    unreachable_state,
};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

macro_rules! proxy_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new().app_data(web::Data::new($state)).service(
                web::scope("/clusters")
                    .service(redirect::get_redirect)
                    .service(redirect::post_redirect)
                    .service(redirect::put_redirect)
                    .service(redirect::patch_redirect)
                    .service(redirect::delete_redirect),
            ),
        )
        .await
    };
}

/// Bind a test Redis, or bail out of the test (see `harness::try_redis_pool`).
macro_rules! redis_or_skip {
    () => {
        match try_redis_pool().await {
            Some(pool) => pool,
            None => return,
        }
    };
}

#[actix_web::test]
async fn forwards_a_request_to_the_upstream_cluster() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/dev/pods"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"kind":"PodList","items":[]}"#)
                .insert_header("content-type", "application/json"),
        )
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream.uri())).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        test::read_body(resp).await,
        r#"{"kind":"PodList","items":[]}"#
    );

    delete_proxy(&pool, &ns, &cluster).await;
}

/// Regression for the `.replace()` rewrite: only the routing prefix may be
/// stripped, never an identical substring further down the path.
#[actix_web::test]
async fn strips_only_the_routing_prefix() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    let (ns, cluster) = unique_cluster();
    // The upstream path deliberately repeats the routing prefix.
    let repeated = format!("/api/v1/namespaces/dev/configmaps/clusters/{ns}/{cluster}");

    Mock::given(method("GET"))
        .and(path(repeated.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&upstream)
        .await;

    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream.uri())).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}{repeated}"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(test::read_body(resp).await, "ok");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn forwards_query_string_and_headers() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .and(query_param("labelSelector", "app=web"))
        .and(header("authorization", "Bearer some-token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("filtered"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream.uri())).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/pods?labelSelector=app%3Dweb"
        ))
        .insert_header(("authorization", "Bearer some-token"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(test::read_body(resp).await, "filtered");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn streams_a_watch_response_without_compressing_it() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    let events = "{\"type\":\"ADDED\"}\n{\"type\":\"MODIFIED\"}\n";
    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .and(query_param("watch", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_string(events))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream.uri())).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/pods?watch=true&timeout=32s"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    // `identity` is what keeps actix' Compress middleware from buffering the stream.
    assert_eq!(
        resp.headers()
            .get("content-encoding")
            .and_then(|v| v.to_str().ok()),
        Some("identity")
    );
    assert_eq!(test::read_body(resp).await, events);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn propagates_the_upstream_status_code() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/api/v1/namespaces/dev/pods/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_string(r#"{"reason":"NotFound"}"#))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream.uri())).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::delete()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods/missing"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(test::read_body(resp).await, r#"{"reason":"NotFound"}"#);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn forwards_a_request_body() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/namespaces/dev/pods"))
        .and(wiremock::matchers::body_string(r#"{"kind":"Pod"}"#))
        .respond_with(ResponseTemplate::new(201).set_body_string("created"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream.uri())).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::post()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods"
        ))
        .set_payload(r#"{"kind":"Pod"}"#)
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::CREATED);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn unknown_cluster_returns_404() {
    let _pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    let (ns, cluster) = unique_cluster();

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn disabled_cluster_returns_404() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.enabled = false;
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn missing_token_returns_401_when_validation_is_required() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(harness::oidc_auth_config(&upstream.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    // The upstream must never have been contacted.
    assert!(
        upstream
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn invalid_token_returns_401_when_validation_is_required() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    // No OIDC discovery document is mounted, so token validation cannot succeed.
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(harness::oidc_auth_config(&upstream.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", "Bearer not-a-valid-token"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn redis_outage_returns_503() {
    let upstream = MockServer::start().await;
    let (ns, cluster) = unique_cluster();

    let app = proxy_app!(unreachable_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}
