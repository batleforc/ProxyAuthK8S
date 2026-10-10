//! The `oidc_provider.config_from` path, against a real ephemeral kube-apiserver.
//!
//! What a wiremock cannot fake, and why this tier is the right one for it:
//!
//! - the relaxed CEL rule (`has(self.config_from)` standing in for a non-empty
//!   inline `issuer_url`/`client_id`) must actually compile and admit the shape
//!   operators will write;
//! - `OidcProvider::resolve` reads a real Secret through a real client, so the
//!   `data`/`stringData` handling and the namespace policy are exercised against
//!   apiserver semantics rather than a hand-rolled fixture.
//!
//! Gated behind the `envtest` feature so a plain `cargo test` needs no binaries.

#![cfg(feature = "envtest")]

mod envtest_support;

use std::collections::BTreeMap;

use crd::authentication_configuration::{OidcConfigError, OidcConfigSource, OidcProvider};
use envtest_support::EnvTest;
use k8s_openapi::api::core::v1::{Namespace, Secret};
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

fn shipped_crd() -> CustomResourceDefinition {
    let yaml = include_str!("../../../../deploy/crds.yaml");
    serde_yaml_ng::from_str(yaml).expect("the generated CRD should deserialize")
}

async fn install_crd(client: kube::Client) {
    let crds: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crd = shipped_crd();
    let name = crd.name_any();
    crds.patch(
        &name,
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

/// Create a Secret in `ns` with the given string values.
async fn create_secret(client: kube::Client, ns: &str, name: &str, data: &[(&str, &str)]) {
    let secrets: Api<Secret> = Api::namespaced(client, ns);
    let string_data: BTreeMap<String, String> = data
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect();
    let secret: Secret = serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": { "name": name, "namespace": ns },
        "stringData": string_data,
    }))
    .expect("secret should deserialize");
    let _ = secrets.delete(name, &DeleteParams::default()).await;
    secrets
        .create(&PostParams::default(), &secret)
        .await
        .expect("the secret should be created");
}

async fn create_namespace(client: kube::Client, name: &str) {
    let namespaces: Api<Namespace> = Api::all(client);
    let namespace: Namespace = serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": { "name": name },
    }))
    .expect("namespace should deserialize");
    // Already existing is fine; the suite reuses one apiserver per test.
    let _ = namespaces.create(&PostParams::default(), &namespace).await;
}

/// A provider that carries nothing inline but the reference, which is the shape
/// `config_from` exists to make possible.
fn provider_from(source: OidcConfigSource) -> OidcProvider {
    OidcProvider {
        enabled: true,
        issuer_url: String::new(),
        client_id: String::new(),
        client_secret: None,
        extra_scope: String::new(),
        audience: String::new(),
        accept_authorized_party: false,
        expose_oauth_authorization_server: false,
        config_from: Some(source),
    }
}

fn secret_ref(name: &str, namespace: Option<&str>) -> OidcConfigSource {
    OidcConfigSource::Secret {
        name: name.to_string(),
        namespace: namespace.map(ToString::to_string),
    }
}

#[tokio::test]
async fn a_whole_provider_block_is_read_from_the_secret() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    create_secret(
        client.clone(),
        "default",
        "oidc-whole-block",
        &[
            ("issuer_url", "https://issuer.example.com"),
            ("client_id", "id-from-secret"),
            ("client_secret", "secret-from-secret"),
            ("audience", "audience-from-secret"),
            ("extra_scope", "groups"),
        ],
    )
    .await;

    let resolved = provider_from(secret_ref("oidc-whole-block", None))
        .resolve(client, "default")
        .await
        .expect("the block should resolve");

    assert_eq!(resolved.issuer_url, "https://issuer.example.com");
    assert_eq!(resolved.client_id, "id-from-secret");
    assert_eq!(
        resolved.client_secret.as_deref(),
        Some("secret-from-secret")
    );
    assert_eq!(resolved.audience, "audience-from-secret");
    assert_eq!(resolved.extra_scope, "groups");
}

/// The common shape: everything stays readable in the CR except the credential.
#[tokio::test]
async fn a_partial_secret_leaves_the_inline_fields_alone() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    create_secret(
        client.clone(),
        "default",
        "oidc-credential-only",
        &[("client_secret", "secret-from-secret")],
    )
    .await;

    let mut provider = provider_from(secret_ref("oidc-credential-only", None));
    provider.issuer_url = "https://inline.example.com".to_string();
    provider.client_id = "inline-id".to_string();

    let resolved = provider
        .resolve(client, "default")
        .await
        .expect("the block should resolve");

    assert_eq!(resolved.issuer_url, "https://inline.example.com");
    assert_eq!(resolved.client_id, "inline-id");
    assert_eq!(
        resolved.client_secret.as_deref(),
        Some("secret-from-secret")
    );
}

