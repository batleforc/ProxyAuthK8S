//! Network-facing behaviour of [`ProxyKubeApiRuntime`]: the reachability probe
//! and the upstream `reqwest` client it builds.
//!
//! The upstream is faked with `wiremock` (plain HTTP) or a raw TCP listener;
//! the kube client in `State` points at a closed port, so anything that would
//! need a real apiserver lives in the api crate's envtest tier instead
//! (`libs/server/api/tests/envtest_crd_runtime.rs`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{Engine, prelude::BASE64_STANDARD};
use common::{State, oidc_conf::OidcConf};
use crd::{ProxyKubeApi, ProxyKubeApiSpec, certificate::CertSource, service::Service};
use crd_runtime::{ProxyKubeApiRuntime, ProxyRuntimeError};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};

/// A real, self-signed CA certificate shared with the CLI's tests.
const TEST_CA: &str = include_str!("../../../cli/cli/testdata/test-ca.pem");

fn state() -> Arc<State> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let kube_config = kube::Config::new("http://127.0.0.1:1".parse().expect("static uri"));
    Arc::new(State::from_parts(
        kube::Client::try_from(kube_config).expect("kube client should build"),
        common::redis_pool::RedisPool::from_url_with_mode("redis://127.0.0.1:1", false)
            .expect("pool should build"),
        OidcConf {
            client_id: "proxyauthk8s".to_string(),
            client_secret: None,
            issuer_url: "https://idp.example.com".to_string(),
            scopes: "openid".to_string(),
            audience: "proxyauthk8s".to_string(),
            accept_authorized_party: false,
            redirect_url: None,
        },
        "https://proxy.example.com".to_string(),
        "https://front.example.com".to_string(),
    ))
}

fn proxy_to(url: &str) -> ProxyKubeApi {
    let mut proxy = ProxyKubeApi::new(
        "prod",
        ProxyKubeApiSpec {
            enabled: true,
            cert: CertSource::Insecure(true),
            client_cert: None,
            service: Service::ExternalService {
                url: url.to_string(),
            },
            auth_config: None,
            security_config: None,
            expose_via_dashboard: false,
            dashboard_group: None,
            proxy_group: None,
            virtual_apis: Vec::new(),
        },
    );
    proxy.metadata.namespace = Some("team-a".to_string());
    proxy
}

async fn upstream_answering(status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(status))
        .mount(&server)
        .await;
    server
}

/// A port that was just free: bind, read the port, drop the listener.
fn closed_port_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

#[tokio::test]
async fn reachable_on_a_successful_answer() {
    let server = upstream_answering(200).await;
    let reachable = proxy_to(&server.uri())
        .is_reachable(state())
        .await
        .expect("probe should not error");
    assert!(reachable);
}

#[tokio::test]
async fn reachable_when_the_upstream_refuses_the_anonymous_probe() {
    // An apiserver answers 401/403 to an unauthenticated `GET /`: it is up.
    for status in [401, 403, 404] {
        let server = upstream_answering(status).await;
        let reachable = proxy_to(&server.uri())
            .is_reachable(state())
            .await
            .expect("probe should not error");
        assert!(reachable, "status {status} means the upstream answered");
    }
}

#[tokio::test]
async fn unreachable_on_a_server_error() {
    for status in [500, 502, 503] {
        let server = upstream_answering(status).await;
        let reachable = proxy_to(&server.uri())
            .is_reachable(state())
            .await
            .expect("probe should not error");
        assert!(!reachable, "status {status} is not healthy");
    }
}

#[tokio::test]
async fn probe_hits_the_configured_url_verbatim() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::path("/livez"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let reachable = proxy_to(&format!("{}/livez", server.uri()))
        .is_reachable(state())
        .await
        .unwrap();
    assert!(reachable);
}

