//! Lot 2: per-cluster authorization, the resource allow-list, and the headers
//! the proxy is authoritative for.
//!
//! One wiremock server plays both the OIDC provider and the upstream Kubernetes
//! API, so a request can be authenticated and forwarded in the same scenario.

mod harness;

use actix_web::{http::StatusCode, test, web, App};
use api::cluster::redirect;
use harness::{
    delete_proxy, mount_oidc_provider, oidc_auth_config, proxy_fixture, security_config,
    seed_proxy, test_state, try_redis_pool, unique_cluster,
};
use wiremock::matchers::{header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A JWT-shaped access token whose `aud` claim names `oidc_auth_config`'s
/// `client_id` (`proxyauthk8s`). `/userinfo` (mocked below) doesn't check the
/// token value, but `ensure_token_audience` runs on every request after it and
/// fails closed under the default Enforce mode — an opaque token has no `aud`
/// to check, so it would be rejected before ever reaching the group/path
/// checks these tests exist to exercise. Signature is not verified here.
const VALID_TOKEN: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJwcm94eWF1dGhrOHMiLCJzdWIiOiJhbGljZS1zdWIifQ.c2lnbmF0dXJlLW5vdC12ZXJpZmllZC1pbi10aGVzZS10ZXN0cw";

macro_rules! proxy_app {
    ($state:expr) => {
        test::init_service(
            App::new().app_data(web::Data::new($state)).service(
                web::scope("/clusters")
                    .service(redirect::get_redirect)
                    .service(redirect::post_redirect),
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

/// The proxy answers its own rejections with a Kubernetes `Status` object.
async fn assert_kubernetes_status(resp: actix_web::dev::ServiceResponse, code: u64) {
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["kind"], "Status");
    assert_eq!(body["apiVersion"], "v1");
    assert_eq!(body["status"], "Failure");
    assert_eq!(body["code"], code);
}

#[actix_web::test]
async fn allows_a_user_inside_the_proxy_group() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    mount_oidc_provider(&upstream, "alice", &["operators"]).await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    proxy.spec.proxy_group = Some("operators".to_string());
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", format!("Bearer {VALID_TOKEN}")))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn rejects_a_user_outside_the_proxy_group() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    mount_oidc_provider(&upstream, "alice", &["viewers"]).await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    proxy.spec.proxy_group = Some("operators".to_string());
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", format!("Bearer {VALID_TOKEN}")))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_kubernetes_status(resp, 403).await;
    // The upstream must not have seen the Kubernetes request at all.
    let kube_calls = upstream
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path() == "/api/v1/pods")
        .count();
    assert_eq!(kube_calls, 0);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn refuses_a_group_restricted_cluster_when_validation_is_disabled() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    // A group is required but nothing resolves a user: the check can never run.
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.proxy_group = Some("operators".to_string());
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn dashboard_exposure_restricts_the_proxy_too() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    mount_oidc_provider(&upstream, "alice", &["unrelated"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    proxy.spec.expose_via_dashboard = true;
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", format!("Bearer {VALID_TOKEN}")))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn rejects_a_path_outside_the_allow_list() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("should not be reached"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.security_config = Some(security_config(vec![(
        "/api/v1/namespaces/dev/pods/**",
        true,
    )]));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/kube-system/secrets"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_kubernetes_status(resp, 403).await;
    assert!(upstream
        .received_requests()
        .await
        .unwrap_or_default()
        .is_empty());

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn allows_a_path_inside_the_allow_list() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/dev/pods/mypod/log"))
        .respond_with(ResponseTemplate::new(200).set_body_string("logs"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.security_config = Some(security_config(vec![(
        "/api/v1/namespaces/dev/pods/**",
        true,
    )]));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods/mypod/log"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(test::read_body(resp).await, "logs");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn the_allow_list_matches_on_the_path_not_the_query_string() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/dev/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.security_config = Some(security_config(vec![(
        "/api/v1/namespaces/dev/pods",
        false,
    )]));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods?watch=true"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn forwards_the_caller_identity_and_appends_to_the_forwarded_chain() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    mount_oidc_provider(&upstream, "alice", &["operators", "dev"]).await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", format!("Bearer {VALID_TOKEN}")))
        .insert_header(("x-forwarded-for", "203.0.113.7"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);

    let received = upstream
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|r| r.url.path() == "/api/v1/pods")
        .expect("the upstream should have received the request");

    let header = |name: &str| {
        received
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };

    assert_eq!(header("x-forwarded-user").as_deref(), Some("alice"));
    assert_eq!(
        header("x-forwarded-groups").as_deref(),
        Some("operators,dev")
    );
    // The chain the client sent must survive; the peer is appended to it.
    let forwarded_for = header("x-forwarded-for").expect("x-forwarded-for should be present");
    assert!(
        forwarded_for.starts_with("203.0.113.7"),
        "unexpected chain: {forwarded_for}"
    );

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn drops_client_supplied_identity_headers() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    // No token validation, so no identity can be asserted: the forged headers
    // must simply disappear rather than reach the cluster.
    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream.uri())).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("x-forwarded-user", "cluster-admin"))
        .insert_header(("x-forwarded-groups", "system:masters"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);

    let received = upstream
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|r| r.url.path() == "/api/v1/pods")
        .expect("the upstream should have received the request");
    assert!(received.headers.get("x-forwarded-user").is_none());
    assert!(received.headers.get("x-forwarded-groups").is_none());

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn an_empty_allow_list_forwards_everything() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(header_exists("x-forwarded-for"))
        .respond_with(ResponseTemplate::new(200).set_body_string("anything"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.security_config = Some(security_config(vec![]));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/kube-system/secrets"
        ))
        .insert_header(("x-forwarded-for", "203.0.113.7"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);

    delete_proxy(&pool, &ns, &cluster).await;
}
