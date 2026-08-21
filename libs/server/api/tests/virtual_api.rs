//! Lot 4: the OpenShift Project virtual API, end to end through the proxy.
//!
//! The wiremock server is a vanilla cluster: it only knows about Namespaces.
//! Everything a client sees as a Project is synthesised by the proxy.

mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::redirect;
use base64::{Engine, prelude::BASE64_STANDARD};
use crd::certificate::CertSource;
use crd::virtual_api::VirtualApiKind;
use harness::{
    delete_proxy, proxy_fixture, security_config, seed_proxy, test_state, try_redis_pool,
    unique_cluster, with_virtual_api,
};
use serde_json::{Value, json};
use wiremock::matchers::{bearer_token, body_json, body_partial_json, method, path, query_param};
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

/// A proxy fixture with the OpenShift Project virtual API turned on.
async fn seed_openshift_proxy(
    pool: &deadpool_redis::Pool,
    ns: &str,
    cluster: &str,
    upstream_url: &str,
) {
    let mut proxy = proxy_fixture(ns, cluster, upstream_url);
    with_virtual_api(&mut proxy, VirtualApiKind::OpenShiftProject);
    seed_proxy(pool, &proxy).await;
}

/// The bearer token used by the `list_fallback_token`-enabled fixtures below.
const FALLBACK_TOKEN: &str = "test-fallback-token";

/// A proxy fixture with the OpenShift Project virtual API turned on and a
/// `list_fallback_token` configured, so `LIST projects` is unconditionally
/// filtered per namespace instead of a plain forwarded `LIST namespaces`.
async fn seed_openshift_proxy_with_fallback(
    pool: &deadpool_redis::Pool,
    ns: &str,
    cluster: &str,
    upstream_url: &str,
) {
    let mut proxy = proxy_fixture(ns, cluster, upstream_url);
    with_virtual_api(&mut proxy, VirtualApiKind::OpenShiftProject);
    proxy.spec.virtual_apis[0].list_fallback_token =
        Some(CertSource::Cert(BASE64_STANDARD.encode(FALLBACK_TOKEN)));
    seed_proxy(pool, &proxy).await;
}

