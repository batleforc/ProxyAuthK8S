//! Lot 7: the envtest tier, against a real ephemeral kube-apiserver.
//!
//! What a wiremock cannot fake, and why this tier exists:
//!
//! - the generated CRD is *actually accepted* — its CEL rules must compile and
//!   stay inside the apiserver's cost budget, which is how two real bugs in the
//!   Lot 5 admission rules were caught;
//! - admission rejects exactly what `validate()` rejects, so the two cannot
//!   drift apart;
//! - defaults declared in the schema are applied by the apiserver, not just by
//!   serde.
//!
//! Gated behind the `envtest` feature so a plain `cargo test` needs no binaries.

#![cfg(feature = "envtest")]

mod envtest_support;

use envtest_support::EnvTest;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::ResourceExt;
use kube::api::{Api, DeleteParams, Patch, PatchParams, PostParams};
use serde_json::json;

macro_rules! envtest_or_skip {
    () => {
        match EnvTest::try_start().await {
            Some(env_test) => env_test,
            None => return,
        }
    };
}

/// The CRD exactly as it is shipped in `deploy/crds.yaml`.
fn shipped_crd() -> CustomResourceDefinition {
    let yaml = include_str!("../../../../deploy/crds.yaml");
    serde_yaml_ng::from_str(yaml).expect("the generated CRD should deserialize")
}

async fn install_crd(client: kube::Client) -> Result<CustomResourceDefinition, kube::Error> {
    let crds: Api<CustomResourceDefinition> = Api::all(client);
    let crd = shipped_crd();
    let name = crd.name_any();
    crds.patch(
        &name,
        &PatchParams::apply("envtest").force(),
        &Patch::Apply(&crd),
    )
    .await
}

/// Wait for the apiserver to start serving the custom resource.
async fn wait_for_crd(client: kube::Client) {
    let crds: Api<CustomResourceDefinition> = Api::all(client);
    for _ in 0..60 {
        if let Ok(crd) = crds.get("proxykubeapis.weebo.si.rs").await {
            let established = crd
                .status
                .and_then(|status| status.conditions)
                .map(|conditions| {
                    conditions
                        .iter()
                        .any(|c| c.type_ == "Established" && c.status == "True")
                })
                .unwrap_or(false);
            if established {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    panic!("the CRD never became established");
}

fn minimal_spec() -> serde_json::Value {
    json!({
        "enabled": true,
        "cert": { "SystemRoots": true },
        "service": { "ExternalService": { "url": "https://cluster.example.com:6443" } },
    })
}

fn proxy_kube_api(name: &str, spec: serde_json::Value) -> serde_json::Value {
    json!({
        "apiVersion": "weebo.si.rs/v1",
        "kind": "ProxyKubeApi",
        "metadata": { "name": name, "namespace": "default" },
        "spec": spec,
    })
}

type DynamicApi = Api<kube::api::DynamicObject>;

fn proxies(client: kube::Client) -> DynamicApi {
    let gvk = kube::api::GroupVersionKind::gvk("weebo.si.rs", "v1", "ProxyKubeApi");
    let resource = kube::api::ApiResource::from_gvk_with_plural(&gvk, "proxykubeapis");
    Api::namespaced_with(client, "default", &resource)
}

/// The headline check: the CRD this repo ships is accepted as-is.
#[tokio::test]
async fn the_generated_crd_is_accepted_by_a_real_apiserver() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");

    let installed = install_crd(client.clone())
        .await
        .expect("the generated CRD should be accepted");

    assert_eq!(installed.spec.group, "weebo.si.rs");
    assert_eq!(installed.spec.names.kind, "ProxyKubeApi");
}

#[tokio::test]
async fn a_valid_resource_is_accepted() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let api = proxies(client);
    let created = api
        .create(
            &PostParams::default(),
            &serde_json::from_value(proxy_kube_api("valid", minimal_spec()))
                .expect("resource should deserialize"),
        )
        .await
        .expect("a valid ProxyKubeApi should be accepted");

    assert_eq!(created.name_any(), "valid");
    let _ = api.delete("valid", &DeleteParams::default()).await;
}

/// The schema default must be applied by the apiserver, not only by serde:
/// a cluster must not become dashboard-visible just because the field was left out.
#[tokio::test]
async fn expose_via_dashboard_defaults_to_false_at_admission() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let api = proxies(client);
    let created = api
        .create(
            &PostParams::default(),
            &serde_json::from_value(proxy_kube_api("defaults", minimal_spec()))
                .expect("resource should deserialize"),
        )
        .await
        .expect("resource should be accepted");

    assert_eq!(created.data["spec"]["expose_via_dashboard"], json!(false));
    assert_eq!(created.data["spec"]["enabled"], json!(true));
    assert_eq!(created.data["spec"]["virtual_apis"], json!([]));

    let _ = api.delete("defaults", &DeleteParams::default()).await;
}

