//! How a virtual API interacts with the resource allow-list.
//!
//! A rewritten request reaches a different upstream path than the one the
//! client asked for, so both are checked: allowing the virtual path must not
//! become a way around the allow-list on the API it maps to.

mod harness;
mod virtual_api_support;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::redirect;
use crd::virtual_api::VirtualApiKind;
use harness::{
    delete_proxy, proxy_fixture, security_config, seed_proxy, test_state, try_redis_pool,
    unique_cluster, with_virtual_api,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

macro_rules! proxy_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new().app_data(web::Data::new($state)).service(
                web::scope("/clusters")
                    .service(redirect::get_redirect)
                    .service(redirect::post_redirect)
                    .service(redirect::delete_redirect),
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
async fn a_cluster_without_virtual_apis_proxies_the_path_verbatim() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    // No mapper enabled: the path goes upstream as typed and 404s there.
    Mock::given(method("GET"))
        .and(path("/apis/project.openshift.io/v1/projects"))
        .respond_with(ResponseTemplate::new(404).set_body_string("no such api"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream.uri())).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projects"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(test::read_body(resp).await, "no such api");

    delete_proxy(&pool, &ns, &cluster).await;
}

/// A virtual path must not become a way around the resource allow-list.
#[actix_web::test]
async fn the_allow_list_also_covers_the_mapped_upstream_path() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("should not be reached"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    with_virtual_api(&mut proxy, VirtualApiKind::OpenShiftProject);
    // The virtual path is allowed, the namespace collection it maps to is not.
    proxy.spec.security_config = Some(security_config(vec![(
        "/apis/project.openshift.io/v1/projects",
        false,
    )]));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projects"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
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
async fn an_allow_list_covering_both_paths_lets_the_request_through() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "NamespaceList",
            "apiVersion": "v1",
            "items": [],
        })))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    with_virtual_api(&mut proxy, VirtualApiKind::OpenShiftProject);
    proxy.spec.security_config = Some(security_config(vec![
        ("/apis/project.openshift.io/v1/projects", false),
        ("/api/v1/namespaces", false),
    ]));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projects"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["kind"], "ProjectList");

    delete_proxy(&pool, &ns, &cluster).await;
}
