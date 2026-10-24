//! Admission rules of `spec.service.Agent` (RFC 0012 §4.3), against a real
//! apiserver.
//!
//! The CEL rules must compile inside the apiserver's cost budget and refuse
//! what `AgentService::validate` refuses: `isIP` and the `epoch` transition
//! rule (`oldSelf`) only exist in a real apiserver.
#![cfg_attr(
    feature = "envtest",
    expect(
        clippy::expect_used,
        clippy::panic,
        reason = "test helpers: a panic is a failed test, with its message"
    )
)]
#![cfg(feature = "envtest")]

mod envtest_support;

use envtest_support::EnvTest;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::ResourceExt;
use kube::api::{Api, DeleteParams, Patch, PatchParams, PostParams};
use serde_json::{Value, json};

macro_rules! envtest_or_skip {
    () => {
        match EnvTest::try_start().await {
            Some(env_test) => env_test,
            None => return,
        }
    };
}

fn shipped_crd() -> CustomResourceDefinition {
    let yaml = include_str!("../../../../deploy/crds.yaml");
    serde_yaml_ng::from_str(yaml).expect("the generated CRD should deserialize")
}

async fn install_crd(client: kube::Client) {
    let crds: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crd = shipped_crd();
    crds.patch(
        &crd.name_any(),
        &PatchParams::apply("envtest").force(),
        &Patch::Apply(&crd),
    )
    .await
    .expect("the generated CRD should be accepted");

    for _ in 0..60 {
        if let Ok(crd) = crds.get("proxykubeapis.weebo.si.rs").await
            && crd
                .status
                .and_then(|status| status.conditions)
                .is_some_and(|conditions| {
                    conditions
                        .iter()
                        .any(|c| c.type_ == "Established" && c.status == "True")
                })
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    panic!("the CRD never became established");
}

type DynamicApi = Api<kube::api::DynamicObject>;

fn proxies(client: kube::Client) -> DynamicApi {
    let gvk = kube::api::GroupVersionKind::gvk("weebo.si.rs", "v1", "ProxyKubeApi");
    let resource = kube::api::ApiResource::from_gvk_with_plural(&gvk, "proxykubeapis");
    Api::namespaced_with(client, "default", &resource)
}

/// A valid `ServiceAccountToken` bootstrap; tests change one field.
fn bootstrap() -> Value {
    json!({
        "subject": "system:serviceaccount:proxyauth-agent:proxyauth-agent",
        "jwks_from": { "ConfigMap": { "name": "prod-sa-jwks", "key": "jwks.json" } },
    })
}

fn resource(name: &str, agent: Value, cert: Value) -> Value {
    json!({
        "apiVersion": "weebo.si.rs/v1",
        "kind": "ProxyKubeApi",
        "metadata": { "name": name, "namespace": "default" },
        "spec": {
            "enabled": true,
            "cert": cert,
            "service": { "Agent": agent },
        },
    })
}

fn private_ca() -> Value {
    json!({ "ConfigMap": { "name": "prod-ca", "key": "ca.crt" } })
}

fn agent(bootstrap: Value) -> Value {
    json!({ "bootstrap": { "ServiceAccountToken": bootstrap } })
}

async fn create(api: &DynamicApi, body: Value) -> Result<kube::api::DynamicObject, kube::Error> {
    api.create(
        &PostParams::default(),
        &serde_json::from_value(body).expect("resource should deserialize"),
    )
    .await
}

async fn rejected(api: &DynamicApi, body: Value) -> String {
    create(api, body)
        .await
        .expect_err("the apiserver should refuse this agent")
        .to_string()
}

#[tokio::test]
async fn a_minimal_agent_is_admitted_with_its_defaults() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let created = create(
        &api,
        resource("agent-minimal", agent(bootstrap()), private_ca()),
    )
    .await
    .expect("a minimal agent should be admitted");

    let service = &created.data["spec"]["service"]["Agent"];
    assert_eq!(service["server_name"], "kubernetes.default.svc");
    assert_eq!(service["epoch"], 0);
    assert_eq!(
        service["bootstrap"]["ServiceAccountToken"]["audience"],
        "proxyauth"
    );
    let _cleanup = api.delete("agent-minimal", &DeleteParams::default()).await;
}