#[tokio::test]
async fn closed_port_is_a_transport_error() {
    let err = proxy_to(&closed_port_url())
        .is_reachable(state())
        .await
        .expect_err("connection refused carries no status");
    match err {
        ProxyRuntimeError::Http(source) => assert!(source.is_connect(), "{source:?}"),
        other => panic!("expected an Http error, got {other:?}"),
    }
}

#[tokio::test]
async fn unresolvable_service_is_unreachable_not_an_error() {
    // A KubernetesService needs the apiserver to resolve; ours is a closed port.
    let mut proxy = proxy_to("http://unused");
    proxy.spec.service = Service::KubernetesService {
        name: "kubernetes".to_string(),
        namespace: None,
        port: None,
        port_name: None,
    };
    assert!(!proxy.is_reachable(state()).await.unwrap());
}

#[tokio::test]
async fn black_holed_upstream_times_out() {
    // Accept the connection, then never answer: only the request timeout
    // (10s) can end the probe.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let holder = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });

    let started = Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(15),
        proxy_to(&format!("http://{addr}")).is_reachable(state()),
    )
    .await
    .expect("the client timeout must end the probe before the test guard");
    let elapsed = started.elapsed();
    holder.abort();

    match outcome {
        Err(ProxyRuntimeError::Http(source)) => assert!(source.is_timeout(), "{source:?}"),
        other => panic!("expected a timeout error, got {other:?}"),
    }
    assert!(
        elapsed >= Duration::from_secs(9) && elapsed < Duration::from_secs(11),
        "probe should end at the 10s request timeout, took {elapsed:?}"
    );
}

#[tokio::test]
async fn client_builds_without_a_ca() {
    assert!(proxy_to("http://unused").get_client(state()).await.is_ok());
}

#[tokio::test]
async fn client_builds_with_a_valid_inline_ca() {
    let mut proxy = proxy_to("http://unused");
    proxy.spec.cert = CertSource::Cert(BASE64_STANDARD.encode(TEST_CA));
    assert!(proxy.get_client(state()).await.is_ok());
}

#[tokio::test]
async fn client_rejects_a_ca_that_is_not_base64() {
    let mut proxy = proxy_to("http://unused");
    proxy.spec.cert = CertSource::Cert("%%%".to_string());
    let err = proxy.get_client(state()).await.expect_err("bad base64");
    assert!(
        matches!(
            err,
            ProxyRuntimeError::Cert(crd::certificate::CertError::Base64(_))
        ),
        "{err:?}"
    );
}

/// Text with no PEM block is rejected up front instead of yielding a client
/// that silently trusts only the default roots (rustls ignores non-PEM input).
#[tokio::test]
async fn ca_text_without_a_pem_block_is_rejected() {
    let mut proxy = proxy_to("http://unused");
    proxy.spec.cert = CertSource::Cert(BASE64_STANDARD.encode("definitely not a certificate"));
    let err = proxy.get_client(state()).await.unwrap_err();
    assert!(matches!(err, ProxyRuntimeError::CaWithoutPem), "{err:?}");
}

#[tokio::test]
async fn client_rejects_a_pem_block_with_a_corrupt_body() {
    let mut proxy = proxy_to("http://unused");
    let corrupt = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n";
    proxy.spec.cert = CertSource::Cert(BASE64_STANDARD.encode(corrupt));
    let err = proxy.get_client(state()).await.expect_err("corrupt DER");
    assert!(matches!(err, ProxyRuntimeError::Http(_)), "{err:?}");
    assert!(
        err.to_string().starts_with("upstream HTTP client error: "),
        "{err}"
    );
}

#[tokio::test]
async fn bad_ca_fails_the_probe_instead_of_reporting_unreachable() {
    let server = upstream_answering(200).await;
    let mut proxy = proxy_to(&server.uri());
    proxy.spec.cert = CertSource::Cert("%%%".to_string());
    assert!(matches!(
        proxy.is_reachable(state()).await,
        Err(ProxyRuntimeError::Cert(_))
    ));
}
