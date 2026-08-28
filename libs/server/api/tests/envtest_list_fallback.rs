//! `LIST projects` filtering against a real apiserver's real RBAC decisions.
//!
//! The wiremock-based tests in `virtual_api.rs` prove the fallback's control
//! flow against a hand-written mock body; this proves the same code against a
//! real `kube-apiserver` evaluating real `ClusterRole`/`ClusterRoleBinding`
//! objects, so a divergence between the real `SelfSubjectAccessReview`/
//! `NamespaceList` response shape and what the mocks assume cannot hide here.
//!
//! Needs both the envtest binaries (`task envtest:setup`) and a reachable
//! Redis — gated behind the `envtest` feature like `envtest.rs`, and skipped
//! (unless `REQUIRE_ENVTEST`/`REQUIRE_TEST_REDIS` are set) exactly like the
//! other tiers when either is unavailable.

#![cfg(feature = "envtest")]

mod envtest_support;
mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::redirect;
use base64::{Engine, prelude::BASE64_STANDARD};
use crd::certificate::CertSource;
use crd::service::Service;
use crd::virtual_api::VirtualApiKind;
use envtest_support::{EnvTest, EnvTestOptions, ExtraToken};
use harness::{
    delete_proxy, proxy_fixture, seed_proxy, test_state, try_redis_pool, unique_cluster,
};
use k8s_openapi::api::core::v1::Namespace;
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding, PolicyRule, RoleRef, Subject};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, PostParams};
use serde_json::Value;

macro_rules! proxy_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new().app_data(web::Data::new($state)).service(
                web::scope("/clusters")
                    .service(redirect::get_redirect)
                    .service(redirect::post_redirect)
                    .service(redirect::delete_redirect),
            ),
        )
        .await
    };
}

const LISTER_TOKEN: &str = "envtest-lister-token";
const RESTRICTED_TOKEN: &str = "envtest-restricted-token";

