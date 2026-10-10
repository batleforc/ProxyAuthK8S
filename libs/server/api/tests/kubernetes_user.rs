//! `User::auth_against_kubernetes` against a wiremock Kubernetes apiserver.
//!
//! The Kubernetes auth mode resolves the caller by asking the *target cluster*
//! who the caller's token belongs to, via a `SelfSubjectReview`. The proxy
//! fixture points at `ExternalService` with a `CertSource::SystemRoots` cert, so
//! `to_kube_client` builds a plain-HTTP client aimed straight at the mock and
//! no real apiserver (nor Redis) is involved — the OIDC mode is covered the
//! same way in `oidc_user.rs`.

mod harness;

use api::model::user::{User, UserAuthError};
use crd::ProxyKubeApi;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Where a Kubernetes client sends `SelfSubjectReview` creates: the resource is
/// cluster-scoped and lives in `authentication.k8s.io/v1`.
const REVIEW_PATH: &str = "/apis/authentication.k8s.io/v1/selfsubjectreviews";

const CALLER_TOKEN: &str = "caller-token";

fn proxy(server: &MockServer) -> ProxyKubeApi {
    harness::proxy_fixture("default", "kube-auth-cluster", &server.uri())
}

/// Answer a `SelfSubjectReview` with the given `status` object.
async fn mount_review(server: &MockServer, status: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path(REVIEW_PATH))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "apiVersion": "authentication.k8s.io/v1",
            "kind": "SelfSubjectReview",
            "status": status,
        })))
        .mount(server)
        .await;
}

