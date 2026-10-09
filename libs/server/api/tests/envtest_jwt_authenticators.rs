//! Admission rules for the `jwt` authenticators, against a real apiserver.
//!
//! These types are the ones an operator is most likely to get wrong — every
//! either/or field was previously modelled as "both required", so a rule could
//! not be written at all. The CEL rules that replaced that must actually compile
//! inside the apiserver's cost budget and reject exactly what
//! `AuthenticationConfiguration::validate` rejects, or the two drift apart and
//! the CR becomes the only source of truth nobody checks.

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

/// A minimal, valid `jwt` authenticator; tests mutate one field to make it invalid.
fn authenticator() -> Value {
    json!({
        "issuer": {
            "url": "https://issuer.example.com",
            "audiences": ["proxyauthk8s"],
        },
        "claim_mappings": {
            "username": { "claim": "sub", "prefix": "oidc:" },
        },
    })
}

fn resource(name: &str, jwt: Value, validate_against: &str) -> Value {
    json!({
        "apiVersion": "weebo.si.rs/v1",
        "kind": "ProxyKubeApi",
        "metadata": { "name": name, "namespace": "default" },
        "spec": {
            "enabled": true,
            "cert": { "Insecure": true },
            "service": { "ExternalService": { "url": "https://cluster.example.com:6443" } },
            "auth_config": {
                "jwt": jwt,
                "oidc_provider": { "enabled": false },
                "validate_against": validate_against,
            },
        },
    })
}

async fn create(api: &DynamicApi, body: Value) -> Result<kube::api::DynamicObject, kube::Error> {
    api.create(
        &PostParams::default(),
        &serde_json::from_value(body).expect("resource should deserialize"),
    )
    .await
}

#[tokio::test]
async fn a_minimal_authenticator_is_accepted() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let created = create(
        &api,
        resource("jwt-minimal", json!([authenticator()]), "JwtAuthenticators"),
    )
    .await
    .expect("a minimal authenticator should be admitted");

    assert_eq!(created.name_any(), "jwt-minimal");
    let _ = api.delete("jwt-minimal", &DeleteParams::default()).await;
}

/// The mode is meaningless without an issuer to trust, and accepting it would
/// mean validating every token against an empty list.
#[tokio::test]
async fn selecting_the_mode_without_an_authenticator_is_rejected() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let err = create(&api, resource("jwt-empty", json!([]), "JwtAuthenticators"))
        .await
        .expect_err("JwtAuthenticators with no authenticator is invalid");
    assert!(
        err.to_string()
            .contains("no jwt authenticator is configured"),
        "the rejection should say what is missing: {err}"
    );
}

/// The either/or that could not be expressed before: supplying both halves, or
/// neither, must be refused rather than silently resolved.
#[tokio::test]
async fn a_claim_validation_rule_must_set_exactly_one_of_claim_or_expression() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    for (name, rule) in [
        (
            "jwt-rule-both",
            json!({ "claim": "hd", "required_value": "example.com",
                    "expression": "claims.hd == 'example.com'", "message": "no" }),
        ),
        (
            "jwt-rule-neither",
            json!({ "required_value": "example.com" }),
        ),
    ] {
        let mut auth = authenticator();
        auth["claim_validation_rules"] = json!([rule]);
        let err = create(&api, resource(name, json!([auth]), "JwtAuthenticators"))
            .await
            .expect_err("exactly one of claim or expression must be set");
        assert!(
            err.to_string()
                .contains("exactly one of claim or expression"),
            "{name}: {err}"
        );
    }
}

/// The positive half of the either/or, and the shape the security docs tell
/// operators to write. Its absence is what let a schema default silently make
/// every CEL claim rule un-appliable while the "rejects the bad shapes" tests
/// stayed green.
#[tokio::test]
async fn an_expression_only_claim_validation_rule_is_accepted() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut auth = authenticator();
    auth["claim_validation_rules"] = json!([{
        "expression": "claims.email.endsWith('@example.com')",
        "message": "only example.com identities may use this cluster",
    }]);

    let created = create(
        &api,
        resource("jwt-expression-rule", json!([auth]), "JwtAuthenticators"),
    )
    .await
    .expect("an expression-only claim validation rule should be admitted");

    assert_eq!(created.name_any(), "jwt-expression-rule");
    let _ = api
        .delete("jwt-expression-rule", &DeleteParams::default())
        .await;
}

