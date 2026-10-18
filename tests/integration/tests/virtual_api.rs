//! Virtual APIs through a real kubectl: the virtual group has to be
//! discoverable from `/apis` in the format kubectl asks for first (aggregated
//! discovery, `apidiscovery.k8s.io/v2`), or kubectl never resolves the
//! resource.

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
    // read (listing every namespace needs the list fallback).
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