/// The CEL rule mirroring `AuthenticationConfiguration::validate`.
#[tokio::test]
async fn admission_rejects_oidc_validation_without_an_enabled_provider() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let mut spec = minimal_spec();
    spec["auth_config"] = json!({
        "jwt": [],
        "oidc_provider": {
            "enabled": false,
            "issuer_url": "https://issuer.example.com",
            "client_id": "proxyauthk8s",
        },
        "validate_against": "OidcProvider",
    });

    let error = proxies(client)
        .create(
            &PostParams::default(),
            &serde_json::from_value(proxy_kube_api("bad-oidc", spec))
                .expect("resource should deserialize"),
        )
        .await
        .expect_err("admission should reject this resource");

    let message = error.to_string();
    assert!(
        message.contains("OidcProvider"),
        "unexpected error: {message}"
    );
}

/// The CEL rule mirroring `AllowedPathConfiguration::validate`.
#[tokio::test]
async fn admission_rejects_an_unknown_path_placeholder() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let mut spec = minimal_spec();
    spec["security_config"] = json!({
        "enabled": true,
        "allowed_resources": [
            { "Path": { "path": "/api/v1/namespaces/{{tenant}}/pods", "parametised": true } }
        ],
    });

    let error = proxies(client)
        .create(
            &PostParams::default(),
            &serde_json::from_value(proxy_kube_api("bad-placeholder", spec))
                .expect("resource should deserialize"),
        )
        .await
        .expect_err("admission should reject this resource");

    let message = error.to_string();
    assert!(
        message.contains("placeholder"),
        "unexpected error: {message}"
    );
}

/// The same rule must accept the placeholders the proxy does support, or the
/// admission rule and the runtime matcher would disagree.
#[tokio::test]
async fn admission_accepts_the_supported_path_placeholders() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let mut spec = minimal_spec();
    spec["security_config"] = json!({
        "enabled": true,
        "allowed_resources": [
            { "Path": { "path": "/api/v1/namespaces/{{username}}/pods", "parametised": true } },
            { "Path": { "path": "/api/v1/namespaces/{{group}}/pods/**", "parametised": true } },
            { "Path": { "path": "/api/v1/namespaces/dev-*/pods", "parametised": true } },
        ],
    });

    let api = proxies(client);
    api.create(
        &PostParams::default(),
        &serde_json::from_value(proxy_kube_api("good-placeholder", spec))
            .expect("resource should deserialize"),
    )
    .await
    .expect("supported placeholders should be accepted");

    let _ = api
        .delete("good-placeholder", &DeleteParams::default())
        .await;
}

fn port_forward_spec(ports: &[&str]) -> serde_json::Value {
    let mut spec = minimal_spec();
    spec["security_config"] = json!({
        "enabled": true,
        "allowed_resources": [
            { "Path": {
                "path": "/api/v1/namespaces/dev/pods/*/portforward",
                "parametised": true,
                "allowed_ports": ports,
            } }
        ],
    });
    spec
}

/// The schema pattern only bounds the digits; the CEL rule must refuse what
/// `PortSpec::range` refuses, or a typo silently allows no port at runtime.
#[tokio::test]
async fn admission_rejects_out_of_bounds_and_reversed_ports() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let api = proxies(client);
    for (name, port) in [
        ("port-zero", "0"),
        ("port-too-high", "70000"),
        ("port-range-too-high", "8000-65536"),
        ("port-range-reversed", "200-100"),
    ] {
        let error = api
            .create(
                &PostParams::default(),
                &serde_json::from_value(proxy_kube_api(name, port_forward_spec(&[port])))
                    .expect("resource should deserialize"),
            )
            .await
            .expect_err("admission should reject this port");
        let message = error.to_string();
        assert!(
            message.contains("1-65535"),
            "{port}: unexpected error: {message}"
        );
    }
}

#[tokio::test]
async fn admission_accepts_valid_ports_and_ranges() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let api = proxies(client);
    api.create(
        &PostParams::default(),
        &serde_json::from_value(proxy_kube_api(
            "good-ports",
            port_forward_spec(&["1", "8080", "9000-9100", "65535", "443-443"]),
        ))
        .expect("resource should deserialize"),
    )
    .await
    .expect("valid ports should be accepted");

    let _ = api.delete("good-ports", &DeleteParams::default()).await;
}

#[tokio::test]
async fn admission_rejects_a_relative_allowed_path() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let mut spec = minimal_spec();
    spec["security_config"] = json!({
        "enabled": true,
        "allowed_resources": [{ "Path": { "path": "api/v1/pods" } }],
    });

    let error = proxies(client)
        .create(
            &PostParams::default(),
            &serde_json::from_value(proxy_kube_api("relative-path", spec))
                .expect("resource should deserialize"),
        )
        .await
        .expect_err("admission should reject a relative path");

    assert!(
        error.to_string().contains("start with"),
        "unexpected error: {error}"
    );
}