/// T4: the apiserver's CA comes from the admin, never from public roots an
/// agent could present a certificate for.
#[tokio::test]
async fn system_roots_are_refused_for_an_agent() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let err = rejected(
        &api,
        resource(
            "agent-system-roots",
            agent(bootstrap()),
            json!({ "SystemRoots": true }),
        ),
    )
    .await;
    assert!(err.contains("SystemRoots"), "{err}");
}

#[tokio::test]
async fn exactly_one_jwks_source_is_admitted() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut none = bootstrap();
    none.as_object_mut().expect("an object").remove("jwks_from");
    let mut two = bootstrap();
    two["jwks_url"] = json!("https://issuer.example.com/keys");

    for (name, body) in [("agent-no-jwks", none), ("agent-two-jwks", two)] {
        let err = rejected(&api, resource(name, agent(body), private_ca())).await;
        assert!(err.contains("exactly one of jwks"), "{name}: {err}");
    }
}

#[tokio::test]
async fn a_plain_http_jwks_url_is_refused() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let body = json!({
        "subject": "system:serviceaccount:a:b",
        "jwks_url": "http://issuer.example.com/keys",
    });
    let err = rejected(&api, resource("agent-http-jwks", agent(body), private_ca())).await;
    assert!(err.contains("https://"), "{err}");
}

#[tokio::test]
async fn only_a_service_account_is_admitted_as_the_subject() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut body = bootstrap();
    body["subject"] = json!("alice");
    let err = rejected(
        &api,
        resource("agent-user-subject", agent(body), private_ca()),
    )
    .await;
    assert!(err.contains("ServiceAccount"), "{err}");
}

#[tokio::test]
async fn a_server_name_is_a_dns_name_or_an_ip() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    for (name, server_name) in [("agent-ip", "10.43.0.1"), ("agent-ipv6", "fd00::1")] {
        let mut service = agent(bootstrap());
        service["server_name"] = json!(server_name);
        create(&api, resource(name, service, private_ca()))
            .await
            .unwrap_or_else(|err| panic!("{server_name} should be admitted: {err}"));
        let _cleanup = api.delete(name, &DeleteParams::default()).await;
    }

    let mut service = agent(bootstrap());
    service["server_name"] = json!("https://kubernetes.default.svc");
    let err = rejected(&api, resource("agent-url-name", service, private_ca())).await;
    assert!(err.contains("server_name"), "{err}");
}

/// T8: lowering `epoch` would make revoked agent certificates valid again.
#[tokio::test]
async fn the_epoch_never_decreases() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut service = agent(bootstrap());
    service["epoch"] = json!(2);
    create(&api, resource("agent-epoch", service, private_ca()))
        .await
        .expect("an agent at epoch 2 should be admitted");

    let set_epoch = |epoch: u64| {
        Patch::Merge(json!({ "spec": { "service": { "Agent": { "epoch": epoch } } } }))
    };
    api.patch("agent-epoch", &PatchParams::default(), &set_epoch(3))
        .await
        .expect("raising the epoch should be admitted");
    let err = api
        .patch("agent-epoch", &PatchParams::default(), &set_epoch(1))
        .await
        .expect_err("lowering the epoch should be refused")
        .to_string();
    assert!(err.contains("epoch must not decrease"), "{err}");

    let _cleanup = api.delete("agent-epoch", &DeleteParams::default()).await;
}

/// Decision 27: a per-cluster `streams_per_tunnel` is a positive count; the
/// cap at the gateway's maximum is the controller's advisory, not admission's.
#[tokio::test]
async fn a_streams_per_tunnel_override_is_positive() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut service = agent(bootstrap());
    service["limits"] = json!({ "streams_per_tunnel": 384 });
    create(&api, resource("agent-limits", service, private_ca()))
        .await
        .expect("a positive override should be admitted");
    let _cleanup = api.delete("agent-limits", &DeleteParams::default()).await;

    let mut service = agent(bootstrap());
    service["limits"] = json!({ "streams_per_tunnel": 0 });
    let err = rejected(&api, resource("agent-limits-zero", service, private_ca())).await;
    assert!(err.contains("streams_per_tunnel"), "{err}");
}