/// A rejection nobody can explain is a support ticket, not a security control.
#[tokio::test]
async fn an_expression_rule_without_a_message_is_rejected() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut auth = authenticator();
    auth["claim_validation_rules"] = json!([{ "expression": "claims.hd == 'example.com'" }]);

    let err = create(
        &api,
        resource("jwt-rule-no-message", json!([auth]), "JwtAuthenticators"),
    )
    .await
    .expect_err("an expression rule requires a message");
    assert!(err.to_string().contains("message"), "{err}");
}

#[tokio::test]
async fn a_claim_mapping_must_set_exactly_one_of_claim_or_expression() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut auth = authenticator();
    auth["claim_mappings"]["username"] = json!({ "claim": "sub", "expression": "claims.sub" });

    let err = create(
        &api,
        resource("jwt-mapping-both", json!([auth]), "JwtAuthenticators"),
    )
    .await
    .expect_err("a mapping cannot set both claim and expression");
    assert!(
        err.to_string()
            .contains("exactly one of claim or expression"),
        "{err}"
    );
}

/// An expression builds whatever name it likes, so a prefix beside it is a
/// contradiction rather than a second chance to prefix.
#[tokio::test]
async fn a_prefix_beside_an_expression_is_rejected() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut auth = authenticator();
    auth["claim_mappings"]["username"] = json!({ "prefix": "oidc:", "expression": "claims.sub" });

    let err = create(
        &api,
        resource("jwt-prefix-expression", json!([auth]), "JwtAuthenticators"),
    )
    .await
    .expect_err("prefix may only be set together with claim");
    assert!(err.to_string().contains("prefix"), "{err}");
}

/// Upstream requires `prefix` to be set explicitly (possibly `""`) whenever
/// `claim` is used, so that the collision decision is conscious: an unprefixed
/// external username can equal an existing local one, and the external identity
/// is then authorized as that local user.
#[tokio::test]
async fn a_claim_mapping_without_an_explicit_prefix_is_rejected() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut auth = authenticator();
    auth["claim_mappings"]["username"] = json!({ "claim": "sub" });

    let err = create(
        &api,
        resource("jwt-no-prefix", json!([auth]), "JwtAuthenticators"),
    )
    .await
    .expect_err("prefix is required with claim");
    assert!(err.to_string().contains("prefix is required"), "{err}");

    // An explicit empty prefix is the documented way to opt out.
    let mut auth = authenticator();
    auth["claim_mappings"]["username"] = json!({ "claim": "sub", "prefix": "" });
    let created = create(
        &api,
        resource("jwt-empty-prefix", json!([auth]), "JwtAuthenticators"),
    )
    .await
    .expect("an explicit empty prefix should be admitted");
    assert_eq!(created.name_any(), "jwt-empty-prefix");
    let _ = api
        .delete("jwt-empty-prefix", &DeleteParams::default())
        .await;
}

/// `issuer.url` is constrained to https, but the proxy actually fetches
/// `discovery_url` — and the JWKS it names decides which keys verify tokens.
/// Unchecked, a compliant `url` plus a plaintext or link-local `discovery_url`
/// would point key discovery wherever the CR author liked, from the proxy's own
/// network position.
#[tokio::test]
async fn a_non_https_discovery_url_is_rejected() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut auth = authenticator();
    auth["issuer"]["discovery_url"] = json!("http://169.254.169.254/openid-configuration");

    let err = create(
        &api,
        resource("jwt-http-discovery", json!([auth]), "JwtAuthenticators"),
    )
    .await
    .expect_err("a non-https discovery_url is invalid");
    assert!(err.to_string().contains("discovery_url"), "{err}");
}