async fn mount_review_failure(server: &MockServer, status_code: u16, body: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path(REVIEW_PATH))
        .respond_with(ResponseTemplate::new(status_code).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn resolves_the_caller_from_a_self_subject_review() {
    let server = MockServer::start().await;
    mount_review(
        &server,
        json!({
            "userInfo": {
                "username": "alice",
                "uid": "alice-uid",
                "groups": ["dev-alice", "system:authenticated"],
                "extra": { "email": ["alice@example.com"] },
            }
        }),
    )
    .await;

    let user = User::auth_against_kubernetes(
        harness::test_state(server.uri()),
        proxy(&server),
        CALLER_TOKEN.to_string(),
    )
    .await
    .expect("the review should succeed")
    .expect("a user should be resolved");

    assert_eq!(user.username, "alice");
    assert_eq!(user.email, "alice@example.com");
    assert_eq!(user.groups, vec!["dev-alice", "system:authenticated"]);
    assert!(user.is_in_group("system:authenticated"));
    assert!(!user.is_in_group("admins"));
}

#[tokio::test]
async fn reviews_the_callers_own_token_against_the_target_cluster() {
    // The whole point of this auth mode: the review must be sent *as the
    // caller*, with an empty request body, so the apiserver reports the
    // caller's identity rather than the proxy's own service account.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(REVIEW_PATH))
        .and(header(
            "authorization",
            format!("Bearer {CALLER_TOKEN}").as_str(),
        ))
        // An empty review: the identity comes from the token alone, never from
        // anything the proxy puts in the body.
        .and(body_json(json!({
            "apiVersion": "authentication.k8s.io/v1",
            "kind": "SelfSubjectReview",
            "metadata": {},
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "apiVersion": "authentication.k8s.io/v1",
            "kind": "SelfSubjectReview",
            "status": { "userInfo": { "username": "alice" } },
        })))
        .mount(&server)
        .await;

    let user = User::auth_against_kubernetes(
        harness::test_state(server.uri()),
        proxy(&server),
        CALLER_TOKEN.to_string(),
    )
    .await
    .expect("the review should succeed")
    .expect("a user should be resolved");
    assert_eq!(user.username, "alice");

    // A mismatched token would have missed the mock above, but assert on the
    // recorded request too so a failure names the reason.
    let requests = server
        .received_requests()
        .await
        .expect("the mock server records requests");
    let review = requests
        .iter()
        .find(|request| request.url.path() == REVIEW_PATH)
        .expect("the review request should have been sent");
    assert_eq!(
        review
            .headers
            .get("authorization")
            .expect("the request should be authenticated"),
        &format!("Bearer {CALLER_TOKEN}")
    );
}

#[tokio::test]
async fn a_review_without_user_info_resolves_to_an_empty_user() {
    // A cluster may answer with no `status`, or a `status` with no `userInfo`.
    // Neither is an error: the caller is simply anonymous, and the empty group
    // list matches no group restriction downstream.
    for status in [json!({}), json!({ "userInfo": {} })] {
        let server = MockServer::start().await;
        mount_review(&server, status).await;

        let user = User::auth_against_kubernetes(
            harness::test_state(server.uri()),
            proxy(&server),
            CALLER_TOKEN.to_string(),
        )
        .await
        .expect("the review should succeed")
        .expect("a user should be resolved");

        assert_eq!(user.username, "");
        assert_eq!(user.email, "");
        assert!(user.groups.is_empty());
    }
}

#[tokio::test]
async fn an_email_is_only_read_from_the_extra_map_when_it_is_there() {
    // `extra` is free-form and provider-dependent; most clusters send no
    // `email` at all, and the first value wins when several are present.
    let cases = [
        (json!({ "username": "alice" }), ""),
        (
            json!({ "username": "alice", "extra": { "other": ["x"] } }),
            "",
        ),
        (json!({ "username": "alice", "extra": { "email": [] } }), ""),
        (
            json!({
                "username": "alice",
                "extra": { "email": ["first@example.com", "second@example.com"] },
            }),
            "first@example.com",
        ),
    ];

    for (user_info, expected_email) in cases {
        let server = MockServer::start().await;
        mount_review(&server, json!({ "userInfo": user_info })).await;

        let user = User::auth_against_kubernetes(
            harness::test_state(server.uri()),
            proxy(&server),
            CALLER_TOKEN.to_string(),
        )
        .await
        .expect("the review should succeed")
        .expect("a user should be resolved");

        assert_eq!(user.username, "alice");
        assert_eq!(user.email, expected_email);
    }
}

#[tokio::test]
async fn a_token_the_cluster_refuses_is_reported_as_a_review_failure() {
    let server = MockServer::start().await;
    mount_review_failure(
        &server,
        401,
        json!({
            "kind": "Status",
            "apiVersion": "v1",
            "status": "Failure",
            "message": "Unauthorized",
            "reason": "Unauthorized",
            "code": 401,
        }),
    )
    .await;

    let error = User::auth_against_kubernetes(
        harness::test_state(server.uri()),
        proxy(&server),
        "expired-token".to_string(),
    )
    .await
    .expect_err("an unauthenticated review should fail");

    assert!(
        matches!(error, UserAuthError::SelfSubjectReview(_)),
        "expected a SelfSubjectReview error, got {error:?}"
    );
}

#[tokio::test]
async fn an_apiserver_failure_is_reported_as_a_review_failure() {
    let server = MockServer::start().await;
    mount_review_failure(
        &server,
        500,
        json!({
            "kind": "Status",
            "apiVersion": "v1",
            "status": "Failure",
            "message": "internal error",
            "code": 500,
        }),
    )
    .await;

    let error = User::auth_against_kubernetes(
        harness::test_state(server.uri()),
        proxy(&server),
        CALLER_TOKEN.to_string(),
    )
    .await
    .expect_err("a failing apiserver should surface an error");

    assert!(
        matches!(error, UserAuthError::SelfSubjectReview(_)),
        "expected a SelfSubjectReview error, got {error:?}"
    );
}

#[tokio::test]
async fn an_unreachable_cluster_is_reported_as_a_review_failure() {
    // Nothing listens on port 1, so the request fails at the transport layer
    // rather than with a Kubernetes `Status` — still a review failure, and
    // never a silently anonymous user.
    let proxy = harness::proxy_fixture("default", "kube-auth-cluster", "http://127.0.0.1:1");

    let error = User::auth_against_kubernetes(
        // No mock server here: nothing on this path reads the OIDC issuer.
        harness::test_state("https://oidc.example.com".to_string()),
        proxy,
        CALLER_TOKEN.to_string(),
    )
    .await
    .expect_err("an unreachable cluster should fail");

    assert!(
        matches!(error, UserAuthError::SelfSubjectReview(_)),
        "expected a SelfSubjectReview error, got {error:?}"
    );
}

#[tokio::test]
async fn the_kubernetes_validate_against_mode_routes_to_the_review() {
    // `get_user_info_with_proxy` is the dispatch point: with
    // `validate_against: Kubernetes` it must ask the cluster, not the OIDC
    // provider — no OIDC endpoint is mounted here, so a mis-dispatch fails.
    let server = MockServer::start().await;
    mount_review(
        &server,
        json!({ "userInfo": { "username": "alice", "groups": ["platform"] } }),
    )
    .await;

    let mut proxy = proxy(&server);
    proxy.spec.auth_config = Some(harness::kubernetes_auth_config());

    let user = User::get_user_info_with_proxy(
        harness::test_state(server.uri()),
        proxy,
        CALLER_TOKEN.to_string(),
    )
    .await
    .expect("the review should succeed")
    .expect("a user should be resolved");

    assert_eq!(user.username, "alice");
    assert_eq!(user.groups, vec!["platform"]);
}

#[tokio::test]
async fn a_proxy_without_an_auth_config_resolves_to_no_user_without_calling_the_cluster() {
    let server = MockServer::start().await;
    // No mock mounted at all: any upstream call would 404 and fail the review.
    let mut proxy = proxy(&server);
    proxy.spec.auth_config = None;

    let user = User::get_user_info_with_proxy(
        harness::test_state(server.uri()),
        proxy,
        CALLER_TOKEN.to_string(),
    )
    .await
    .expect("an unauthenticated proxy is not an error");

    assert!(user.is_none(), "expected no user, got {user:?}");
    assert!(
        server
            .received_requests()
            .await
            .expect("the mock server records requests")
            .is_empty(),
        "no upstream request should have been made"
    );
}

#[tokio::test]
async fn a_cluster_url_that_cannot_be_parsed_fails_before_any_request() {
    // The service URL comes from the CR, so a malformed one must surface as a
    // client-build failure rather than a panic — and be told apart from a
    // review the cluster actually answered.
    let proxy = harness::proxy_fixture("default", "kube-auth-cluster", "http://[::1");

    let error = User::auth_against_kubernetes(
        harness::test_state("https://oidc.example.com".to_string()),
        proxy,
        CALLER_TOKEN.to_string(),
    )
    .await
    .expect_err("an unparseable cluster URL should fail");

    assert!(
        matches!(error, UserAuthError::Runtime(_)),
        "expected a runtime error, got {error:?}"
    );
}
