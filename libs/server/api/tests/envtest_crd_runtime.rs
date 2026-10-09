//! `crd_runtime::ProxyKubeApiRuntime` against a real kube-apiserver.
//!
//! The fast tier (`libs/server/crd_runtime/tests/runtime.rs`) covers what a
//! fake upstream can: this tier covers the paths that read cluster objects —
//! a CA held in a Secret or ConfigMap, a target resolved from a
//! `KubernetesService` — and checks the resulting client really verifies the
//! apiserver's TLS certificate and authenticates with the rendered kubeconfig.
//!
//! Gated behind the `envtest` feature so a plain `cargo test` needs no binaries.

#![cfg(feature = "envtest")]

mod envtest_support;
mod harness;

use std::{collections::BTreeMap, sync::Arc};

use base64::{Engine, engine::general_purpose::STANDARD};
use common::{State, oidc_conf::OidcConf};
use crd::{ProxyKubeApi, certificate::CertSource, service::Service};
use crd_runtime::{ProxyKubeApiRuntime, ProxyRuntimeError};
use envtest_support::EnvTest;
use harness::{install_crypto_provider, proxy_fixture, unique_cluster};
use k8s_openapi::{
    ByteString,
    api::core::v1::{
        ConfigMap, Namespace, Secret, Service as KubeService, ServicePort, ServiceSpec,
    },
};
use kube::api::{Api, ListParams, ObjectMeta, PostParams};

macro_rules! envtest_or_skip {
    () => {
        match EnvTest::try_start().await {
            Some(env_test) => env_test,
            None => return,
        }
    };
}

const NS: &str = "default";