/// The deprecated spelling must stay in the schema: the apiserver prunes fields
/// it does not know, so a serde alias alone would silently drop the security
/// configuration of every resource written before the rename.
#[tokio::test]
async fn the_deprecated_allowed_ressources_spelling_survives_admission() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let mut spec = minimal_spec();
    spec["security_config"] = json!({
        "enabled": true,
        "allowed_ressources": [{ "Path": { "path": "/api/v1/pods" } }],
    });

    let api = proxies(client);
    let created = api
        .create(
            &PostParams::default(),
            &serde_json::from_value(proxy_kube_api("legacy-spelling", spec))
                .expect("resource should deserialize"),
        )
        .await
        .expect("a resource using the old spelling should still be accepted");

    assert_eq!(
        created.data["spec"]["security_config"]["allowed_ressources"][0]["Path"]["path"],
        json!("/api/v1/pods"),
        "the deprecated field must not be pruned"
    );

    let _ = api
        .delete("legacy-spelling", &DeleteParams::default())
        .await;
}

/// The CEL rules apply to the deprecated field too, otherwise the rename would
/// open a way around admission.
#[tokio::test]
async fn admission_checks_the_deprecated_spelling_too() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let mut spec = minimal_spec();
    spec["security_config"] = json!({
        "enabled": true,
        "allowed_ressources": [
            { "Path": { "path": "/api/v1/namespaces/{{tenant}}/pods", "parametised": true } }
        ],
    });

    let error = proxies(client)
        .create(
            &PostParams::default(),
            &serde_json::from_value(proxy_kube_api("legacy-bad-placeholder", spec))
                .expect("resource should deserialize"),
        )
        .await
        .expect_err("admission should reject this resource");

    assert!(
        error.to_string().contains("placeholder"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn admission_rejects_an_empty_proxy_group() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let mut spec = minimal_spec();
    spec["proxy_group"] = json!("");

    let error = proxies(client)
        .create(
            &PostParams::default(),
            &serde_json::from_value(proxy_kube_api("empty-group", spec))
                .expect("resource should deserialize"),
        )
        .await
        .expect_err("admission should reject an empty proxy_group");

    assert!(
        error.to_string().contains("proxy_group"),
        "unexpected error: {error}"
    );
}

/// `virtual_apis.kind` is a closed enum, so a mapper the binary does not
/// implement is refused at admission rather than ignored at runtime.
#[tokio::test]
async fn admission_rejects_an_unknown_virtual_api() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let mut spec = minimal_spec();
    spec["virtual_apis"] = json!([{ "kind": "NotAMapper" }]);

    let error = proxies(client)
        .create(
            &PostParams::default(),
            &serde_json::from_value(proxy_kube_api("bad-virtual-api", spec))
                .expect("resource should deserialize"),
        )
        .await
        .expect_err("admission should reject an unknown virtual API");

    assert!(
        error.to_string().contains("NotAMapper") || error.to_string().contains("supported values"),
        "unexpected error: {error}"
    );
}

/// The status subresource must exist, otherwise the controller cannot report.
#[tokio::test]
async fn the_status_subresource_is_served() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let api = proxies(client);
    api.create(
        &PostParams::default(),
        &serde_json::from_value(proxy_kube_api("with-status", minimal_spec()))
            .expect("resource should deserialize"),
    )
    .await
    .expect("resource should be accepted");

    let patched = api
        .patch_status(
            "with-status",
            &PatchParams::apply("envtest").force(),
            &Patch::Apply(json!({
                "apiVersion": "weebo.si.rs/v1",
                "kind": "ProxyKubeApi",
                "status": { "exposed": true },
            })),
        )
        .await
        .expect("the status subresource should accept a patch");

    assert_eq!(patched.data["status"]["exposed"], json!(true));

    let _ = api.delete("with-status", &DeleteParams::default()).await;
}

/// A finalizer must survive a delete: the object stays until it is removed.
#[tokio::test]
async fn a_finalizer_holds_the_object_until_it_is_removed() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await.expect("CRD install");
    wait_for_crd(client.clone()).await;

    let api = proxies(client);
    let mut resource = proxy_kube_api("with-finalizer", minimal_spec());
    resource["metadata"]["finalizers"] = json!([crd::PROXY_KUBE_FINALIZER]);
    api.create(
        &PostParams::default(),
        &serde_json::from_value(resource).expect("resource should deserialize"),
    )
    .await
    .expect("resource should be accepted");

    api.delete("with-finalizer", &DeleteParams::default())
        .await
        .expect("delete should be accepted");

    let still_there = api
        .get("with-finalizer")
        .await
        .expect("the finalizer should hold the object");
    assert!(
        still_there.metadata.deletion_timestamp.is_some(),
        "the object should be marked for deletion"
    );

    // Removing the finalizer lets the apiserver reap it.
    api.patch(
        "with-finalizer",
        &PatchParams::default(),
        &Patch::Merge(json!({ "metadata": { "finalizers": [] } })),
    )
    .await
    .expect("removing the finalizer should be accepted");

    for _ in 0..40 {
        if api.get("with-finalizer").await.is_err() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the object was never reaped after the finalizer was removed");
}