async fn create_namespace(client: kube::Client, name: &str) {
    let namespaces: Api<Namespace> = Api::all(client);
    let namespace = Namespace {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    namespaces
        .create(&PostParams::default(), &namespace)
        .await
        .unwrap_or_else(|err| panic!("namespace {name} should be created: {err}"));
}

/// Grant `verbs` on `resource` (core group) to `username`, scoped to
/// `resource_names` when non-empty (empty means unrestricted).
async fn grant(
    client: kube::Client,
    role_name: &str,
    username: &str,
    resource: &str,
    verbs: &[&str],
    resource_names: &[&str],
) {
    let cluster_roles: Api<ClusterRole> = Api::all(client.clone());
    let bindings: Api<ClusterRoleBinding> = Api::all(client);

    let rule = PolicyRule {
        api_groups: Some(vec![String::new()]),
        resources: Some(vec![resource.to_string()]),
        verbs: verbs.iter().map(|v| (*v).to_string()).collect(),
        resource_names: if resource_names.is_empty() {
            None
        } else {
            Some(resource_names.iter().map(|n| (*n).to_string()).collect())
        },
        ..Default::default()
    };
    let cluster_role = ClusterRole {
        metadata: ObjectMeta {
            name: Some(role_name.to_string()),
            ..Default::default()
        },
        rules: Some(vec![rule]),
        ..Default::default()
    };
    cluster_roles
        .create(&PostParams::default(), &cluster_role)
        .await
        .unwrap_or_else(|err| panic!("ClusterRole {role_name} should be created: {err}"));

    let binding = ClusterRoleBinding {
        metadata: ObjectMeta {
            name: Some(role_name.to_string()),
            ..Default::default()
        },
        role_ref: RoleRef {
            api_group: "rbac.authorization.k8s.io".to_string(),
            kind: "ClusterRole".to_string(),
            name: role_name.to_string(),
        },
        subjects: Some(vec![Subject {
            kind: "User".to_string(),
            name: username.to_string(),
            ..Default::default()
        }]),
    };
    bindings
        .create(&PostParams::default(), &binding)
        .await
        .unwrap_or_else(|err| panic!("ClusterRoleBinding {role_name} should be created: {err}"));
}

/// `LIST projects` filtered against real RBAC: `restricted-user` can `get`
/// only `visible-ns`, so that is the only `Project` the proxy must return —
/// not `hidden-ns`, and not a 403 (real OpenShift never 403s a caller who
/// simply can't see everything).
#[tokio::test]
async fn list_fallback_filters_to_real_rbac_visibility() {
    let Some(pool) = try_redis_pool().await else {
        return;
    };
    let options = EnvTestOptions {
        authorization_mode: "RBAC",
        extra_tokens: &[
            ExtraToken {
                token: LISTER_TOKEN,
                username: "lister",
                uid: "uid-lister",
                groups: "",
            },
            ExtraToken {
                token: RESTRICTED_TOKEN,
                username: "restricted-user",
                uid: "uid-restricted",
                groups: "",
            },
        ],
    };
    let env_test = match EnvTest::try_start_with(options).await {
        Some(env_test) => env_test,
        None => return,
    };
    let admin = env_test.client().expect("admin client should build");

    create_namespace(admin.clone(), "visible-ns").await;
    create_namespace(admin.clone(), "hidden-ns").await;

    // The privileged identity: `list` only, nothing else.
    grant(
        admin.clone(),
        "proxyauthk8s-project-lister",
        "lister",
        "namespaces",
        &["list"],
        &[],
    )
    .await;
    // The caller: `get` on exactly one namespace, and (since RBAC mode has no
    // bootstrap `system:basic-user` binding to lean on here) explicit rights
    // to run the access review itself.
    grant(
        admin.clone(),
        "restricted-user-namespace-getter",
        "restricted-user",
        "namespaces",
        &["get"],
        &["visible-ns"],
    )
    .await;
    grant_selfsubjectaccessreview(admin.clone(), "restricted-user").await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, env_test.url());
    proxy.spec.cert = CertSource::Cert(BASE64_STANDARD.encode(env_test.serving_cert_pem()));
    proxy.spec.service = Service::ExternalService {
        url: env_test.url().to_string(),
    };
    let mut virtual_api =
        crd::virtual_api::VirtualApiConfiguration::new(VirtualApiKind::OpenShiftProject);
    virtual_api.list_fallback_token = Some(CertSource::Cert(BASE64_STANDARD.encode(LISTER_TOKEN)));
    proxy.spec.virtual_apis.push(virtual_api);
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(env_test.url().to_string()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/apis/project.openshift.io/v1/projects"
        ))
        .insert_header(("authorization", format!("Bearer {RESTRICTED_TOKEN}")))
        .to_request();
    let resp = test::call_service(&app, req).await;

    if resp.status() != StatusCode::OK {
        let status = resp.status();
        let body = test::read_body(resp).await;
        panic!("status={status} body={}", String::from_utf8_lossy(&body));
    }
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["kind"], "ProjectList");
    let names: Vec<&str> = body["items"]
        .as_array()
        .expect("items should be a list")
        .iter()
        .filter_map(|item| item["metadata"]["name"].as_str())
        .collect();
    assert_eq!(names, vec!["visible-ns"]);

    delete_proxy(&pool, &ns, &cluster).await;
}

/// `RBAC` mode has no `AlwaysAllow` shortcut, and this envtest instance is not
/// guaranteed to have the `system:basic-user` bootstrap binding settled by the
/// time the test runs — grant it explicitly so the test does not depend on
/// that timing.
async fn grant_selfsubjectaccessreview(client: kube::Client, username: &str) {
    let cluster_roles: Api<ClusterRole> = Api::all(client.clone());
    let bindings: Api<ClusterRoleBinding> = Api::all(client);

    let role_name = format!("{username}-can-review");
    let rule = PolicyRule {
        api_groups: Some(vec!["authorization.k8s.io".to_string()]),
        resources: Some(vec!["selfsubjectaccessreviews".to_string()]),
        verbs: vec!["create".to_string()],
        ..Default::default()
    };
    let cluster_role = ClusterRole {
        metadata: ObjectMeta {
            name: Some(role_name.clone()),
            ..Default::default()
        },
        rules: Some(vec![rule]),
        ..Default::default()
    };
    cluster_roles
        .create(&PostParams::default(), &cluster_role)
        .await
        .unwrap_or_else(|err| panic!("ClusterRole {role_name} should be created: {err}"));

    let binding = ClusterRoleBinding {
        metadata: ObjectMeta {
            name: Some(role_name.clone()),
            ..Default::default()
        },
        role_ref: RoleRef {
            api_group: "rbac.authorization.k8s.io".to_string(),
            kind: "ClusterRole".to_string(),
            name: role_name.clone(),
        },
        subjects: Some(vec![Subject {
            kind: "User".to_string(),
            name: username.to_string(),
            ..Default::default()
        }]),
    };
    bindings
        .create(&PostParams::default(), &binding)
        .await
        .unwrap_or_else(|err| panic!("ClusterRoleBinding {role_name} should be created: {err}"));
}
