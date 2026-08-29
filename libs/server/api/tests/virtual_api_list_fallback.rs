//! `LIST projects` filtered per namespace, against a wiremock cluster.
//!
//! Covers the filtering itself: what a caller may see, an empty result, a
//! discovery failure, a namespace whose check errors, and the query string
//! being carried through to the privileged call. The `SelfSubjectRulesReview`
//! fast path in front of all this is `virtual_api_rules_review.rs`.

mod harness;
mod virtual_api_support;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::redirect;
use harness::{delete_proxy, test_state, try_redis_pool, unique_cluster};
use serde_json::{Value, json};
use virtual_api_support::{FALLBACK_TOKEN, seed_openshift_proxy_with_fallback};
use wiremock::matchers::{bearer_token, body_partial_json, method, path, query_param};
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

/// With `list_fallback_token` configured, `LIST projects` is filtered
/// unconditionally — the plain (impersonated) namespace list is never even
/// attempted, only the privileged discovery call is.
#[actix_web::test]
async fn list_fallback_filters_to_what_the_caller_can_get() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces"))
        .and(bearer_token(FALLBACK_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "NamespaceList",
            "apiVersion": "v1",
            "items": [
                { "metadata": { "name": "dev" }, "status": { "phase": "Active" } },
                { "metadata": { "name": "prod" }, "status": { "phase": "Active" } },
            ],
        })))
        .mount(&upstream)
        .await;

    Mock::given(method("POST"))
        .and(path("/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"))
        .and(body_partial_json(json!({
            "spec": { "resourceAttributes": { "name": "dev", "verb": "get", "resource": "namespaces" } }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "kind": "SelfSubjectAccessReview",
            "status": { "allowed": true },
        })))
        .mount(&upstream)
        .await;

    Mock::given(method("POST"))
        .and(path("/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"))
        .and(body_partial_json(json!({
            "spec": { "resourceAttributes": { "name": "prod", "verb": "get", "resource": "namespaces" } }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "kind": "SelfSubjectAccessReview",
            "status": { "allowed": false },
        })))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy_with_fallback(&pool, &ns, &cluster, &upstream.uri()).await;

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
    let names: Vec<&str> = body["items"]
        .as_array()
        .expect("items should be a list")
        .iter()
        .filter_map(|item| item["metadata"]["name"].as_str())
        .collect();
    assert_eq!(names, vec!["dev"]);

    // Only the privileged (bearer-token) call reached /api/v1/namespaces —
    // the plain impersonated forward was never attempted.
    let namespace_requests: Vec<_> = upstream
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|req| req.method == "GET" && req.url.path() == "/api/v1/namespaces")
        .collect();
    assert_eq!(namespace_requests.len(), 1);
    assert!(
        namespace_requests[0]
            .headers
            .get("authorization")
            .is_some_and(|value| value == &format!("Bearer {FALLBACK_TOKEN}"))
    );

    delete_proxy(&pool, &ns, &cluster).await;
}

/// The caller can `list` (via the privileged token) but has `get` on nothing:
/// a real, filtered empty list, not the raw 403 the unconfigured path returns.
#[actix_web::test]
async fn list_fallback_returns_an_empty_list_when_nothing_is_allowed() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces"))
        .and(bearer_token(FALLBACK_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "NamespaceList",
            "apiVersion": "v1",
            "items": [{ "metadata": { "name": "secret-project" } }],
        })))
        .mount(&upstream)
        .await;

    Mock::given(method("POST"))
        .and(path(
            "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews",
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "kind": "SelfSubjectAccessReview",
            "status": { "allowed": false },
        })))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy_with_fallback(&pool, &ns, &cluster, &upstream.uri()).await;

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
    assert_eq!(body["items"].as_array().map(Vec::len), Some(0));

    delete_proxy(&pool, &ns, &cluster).await;
}

/// The privileged discovery call itself failing must never widen visibility
/// (fall back to an unfiltered list) or hide it (a silent empty list) — it
/// surfaces as a `503`.
#[actix_web::test]
async fn list_fallback_discovery_failure_is_a_503() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    // No mock for /api/v1/namespaces at all: wiremock 404s, which the
    // discovery step treats as a hard failure.

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy_with_fallback(&pool, &ns, &cluster, &upstream.uri()).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projects"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    delete_proxy(&pool, &ns, &cluster).await;
}

/// One namespace's access check erroring (as opposed to a plain denial) drops
/// just that namespace, fail-closed — the rest of the list still returns.
#[actix_web::test]
async fn list_fallback_excludes_a_namespace_whose_check_errors() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces"))
        .and(bearer_token(FALLBACK_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "NamespaceList",
            "apiVersion": "v1",
            "items": [
                { "metadata": { "name": "dev" } },
                { "metadata": { "name": "prod" } },
            ],
        })))
        .mount(&upstream)
        .await;

    // Only "dev"'s review is mocked; "prod"'s request matches no stub and
    // wiremock 404s it, which `check_access` treats as an error.
    Mock::given(method("POST"))
        .and(path(
            "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews",
        ))
        .and(body_partial_json(json!({
            "spec": { "resourceAttributes": { "name": "dev" } }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "kind": "SelfSubjectAccessReview",
            "status": { "allowed": true },
        })))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy_with_fallback(&pool, &ns, &cluster, &upstream.uri()).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projects"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    let names: Vec<&str> = body["items"]
        .as_array()
        .expect("items should be a list")
        .iter()
        .filter_map(|item| item["metadata"]["name"].as_str())
        .collect();
    assert_eq!(names, vec!["dev"]);

    delete_proxy(&pool, &ns, &cluster).await;
}

/// Existing selector/pagination query params keep working through the
/// privileged discovery call.
#[actix_web::test]
async fn list_fallback_forwards_the_query_string_to_the_privileged_call() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces"))
        .and(bearer_token(FALLBACK_TOKEN))
        .and(query_param("labelSelector", "team=a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "NamespaceList",
            "apiVersion": "v1",
            "items": [{ "metadata": { "name": "dev" } }],
        })))
        .mount(&upstream)
        .await;

    Mock::given(method("POST"))
        .and(path(
            "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews",
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "kind": "SelfSubjectAccessReview",
            "status": { "allowed": true },
        })))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy_with_fallback(&pool, &ns, &cluster, &upstream.uri()).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projects?labelSelector=team%3Da"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["items"][0]["metadata"]["name"], "dev");

    delete_proxy(&pool, &ns, &cluster).await;
}