/// Without a username mapping a token that validates maps to no user at all.
#[tokio::test]
async fn a_missing_username_mapping_is_rejected() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut auth = authenticator();
    auth["claim_mappings"] = json!({});

    let err = create(
        &api,
        resource("jwt-no-username", json!([auth]), "JwtAuthenticators"),
    )
    .await
    .expect_err("claim_mappings.username is required");
    assert!(err.to_string().contains("username"), "{err}");
}

/// An authenticator that accepts any audience is the audience-confusion hole
/// this area exists to close, so an empty list is refused at admission.
#[tokio::test]
async fn an_authenticator_without_audiences_is_rejected() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut auth = authenticator();
    auth["issuer"]["audiences"] = json!([]);

    let err = create(
        &api,
        resource("jwt-no-audience", json!([auth]), "JwtAuthenticators"),
    )
    .await
    .expect_err("at least one audience is required");
    assert!(err.to_string().contains("audience"), "{err}");
}

#[tokio::test]
async fn a_non_https_issuer_is_rejected() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut auth = authenticator();
    auth["issuer"]["url"] = json!("http://issuer.example.com");

    let err = create(
        &api,
        resource("jwt-http", json!([auth]), "JwtAuthenticators"),
    )
    .await
    .expect_err("an http:// issuer is invalid");
    assert!(err.to_string().contains("https"), "{err}");
}

/// The schema defaults must be applied by the apiserver, not only by serde: an
/// operator who omits them should get the same policy the apiserver itself
/// defaults to.
#[tokio::test]
async fn the_issuer_policy_defaults_are_applied_at_admission() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let created = create(
        &api,
        resource(
            "jwt-defaults",
            json!([authenticator()]),
            "JwtAuthenticators",
        ),
    )
    .await
    .expect("the authenticator should be admitted");

    let issuer = &created.data["spec"]["auth_config"]["jwt"][0]["issuer"];
    assert_eq!(issuer["audience_match_policy"], "MatchAny");
    assert_eq!(issuer["egress_selector"], "controlplane");
    let _ = api.delete("jwt-defaults", &DeleteParams::default()).await;
}

/// The validator selects with `.find()`, so a second entry for the same issuer —
/// a narrower audience, an extra claim rule — would never run while the operator
/// believed it did. Asserted at *admission*, not only in `validate()`: a Rust
/// check without its CEL mirror is exactly the drift this tier exists to catch.
#[tokio::test]
async fn two_authenticators_for_the_same_issuer_are_rejected() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut second = authenticator();
    second["issuer"]["audiences"] = json!(["another-audience"]);

    let err = create(
        &api,
        resource(
            "jwt-duplicate-issuer",
            json!([authenticator(), second]),
            "JwtAuthenticators",
        ),
    )
    .await
    .expect_err("two authenticators for one issuer is invalid");
    assert!(
        err.to_string().contains("same issuer"),
        "the rejection should say what is wrong: {err}"
    );

    // Two *different* issuers remain perfectly valid.
    let mut other = authenticator();
    other["issuer"]["url"] = json!("https://other-issuer.example.com");
    let created = create(
        &api,
        resource(
            "jwt-two-issuers",
            json!([authenticator(), other]),
            "JwtAuthenticators",
        ),
    )
    .await
    .expect("two distinct issuers should be admitted");
    assert_eq!(created.name_any(), "jwt-two-issuers");
    let _ = api
        .delete("jwt-two-issuers", &DeleteParams::default())
        .await;
}

/// Rules are checked whenever they are present, not only when the mode is
/// selected: a wrong rule should be rejected when it is written, not the day
/// someone flips `validate_against`.
#[tokio::test]
async fn an_invalid_authenticator_is_rejected_even_when_the_mode_is_not_selected() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;
    let api = proxies(client);

    let mut auth = authenticator();
    auth["issuer"]["url"] = json!("http://issuer.example.com");

    let err = create(
        &api,
        resource("jwt-unselected", json!([auth]), "Kubernetes"),
    )
    .await
    .expect_err("an invalid authenticator is invalid whether or not it is selected");
    assert!(err.to_string().contains("https"), "{err}");
}
