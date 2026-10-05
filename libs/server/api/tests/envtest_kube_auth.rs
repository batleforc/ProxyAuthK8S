//! The `ValidateAgainst::Kubernetes` auth mode, against a real kube-apiserver.
//!
//! In this mode the proxy does not interpret the bearer token itself: it asks
//! the target cluster who the caller is with a `SelfSubjectReview` (GA since
//! Kubernetes 1.28; envtest runs 1.31). A wiremock could only replay a canned
//! answer, so this tier checks the real thing: a ServiceAccount token minted by
//! the apiserver resolves to the identity the apiserver assigns it, and a token
//! the apiserver does not know is refused.
//!
//! The proxy reaches the apiserver over TLS with `CertSource::Cert` (the
//! envtest's self-signed bundle), so certificate verification stays on.
//!
//! Gated behind the `envtest` feature so a plain `cargo test` needs no binaries.

#![cfg(feature = "envtest")]

mod envtest_support;
mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::redirect;
use api::model::user::{User, UserAuthError};
use base64::Engine;
use crd::{
    ProxyKubeApi,
    authentication_configuration::{AuthenticationConfiguration, OidcProvider, ValidateAgainst},
    certificate::CertSource,
};
use envtest_support::EnvTest;
use harness::{
    delete_proxy, proxy_fixture, seed_proxy, test_state, try_redis_pool, unique_cluster,
};
use k8s_openapi::api::authentication::v1::{TokenRequest, TokenRequestSpec};
use k8s_openapi::api::core::v1::ServiceAccount;
use kube::api::{Api, ObjectMeta, PostParams};

macro_rules! envtest_or_skip {
    () => {
        match EnvTest::try_start().await {
            Some(env_test) => env_test,
            None => return,
        }
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

const SA_NAMESPACE: &str = "default";

/// Create a ServiceAccount and mint a short-lived token for it via TokenRequest.
async fn service_account_token(client: kube::Client, name: &str) -> String {
    let accounts: Api<ServiceAccount> = Api::namespaced(client, SA_NAMESPACE);
    accounts
        .create(
            &PostParams::default(),
            &ServiceAccount {
                metadata: ObjectMeta {
                    name: Some(name.to_string()),
                    ..ObjectMeta::default()
                },
                ..ServiceAccount::default()
            },
        )
        .await
        .expect("the ServiceAccount should be created");

    let request = TokenRequest {
        spec: TokenRequestSpec {
            audiences: Vec::new(),
            expiration_seconds: Some(600),
            ..TokenRequestSpec::default()
        },
        ..TokenRequest::default()
    };
    accounts
        .create_token_request(name, &PostParams::default(), &request)
        .await
        .expect("the TokenRequest should be served")
        .status
        .expect("a TokenRequest response carries a status")
        .token
}

/// A proxy targeting the envtest apiserver, validating tokens against it.
fn kubernetes_mode_proxy(env_test: &EnvTest, ns: &str, cluster: &str) -> ProxyKubeApi {
    let mut proxy = proxy_fixture(ns, cluster, env_test.url());
    proxy.spec.cert = CertSource::Cert(
        base64::engine::general_purpose::STANDARD
            .encode(env_test.ca_pem().expect("the apiserver CA should exist")),
    );
    proxy.spec.auth_config = Some(AuthenticationConfiguration {
        jwt: Vec::new(),
        oidc_provider: OidcProvider {
            enabled: false,
            issuer_url: String::new(),
            client_id: String::new(),
            client_secret: None,
            extra_scope: String::new(),
            audience: String::new(),
            accept_authorized_party: false,
            expose_oauth_authorization_server: false,
        },
        disable_validation: false,
        validate_against: ValidateAgainst::Kubernetes,
    });
    proxy
}

fn unique_account(prefix: &str) -> String {
    let (_, cluster) = unique_cluster();
    format!("{prefix}-{cluster}")
}

#[tokio::test]
async fn a_service_account_token_resolves_to_its_kubernetes_identity() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    let account = unique_account("sa");
    let token = service_account_token(client, &account).await;

    let (ns, cluster) = unique_cluster();
    let proxy = kubernetes_mode_proxy(&env_test, &ns, &cluster);
    // Redis is never touched on this path, so any `State` will do.
    let user = User::auth_against_kubernetes(test_state(String::new()), proxy, token)
        .await
        .expect("a valid token should be accepted")
        .expect("a valid token should resolve to a user");

    assert_eq!(
        user.username,
        format!("system:serviceaccount:{SA_NAMESPACE}:{account}")
    );
    for group in [
        "system:serviceaccounts",
        &format!("system:serviceaccounts:{SA_NAMESPACE}"),
        "system:authenticated",
    ] {
        assert!(
            user.is_in_group(group),
            "missing {group} in {:?}",
            user.groups
        );
    }
    // ServiceAccounts carry no `email` extra.
    assert_eq!(user.email, "");
}

/// The dispatcher must route `ValidateAgainst::Kubernetes` to the review.
#[tokio::test]
async fn get_user_info_with_proxy_dispatches_to_the_kubernetes_mode() {
    let env_test = envtest_or_skip!();
    let user = User::get_user_info_with_proxy(
        test_state(String::new()),
        kubernetes_mode_proxy(&env_test, "default", "dispatch"),
        env_test.token().to_string(),
    )
    .await
    .expect("the static admin token should be accepted")
    .expect("the static admin token should resolve to a user");

    // Identity declared in the envtest `--token-auth-file`.
    assert_eq!(user.username, "envtest-admin");
    assert!(user.is_in_group("system:masters"), "{:?}", user.groups);
}

#[tokio::test]
async fn an_unknown_token_is_rejected_by_the_self_subject_review() {
    let env_test = envtest_or_skip!();
    let (ns, cluster) = unique_cluster();
    let proxy = kubernetes_mode_proxy(&env_test, &ns, &cluster);

    let result = User::auth_against_kubernetes(
        test_state(String::new()),
        proxy,
        "not-a-real-token".to_string(),
    )
    .await;

    match result {
        Err(UserAuthError::SelfSubjectReview(kube::Error::Api(status))) => {
            assert_eq!(status.code, 401, "unexpected status: {status:?}");
        }
        other => panic!("expected a 401 SelfSubjectReview error, got {other:?}"),
    }
}

/// A token for a ServiceAccount that was deleted must stop working: the
/// apiserver checks the account still exists, and the proxy defers to it.
#[tokio::test]
async fn a_token_of_a_deleted_service_account_is_rejected() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    let account = unique_account("gone");
    let token = service_account_token(client.clone(), &account).await;
    let accounts: Api<ServiceAccount> = Api::namespaced(client, SA_NAMESPACE);
    accounts
        .delete(&account, &Default::default())
        .await
        .expect("the ServiceAccount should be deleted");

    let (ns, cluster) = unique_cluster();
    let result = User::auth_against_kubernetes(
        test_state(String::new()),
        kubernetes_mode_proxy(&env_test, &ns, &cluster),
        token,
    )
    .await;

    assert!(
        matches!(result, Err(UserAuthError::SelfSubjectReview(_))),
        "expected a SelfSubjectReview error, got {result:?}"
    );
}