/// A `config_from` Secret is routinely managed by external-secrets and shared
/// with other consumers, so keys this block does not know must not fail it.
#[tokio::test]
async fn unrelated_keys_in_the_secret_are_ignored() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    create_secret(
        client.clone(),
        "default",
        "oidc-shared-secret",
        &[
            ("client_secret", "secret-from-secret"),
            ("tls.crt", "-----BEGIN CERTIFICATE-----"),
            ("some-other-consumer", "value"),
        ],
    )
    .await;

    let mut provider = provider_from(secret_ref("oidc-shared-secret", None));
    provider.issuer_url = "https://inline.example.com".to_string();
    provider.client_id = "inline-id".to_string();

    let resolved = provider
        .resolve(client, "default")
        .await
        .expect("unrelated keys should not fail the block");

    assert_eq!(
        resolved.client_secret.as_deref(),
        Some("secret-from-secret")
    );
}

/// Almost always a mistyped key name, so it must not silently fall back to the
/// inline block the operator was trying to replace.
#[tokio::test]
async fn a_secret_with_no_recognised_key_is_an_error() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    create_secret(
        client.clone(),
        "default",
        "oidc-typo",
        &[("clientSecret", "camelCase is not a key")],
    )
    .await;

    let mut provider = provider_from(secret_ref("oidc-typo", None));
    provider.issuer_url = "https://inline.example.com".to_string();
    provider.client_id = "inline-id".to_string();

    let err = provider
        .resolve(client, "default")
        .await
        .expect_err("a Secret with no recognised key should fail");
    assert!(
        matches!(err, OidcConfigError::NoRecognisedKey { .. }),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn a_missing_secret_fails_closed() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");

    let mut provider = provider_from(secret_ref("does-not-exist", None));
    provider.issuer_url = "https://inline.example.com".to_string();
    provider.client_id = "inline-id".to_string();

    let err = provider
        .resolve(client, "default")
        .await
        .expect_err("a missing Secret must not fall back to the inline block");
    assert!(
        matches!(err, OidcConfigError::Read { .. }),
        "unexpected error: {err}"
    );
}

/// The reference names another namespace and `PROXYAUTH_ALLOW_CROSS_NS_OIDC` is
/// unset, so the read is pinned back to the resource's own namespace: a tenant
/// who can create a `ProxyKubeApi` cannot use it to read someone else's OAuth
/// client secret.
#[tokio::test]
async fn a_cross_namespace_reference_is_pinned_to_the_resource_namespace() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    create_namespace(client.clone(), "other-tenant").await;
    create_secret(
        client.clone(),
        "other-tenant",
        "oidc-cross-ns",
        &[("client_secret", "the-other-tenants-secret")],
    )
    .await;
    // Same name in the resource's own namespace, with a different value: if the
    // policy were bypassed the assertion below would see the other tenant's.
    create_secret(
        client.clone(),
        "default",
        "oidc-cross-ns",
        &[("client_secret", "our-own-secret")],
    )
    .await;

    let mut provider = provider_from(secret_ref("oidc-cross-ns", Some("other-tenant")));
    provider.issuer_url = "https://inline.example.com".to_string();
    provider.client_id = "inline-id".to_string();

    let resolved = provider
        .resolve(client, "default")
        .await
        .expect("the pinned read should succeed");

    assert_eq!(resolved.client_secret.as_deref(), Some("our-own-secret"));
}

/// The relaxed CEL rule: a block whose `issuer_url`/`client_id` arrive from the
/// Secret must be admitted even though both are empty in the CR.
#[tokio::test]
async fn a_config_from_only_provider_is_admitted() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;

    let api = proxies(client);
    let resource = json!({
        "apiVersion": "weebo.si.rs/v1",
        "kind": "ProxyKubeApi",
        "metadata": { "name": "config-from-only", "namespace": "default" },
        "spec": {
            "enabled": true,
            "cert": { "SystemRoots": true },
            "service": { "ExternalService": { "url": "https://cluster.example.com:6443" } },
            "auth_config": {
                "oidc_provider": {
                    "enabled": true,
                    "config_from": { "Secret": { "name": "oidc-config" } },
                },
                "validate_against": "OidcProvider",
            },
        },
    });

    let created = api
        .create(
            &PostParams::default(),
            &serde_json::from_value(resource).expect("resource should deserialize"),
        )
        .await
        .expect("a config_from-only provider should be admitted");

    assert_eq!(created.name_any(), "config-from-only");
    let _ = api
        .delete("config-from-only", &DeleteParams::default())
        .await;
}

/// The relaxation is scoped to `config_from`: without it, an enabled provider
/// still has to carry `issuer_url` and `client_id` inline.
#[tokio::test]
async fn an_enabled_provider_without_config_from_still_needs_its_fields() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;

    let api = proxies(client);
    let resource = json!({
        "apiVersion": "weebo.si.rs/v1",
        "kind": "ProxyKubeApi",
        "metadata": { "name": "no-config-from", "namespace": "default" },
        "spec": {
            "enabled": true,
            "cert": { "SystemRoots": true },
            "service": { "ExternalService": { "url": "https://cluster.example.com:6443" } },
            "auth_config": {
                "oidc_provider": { "enabled": true },
                "validate_against": "OidcProvider",
            },
        },
    });

    let err = api
        .create(
            &PostParams::default(),
            &serde_json::from_value(resource).expect("resource should deserialize"),
        )
        .await
        .expect_err("an enabled provider with neither inline fields nor config_from is invalid");

    let rendered = err.to_string();
    assert!(
        rendered.contains("issuer_url") && rendered.contains("config_from"),
        "the rejection should name what is missing: {rendered}"
    );
}
