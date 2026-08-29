//! The `SelfSubjectRulesReview` fast path in front of the per-item filtering.
//!
//! One rules review resolves most or every candidate in a single call, and the
//! outcome is cached. These prove a resolved review really does replace the
//! per-item `SelfSubjectAccessReview` calls, that an `incomplete` review falls
//! back to them, and that a second request is served from the cache.

mod harness;
mod virtual_api_support;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::redirect;
use harness::{delete_proxy, test_state, try_redis_pool, unique_cluster};
use serde_json::{Value, json};
use virtual_api_support::{FALLBACK_TOKEN, seed_openshift_proxy_with_fallback};
use wiremock::matchers::{bearer_token, body_partial_json, method, path};
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

/// A `SelfSubjectRulesReview` with a `resourceNames`-scoped rule resolves
/// filtering in one call — no per-item `SelfSubjectAccessReview` at all.
#[actix_web::test]
async fn list_fallback_resolves_via_rules_review_without_any_access_review() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    // Every candidate is covered by the resolved rule below — nothing is
    // left unresolved, so nothing should need a per-item access review. A
    // candidate outside the resolved set (e.g. "prod") would correctly still
    // need one: a rules review only ever shortcuts an *allow*, never a deny.
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces"))
        .and(bearer_token(FALLBACK_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "NamespaceList",
            "apiVersion": "v1",
            "items": [
                { "metadata": { "name": "dev" } },
            ],
        })))
        .mount(&upstream)
        .await;

    Mock::given(method("POST"))
        .and(path(
            "/apis/authorization.k8s.io/v1/selfsubjectrulesreviews",
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "kind": "SelfSubjectRulesReview",
            "status": {
                "incomplete": false,
                "resourceRules": [
                    {
                        "verbs": ["get"],
                        "apiGroups": [""],
                        "resources": ["namespaces"],
                        "resourceNames": ["dev"],
                    },
                ],
            },
        })))
        .mount(&upstream)
        .await;
    // Deliberately no mock for selfsubjectaccessreviews: if the fast path
    // fails to resolve everything, the request would 503 rather than
    // silently falling back, so a missing mock is a strong enough signal.

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

    let sar_requests: Vec<_> = upstream
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|req| req.url.path() == "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews")
        .collect();
    assert!(
        sar_requests.is_empty(),
        "the rules review alone should have resolved every candidate"
    );

    delete_proxy(&pool, &ns, &cluster).await;
}

/// A rule with no `resourceNames` at all means every namespace is allowed —
/// still resolved by the rules review alone.
#[actix_web::test]
async fn list_fallback_resolves_unrestricted_rules_without_any_access_review() {
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

    Mock::given(method("POST"))
        .and(path(
            "/apis/authorization.k8s.io/v1/selfsubjectrulesreviews",
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "kind": "SelfSubjectRulesReview",
            "status": {
                "resourceRules": [
                    { "verbs": ["get", "list"], "apiGroups": [""], "resources": ["namespaces"] },
                ],
            },
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
    assert_eq!(names, vec!["dev", "prod"]);

    delete_proxy(&pool, &ns, &cluster).await;
}

/// `incomplete: true` means nothing is resolved: every candidate still falls
/// back to a real per-item access review, exactly like an unavailable review.
#[actix_web::test]
async fn list_fallback_falls_back_to_per_item_checks_when_incomplete() {
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

    Mock::given(method("POST"))
        .and(path(
            "/apis/authorization.k8s.io/v1/selfsubjectrulesreviews",
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "kind": "SelfSubjectRulesReview",
            "status": { "incomplete": true, "resourceRules": [] },
        })))
        .mount(&upstream)
        .await;

    Mock::given(method("POST"))
        .and(path(
            "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews",
        ))
        .and(body_partial_json(json!({
            "spec": { "resourceAttributes": { "name": "dev" } }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "status": { "allowed": true },
        })))
        .mount(&upstream)
        .await;
    Mock::given(method("POST"))
        .and(path(
            "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews",
        ))
        .and(body_partial_json(json!({
            "spec": { "resourceAttributes": { "name": "prod" } }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
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
    let names: Vec<&str> = body["items"]
        .as_array()
        .expect("items should be a list")
        .iter()
        .filter_map(|item| item["metadata"]["name"].as_str())
        .collect();
    assert_eq!(names, vec!["dev"]);

    delete_proxy(&pool, &ns, &cluster).await;
}

/// A second request from the same caller within the cache TTL skips both the
/// rules review and any access review — only the (always-fresh) privileged
/// namespace list is re-fetched.
#[actix_web::test]
async fn list_fallback_caches_the_resolved_allow_set() {
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

    Mock::given(method("POST"))
        .and(path(
            "/apis/authorization.k8s.io/v1/selfsubjectrulesreviews",
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "kind": "SelfSubjectRulesReview",
            "status": {
                "resourceRules": [
                    {
                        "verbs": ["get"],
                        "apiGroups": [""],
                        "resources": ["namespaces"],
                        "resourceNames": ["dev"],
                    },
                ],
            },
        })))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy_with_fallback(&pool, &ns, &cluster, &upstream.uri()).await;

    let app = proxy_app!(test_state(upstream.uri()));
    for _ in 0..2 {
        let req = test::TestRequest::get()
            .uri(&format!(
                "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projects"
            ))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = test::read_body_json(resp).await;
        assert_eq!(body["items"][0]["metadata"]["name"], "dev");
        assert_eq!(body["items"].as_array().map(Vec::len), Some(1));
    }

    let requests = upstream.received_requests().await.unwrap_or_default();
    let rules_review_calls = requests
        .iter()
        .filter(|req| req.url.path() == "/apis/authorization.k8s.io/v1/selfsubjectrulesreviews")
        .count();
    let namespace_list_calls = requests
        .iter()
        .filter(|req| req.method == "GET" && req.url.path() == "/api/v1/namespaces")
        .count();
    assert_eq!(
        rules_review_calls, 1,
        "the second call should hit the cache"
    );
    assert_eq!(
        namespace_list_calls, 2,
        "the candidate list itself is always fetched fresh"
    );

    delete_proxy(&pool, &ns, &cluster).await;
}