macro_rules! proxy_app {
    ($state:expr_2021) => {
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

/// End to end through the proxy: the token is validated against the cluster,
/// then forwarded unchanged, so the upstream answers as that ServiceAccount.
#[actix_web::test]
async fn the_proxy_forwards_a_valid_kubernetes_token() {
    let pool = redis_or_skip!();
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    let account = unique_account("proxy");
    let token = service_account_token(client, &account).await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &kubernetes_mode_proxy(&env_test, &ns, &cluster)).await;

    let app = proxy_app!(test_state(String::new()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/{SA_NAMESPACE}/serviceaccounts/{account}"
        ))
        .insert_header(("Authorization", format!("Bearer {token}")))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let status = resp.status();
    let body: serde_json::Value =
        serde_json::from_slice(&test::read_body(resp).await).expect("JSON body");

    delete_proxy(&pool, &ns, &cluster).await;
    assert_eq!(status, StatusCode::OK, "unexpected body: {body}");
    assert_eq!(body["kind"], "ServiceAccount");
    assert_eq!(body["metadata"]["name"], account.as_str());
}

/// The proxy must answer 401 itself, without leaking the review's detail.
#[actix_web::test]
async fn the_proxy_rejects_an_invalid_kubernetes_token_with_401() {
    let pool = redis_or_skip!();
    let env_test = envtest_or_skip!();

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &kubernetes_mode_proxy(&env_test, &ns, &cluster)).await;

    let app = proxy_app!(test_state(String::new()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/namespaces"))
        .insert_header(("Authorization", "Bearer not-a-real-token"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let status = resp.status();
    let body = String::from_utf8_lossy(&test::read_body(resp).await).to_string();

    delete_proxy(&pool, &ns, &cluster).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        !body.contains("SelfSubjectReview"),
        "the 401 must not leak the review detail: {body}"
    );
}