#[actix_web::test]
async fn serves_group_discovery_without_touching_the_cluster() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy(&pool, &ns, &cluster, &upstream.uri()).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["kind"], "APIGroup");
    assert_eq!(body["name"], "project.openshift.io");
    assert_eq!(
        body["preferredVersion"]["groupVersion"],
        "project.openshift.io/v1"
    );
    // The cluster was never contacted for this.
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
async fn serves_version_discovery_without_touching_the_cluster() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy(&pool, &ns, &cluster, &upstream.uri()).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["kind"], "APIResourceList");
    assert_eq!(body["groupVersion"], "project.openshift.io/v1");
    assert_eq!(body["resources"][0]["name"], "projects");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn merges_the_virtual_group_into_the_cluster_group_list() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/apis"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "APIGroupList",
            "apiVersion": "v1",
            "groups": [
                { "name": "apps", "versions": [{ "groupVersion": "apps/v1", "version": "v1" }] },
            ],
        })))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy(&pool, &ns, &cluster, &upstream.uri()).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/apis"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    let names: Vec<&str> = body["groups"]
        .as_array()
        .expect("groups should be a list")
        .iter()
        .filter_map(|group| group["name"].as_str())
        .collect();
    assert_eq!(names, vec!["apps", "project.openshift.io"]);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn lists_namespaces_as_projects() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "NamespaceList",
            "apiVersion": "v1",
            "metadata": { "resourceVersion": "1234" },
            "items": [
                {
                    "metadata": {
                        "name": "dev",
                        "annotations": { "openshift.io/display-name": "Development" },
                    },
                    "status": { "phase": "Active" },
                },
                { "metadata": { "name": "prod" }, "status": { "phase": "Active" } },
            ],
        })))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy(&pool, &ns, &cluster, &upstream.uri()).await;

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
    assert_eq!(body["apiVersion"], "project.openshift.io/v1");
    assert_eq!(body["metadata"]["resourceVersion"], "1234");
    assert_eq!(body["items"][0]["kind"], "Project");
    assert_eq!(body["items"][0]["metadata"]["name"], "dev");
    assert_eq!(
        body["items"][0]["metadata"]["annotations"]["openshift.io/display-name"],
        "Development"
    );
    assert_eq!(body["items"][1]["metadata"]["name"], "prod");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn reads_a_single_namespace_as_a_project() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/dev"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "Namespace",
            "apiVersion": "v1",
            "metadata": { "name": "dev", "uid": "abc-123" },
            "status": { "phase": "Active" },
        })))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy(&pool, &ns, &cluster, &upstream.uri()).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projects/dev"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["kind"], "Project");
    assert_eq!(body["apiVersion"], "project.openshift.io/v1");
    assert_eq!(body["metadata"]["name"], "dev");
    assert_eq!(body["metadata"]["uid"], "abc-123");
    assert_eq!(body["status"]["phase"], "Active");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn a_project_request_creates_a_namespace() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    // The cluster must receive a Namespace, never a ProjectRequest.
    Mock::given(method("POST"))
        .and(path("/api/v1/namespaces"))
        .and(body_json(json!({
            "kind": "Namespace",
            "apiVersion": "v1",
            "metadata": {
                "name": "dev",
                "annotations": {
                    "openshift.io/display-name": "Development",
                    "openshift.io/description": "the dev project",
                },
            },
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "kind": "Namespace",
            "apiVersion": "v1",
            "metadata": { "name": "dev" },
        })))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy(&pool, &ns, &cluster, &upstream.uri()).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::post()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projectrequests"
        ))
        .insert_header(("content-type", "application/json"))
        .set_payload(
            json!({
                "kind": "ProjectRequest",
                "apiVersion": "project.openshift.io/v1",
                "metadata": { "name": "dev" },
                "displayName": "Development",
                "description": "the dev project",
            })
            .to_string(),
        )
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["kind"], "Project");
    assert_eq!(body["metadata"]["name"], "dev");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn deleting_a_project_deletes_the_namespace() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("DELETE"))
        .and(path("/api/v1/namespaces/dev"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "Namespace",
            "apiVersion": "v1",
            "metadata": { "name": "dev" },
            "status": { "phase": "Terminating" },
        })))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy(&pool, &ns, &cluster, &upstream.uri()).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::delete()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projects/dev"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["kind"], "Project");
    assert_eq!(body["status"]["phase"], "Terminating");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn translates_each_event_of_a_watch_stream() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    let events = concat!(
        r#"{"type":"ADDED","object":{"kind":"Namespace","apiVersion":"v1","metadata":{"name":"dev"}}}"#,
        "\n",
        r#"{"type":"MODIFIED","object":{"kind":"Namespace","apiVersion":"v1","metadata":{"name":"prod"}}}"#,
        "\n",
    );

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces"))
        .and(query_param("watch", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_string(events))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy(&pool, &ns, &cluster, &upstream.uri()).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projects?watch=true"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = test::read_body(resp).await;
    let lines: Vec<Value> = String::from_utf8_lossy(&body)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("each event should be JSON"))
        .collect();

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["type"], "ADDED");
    assert_eq!(lines[0]["object"]["kind"], "Project");
    assert_eq!(lines[0]["object"]["apiVersion"], "project.openshift.io/v1");
    assert_eq!(lines[0]["object"]["metadata"]["name"], "dev");
    assert_eq!(lines[1]["type"], "MODIFIED");
    assert_eq!(lines[1]["object"]["kind"], "Project");
    assert_eq!(lines[1]["object"]["metadata"]["name"], "prod");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn an_upstream_error_reaches_the_client_untranslated() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    // The documented limitation: a user without cluster-wide list rights gets
    // the apiserver's own 403, not an empty ProjectList.
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "kind": "Status",
            "apiVersion": "v1",
            "status": "Failure",
            "reason": "Forbidden",
            "code": 403,
        })))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_openshift_proxy(&pool, &ns, &cluster, &upstream.uri()).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projects"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["kind"], "Status");
    assert_eq!(body["reason"], "Forbidden");

    delete_proxy(&pool, &ns, &cluster).await;
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
        .and(path("/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"))
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
        .and(path("/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"))
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
        .and(path("/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"))
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
        .and(path("/apis/authorization.k8s.io/v1/selfsubjectrulesreviews"))
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
        .and(path("/apis/authorization.k8s.io/v1/selfsubjectrulesreviews"))
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
        .and(path("/apis/authorization.k8s.io/v1/selfsubjectrulesreviews"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "kind": "SelfSubjectRulesReview",
            "status": { "incomplete": true, "resourceRules": [] },
        })))
        .mount(&upstream)
        .await;

    Mock::given(method("POST"))
        .and(path("/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"))
        .and(body_partial_json(json!({
            "spec": { "resourceAttributes": { "name": "dev" } }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "status": { "allowed": true },
        })))
        .mount(&upstream)
        .await;
    Mock::given(method("POST"))
        .and(path("/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"))
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
        .and(path("/apis/authorization.k8s.io/v1/selfsubjectrulesreviews"))
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
    assert_eq!(rules_review_calls, 1, "the second call should hit the cache");
    assert_eq!(
        namespace_list_calls, 2,
        "the candidate list itself is always fetched fresh"
    );

    delete_proxy(&pool, &ns, &cluster).await;
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