/// A `State` whose kube client is the envtest apiserver, so cert and service
/// lookups resolve for real. Redis is never touched on these paths.
fn state(env_test: &EnvTest) -> Arc<State> {
    install_crypto_provider();
    Arc::new(State::from_parts(
        env_test.client().expect("client should build"),
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

fn unique(prefix: &str) -> String {
    let (_, cluster) = unique_cluster();
    format!("{prefix}-{cluster}")
}

fn meta(name: &str, ns: &str) -> ObjectMeta {
    ObjectMeta {
        name: Some(name.to_string()),
        namespace: Some(ns.to_string()),
        ..ObjectMeta::default()
    }
}

async fn create_configmap(client: kube::Client, ns: &str, name: &str, key: &str, value: String) {
    Api::<ConfigMap>::namespaced(client, ns)
        .create(
            &PostParams::default(),
            &ConfigMap {
                metadata: meta(name, ns),
                data: Some(BTreeMap::from([(key.to_string(), value)])),
                ..ConfigMap::default()
            },
        )
        .await
        .expect("configmap should be created");
}

async fn create_secret(client: kube::Client, name: &str, key: &str, value: Vec<u8>) {
    Api::<Secret>::namespaced(client, NS)
        .create(
            &PostParams::default(),
            &Secret {
                metadata: meta(name, NS),
                data: Some(BTreeMap::from([(key.to_string(), ByteString(value))])),
                ..Secret::default()
            },
        )
        .await
        .expect("secret should be created");
}

/// The probe and a kube client built from the rendered kubeconfig both verify
/// the apiserver against `proxy`'s CA, and the client authenticates.
async fn assert_verified_and_authenticated(env_test: &EnvTest, proxy: &ProxyKubeApi) {
    let state = state(env_test);
    assert!(
        proxy
            .is_reachable(state.clone())
            .await
            .expect("probe over verified TLS should not error"),
        "the apiserver answers the anonymous probe"
    );

    let client = proxy
        .to_kube_client(state, None, Some(env_test.token().to_string()))
        .await
        .unwrap_or_else(|err| panic!("kube client should build: {err}"));
    Api::<Namespace>::all(client)
        .list(&ListParams::default())
        .await
        .expect("the rendered kubeconfig trusts the CA and carries the token");
}

#[tokio::test]
async fn ca_from_a_configmap_or_secret_verifies_the_apiserver() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    let pem = env_test.ca_pem().expect("the apiserver CA should exist");
    let (_, cluster) = unique_cluster();

    // ConfigMap: the PEM as plain text.
    let configmap = unique("ca-cm");
    create_configmap(client.clone(), NS, &configmap, "ca.crt", pem.clone()).await;
    let mut proxy = proxy_fixture(NS, &cluster, env_test.url());
    proxy.spec.cert = CertSource::ConfigMap {
        name: configmap,
        key: "ca.crt".to_string(),
        namespace: None,
    };
    assert_verified_and_authenticated(&env_test, &proxy).await;

    // Secret, the way cert-manager and `kubectl create secret` store it: the
    // raw PEM bytes (base64 only on the wire).
    let raw_secret = unique("ca-raw");
    create_secret(
        client.clone(),
        &raw_secret,
        "ca.crt",
        pem.clone().into_bytes(),
    )
    .await;
    proxy.spec.cert = CertSource::Secret {
        name: raw_secret,
        key: "ca.crt".to_string(),
        namespace: None,
    };
    assert_verified_and_authenticated(&env_test, &proxy).await;

    // Secret holding a base64-encoded PEM, the form older releases required.
    let b64_secret = unique("ca-b64");
    create_secret(
        client.clone(),
        &b64_secret,
        "ca.crt",
        STANDARD.encode(&pem).into_bytes(),
    )
    .await;
    proxy.spec.cert = CertSource::Secret {
        name: b64_secret,
        key: "ca.crt".to_string(),
        namespace: None,
    };
    assert_verified_and_authenticated(&env_test, &proxy).await;

    // A missing key is a typed error, surfaced by both the probe and the client.
    proxy.spec.cert = CertSource::ConfigMap {
        name: unique("absent"),
        key: "ca.crt".to_string(),
        namespace: None,
    };
    let err = proxy
        .get_client(state(&env_test))
        .await
        .expect_err("a missing configmap cannot yield a CA");
    assert!(
        matches!(
            err,
            ProxyRuntimeError::Cert(crd::certificate::CertError::Read {
                kind: "configmap",
                ..
            })
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_cross_namespace_ca_reference_is_pinned_to_the_resource_namespace() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    let pem = env_test.ca_pem().expect("the apiserver CA should exist");

    let other_ns = unique("other");
    Api::<Namespace>::all(client.clone())
        .create(
            &PostParams::default(),
            &Namespace {
                metadata: ObjectMeta {
                    name: Some(other_ns.clone()),
                    ..ObjectMeta::default()
                },
                ..Namespace::default()
            },
        )
        .await
        .expect("namespace should be created");
    let name = unique("ca");
    create_configmap(client, &other_ns, &name, "ca.crt", pem).await;

    // PROXYAUTH_ALLOW_CROSS_NS_CERT is unset: the read is pinned to `default`,
    // where no such ConfigMap exists, instead of leaking the other namespace's.
    let (_, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(NS, &cluster, env_test.url());
    proxy.spec.cert = CertSource::ConfigMap {
        name: name.clone(),
        key: "ca.crt".to_string(),
        namespace: Some(other_ns),
    };
    let err = proxy
        .to_kubeconfig(state(&env_test), None, None)
        .await
        .expect_err("cross-namespace read must be denied");
    match err {
        ProxyRuntimeError::Cert(crd::certificate::CertError::Read { name: read, .. }) => {
            assert_eq!(read, name);
        }
        other => panic!("expected a pinned, failed read, got {other:?}"),
    }
}

#[tokio::test]
async fn a_kubernetes_service_target_resolves_through_the_apiserver() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    let services: Api<KubeService> = Api::namespaced(client, NS);

    let name = unique("upstream");
    let created = services
        .create(
            &PostParams::default(),
            &KubeService {
                metadata: meta(&name, NS),
                spec: Some(ServiceSpec {
                    ports: Some(vec![
                        ServicePort {
                            name: Some("metrics".to_string()),
                            port: 9090,
                            ..ServicePort::default()
                        },
                        ServicePort {
                            name: Some("https".to_string()),
                            port: 6443,
                            ..ServicePort::default()
                        },
                    ]),
                    ..ServiceSpec::default()
                }),
                ..KubeService::default()
            },
        )
        .await
        .expect("service should be created");
    let cluster_ip = created
        .spec
        .and_then(|spec| spec.cluster_ip)
        .expect("the apiserver assigns a ClusterIP");

    let (_, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(NS, &cluster, "http://unused");
    let server_for = |proxy: &ProxyKubeApi| {
        let proxy = proxy.clone();
        let state = state(&env_test);
        async move {
            proxy
                .to_kubeconfig(state, None, None)
                .await
                .map(|kubeconfig| kubeconfig.clusters[0].cluster.clone().unwrap().server)
        }
    };

    proxy.spec.service = Service::KubernetesService {
        name: name.clone(),
        namespace: None,
        port: None,
        port_name: Some("https".to_string()),
    };
    assert_eq!(
        server_for(&proxy).await.unwrap().as_deref(),
        Some(format!("https://{cluster_ip}:6443").as_str())
    );

    proxy.spec.service = Service::KubernetesService {
        name: name.clone(),
        namespace: None,
        port: None,
        port_name: None,
    };
    assert_eq!(
        server_for(&proxy).await.unwrap().as_deref(),
        Some(format!("https://{cluster_ip}:9090").as_str()),
        "without a port selector the first port is used"
    );

    proxy.spec.service = Service::KubernetesService {
        name: name.clone(),
        namespace: None,
        port: Some(1),
        port_name: None,
    };
    assert!(matches!(
        server_for(&proxy).await,
        Err(ProxyRuntimeError::Service(
            crd::service::ServiceError::PortNotFound { port: 1, .. }
        ))
    ));

    // A service that does not exist makes the probe report "unreachable"
    // rather than fail.
    proxy.spec.service = Service::KubernetesService {
        name: unique("missing"),
        namespace: None,
        port: None,
        port_name: None,
    };
    assert!(!proxy.is_reachable(state(&env_test)).await.unwrap());
}
