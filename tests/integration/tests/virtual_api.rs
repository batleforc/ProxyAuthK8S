//! Virtual APIs through real clients: the virtual group has to be
//! discoverable from `/apis` in the format kubectl and oc ask for first
//! (aggregated discovery, `apidiscovery.k8s.io/v2`), or they never resolve
//! the resource.

#![expect(
    clippy::panic,
    reason = "test harness: a panic is a failed test, with its message"
)]

use std::time::Duration;

use integration::{
    Bench,
    fixture::{self, CLUSTER, Proxied},
};
use serde_json::{Value, json};

/// The fixture with no restriction.
async fn unrestricted(test: &str) -> Proxied {
    fixture::proxied(test, json!({ "enabled": true, "allowed_resources": [] })).await
}

/// Enable `virtual_api` (an `OpenShiftProject` configuration) on the fixture's
/// cluster, and wait until the proxy serves its group.
async fn enable_projects(p: &Proxied, virtual_api: Value) {
    let patch = json!({ "spec": { "virtual_apis": [virtual_api] } });
    let out = Bench::get().admin(&[
        "-n",
        &p.ns.name,
        "patch",
        &format!("proxykubeapi/{CLUSTER}"),
        "--type=merge",
        "-p",
        &patch.to_string(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The group's own discovery is the proxy's: it answers once the
    // controller has copied the patched spec into Redis.
    for _ in 0..30 {
        let group = p
            .reader
            .kubectl(&["get", "--raw", "/apis/project.openshift.io/v1"])
            .await;
        if group.success() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    panic!("the virtual group was never served");
}

#[tokio::test]
async fn kubectl_resolves_a_virtual_resource() {
    let p = unrestricted("virtual-api").await;
    enable_projects(&p, json!({ "kind": "OpenShiftProject" })).await;
    let ns = p.ns.name.as_str();

    // The merge must not hide the cluster's own groups either.
    let deployments = p.reader.kubectl(&["get", "deployments", "-n", ns]).await;
    assert!(
        deployments.success(),
        "kubectl get deployments: {deployments:?}"
    );

    // A single Project, the caller's namespace: its RoleBinding allows the
    // read (listing every namespace needs the list fallback, below).
    let project = p
        .reader
        .kubectl(&[
            "get",
            "project",
            ns,
            "-o",
            "jsonpath={.kind}/{.metadata.name}",
        ])
        .await;
    assert!(project.success(), "kubectl get project: {project:?}");
    assert_eq!(
        project.stdout.trim(),
        format!("Project/{ns}"),
        "{project:?}"
    );
}

/// A token that may list namespaces and nothing else, in a Secret of `ns`:
/// the list fallback's least-privilege credential.
fn namespace_lister(ns: &str) -> &'static str {
    const SECRET: &str = "ns-lister-token";
    let role = format!("{ns}-ns-lister");
    let items = json!({
        "apiVersion": "v1",
        "kind": "List",
        "items": [
            { "apiVersion": "v1", "kind": "ServiceAccount",
              "metadata": { "name": "ns-lister", "namespace": ns } },
            { "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
              "metadata": { "name": role },
              "rules": [{ "apiGroups": [""], "resources": ["namespaces"], "verbs": ["list"] }] },
            { "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRoleBinding",
              "metadata": { "name": role },
              "roleRef": { "apiGroup": "rbac.authorization.k8s.io",
                           "kind": "ClusterRole", "name": role },
              "subjects": [{ "kind": "ServiceAccount", "name": "ns-lister", "namespace": ns }] },
            { "apiVersion": "v1", "kind": "Secret", "type": "kubernetes.io/service-account-token",
              "metadata": { "name": SECRET, "namespace": ns,
                            "annotations": { "kubernetes.io/service-account.name": "ns-lister" } } },
        ],
    });
    let bench = Bench::get();
    bench.apply(&items.to_string());
    // The token controller fills the Secret asynchronously.
    for _ in 0..30 {
        let out = bench.admin(&[
            "-n",
            ns,
            "get",
            "secret",
            SECRET,
            "-o",
            "jsonpath={.data.token}",
        ]);
        if !out.stdout.is_empty() {
            return SECRET;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    panic!("the service account token was never issued");
}

/// `oc projects` for a caller who may not list namespaces: the list fallback
/// enumerates them with its own token and keeps only those the caller can
/// read, so the caller's namespace is listed and `kube-system` is not.
#[tokio::test]
async fn oc_projects_lists_only_the_callers_projects() {
    let p = unrestricted("oc-projects").await;
    let ns = p.ns.name.as_str();
    let secret = namespace_lister(ns);
    enable_projects(
        &p,
        json!({
            "kind": "OpenShiftProject",
            "list_fallback_token": { "Secret": {
                "name": secret, "key": "token", "namespace": ns } },
        }),
    )
    .await;

    // Without the fallback the apiserver refuses this list to the caller.
    let direct = p.reader.kubectl(&["get", "namespaces"]).await;
    assert!(
        !direct.success(),
        "the caller can list namespaces: {direct:?}"
    );

    let projects = p.reader.oc(&["projects"]).await;
    assert!(projects.success(), "oc projects: {projects:?}");
    assert!(
        projects.stdout.contains(ns),
        "the caller's project is missing: {projects:?}"
    );
    assert!(
        !projects.stdout.contains("kube-system"),
        "a project the caller cannot read is listed: {projects:?}"
    );
}
