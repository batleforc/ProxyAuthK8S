//! RFC 0014 T28 (decision 50): in a trusted mode the apiserver no longer
//! checks the audience, so the proxy must. With `OIDC_AUDIENCE_VALIDATION`
//! off, a `FrontProxy` or `Issuer` cluster is closed with `503` and the
//! message names who can act, before any token is read; a `Passthrough`
//! cluster keeps working as before. RFC 0010 T11: `TokenExchange` replaces
//! the token too, so it is closed the same way.
//!
//! Each test installs the process-wide configuration, which holds because
//! nextest runs every test in its own process.

mod harness;

use axum::http::StatusCode;
use crd::upstream_auth::{UpstreamAuth, UpstreamAuthMode};
use harness::{memory_state, memory_store, proxy_fixture, seed_proxy, test, unique_cluster};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

fn audience_validation_off() {
    let config = common::config::init(common::config::Config::from_lookup(|key| match key {
        "OIDC_AUDIENCE_VALIDATION" => Some("off".to_string()),
        "UPSTREAM_AUTH_ALLOWED_MODES" => Some("FrontProxy,Issuer,TokenExchange".to_string()),
        _ => None,
    }));
    assert_eq!(
        config.oidc.audience_validation,
        common::token_audience::AudienceValidationMode::Off,
        "another configuration was installed first; run under nextest"
    );
}

async fn status_of(mode: Option<UpstreamAuthMode>) -> (StatusCode, String, usize) {
    let store = memory_store();
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&upstream)
        .await;
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.upstream_auth = mode.map(|mode| UpstreamAuth {
        mode,
        user_prefix: Some("proxyauth:".to_string()),
        group_prefix: Some("proxyauth:".to_string()),
        issuer_key: None,
        token_exchange: (mode == UpstreamAuthMode::TokenExchange).then(Default::default),
    });
    seed_proxy(&store, &proxy).await;

    let app = harness::app(memory_state(&store, upstream.uri()));
    let request = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", "Bearer anything"))
        .to_request();
    let response = test::call_service(&app, request).await;
    let status = response.status();
    let body = String::from_utf8_lossy(&test::read_body(response).await).to_string();
    let reached = upstream.received_requests().await.unwrap_or_default().len();
    (status, body, reached)
}

#[tokio::test]
async fn a_trusted_mode_without_audience_validation_is_refused() {
    audience_validation_off();
    for mode in [UpstreamAuthMode::FrontProxy, UpstreamAuthMode::Issuer] {
        let (status, body, reached) = status_of(Some(mode)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{mode:?}: {body}");
        assert!(body.contains("OIDC_AUDIENCE_VALIDATION=enforce"), "{body}");
        assert_eq!(reached, 0, "{mode:?}: the apiserver must not be reached");
    }
}

#[tokio::test]
async fn token_exchange_without_audience_validation_is_refused() {
    audience_validation_off();
    let (status, body, reached) = status_of(Some(UpstreamAuthMode::TokenExchange)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body.contains("OIDC_AUDIENCE_VALIDATION=enforce"), "{body}");
    assert_eq!(reached, 0, "the apiserver must not be reached");
}

#[tokio::test]
async fn passthrough_without_audience_validation_is_served_as_before() {
    audience_validation_off();
    let (status, body, reached) = status_of(None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(reached, 1);
}
