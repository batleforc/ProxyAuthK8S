//! RFC 0014 §6.P, phase 1: the `upstream_auth` guard and
//! `oidc_provider.claim_mappings`, through the proxy, with the real kubectl
//! and the bench's Dex.

#![expect(
    clippy::indexing_slicing,
    reason = "test harness: a panic is a failed test, with its message"
)]

use integration::{Bench, fixture, observe};
use serde_json::{Value, json};

/// `kubectl patch --type merge` of a cluster, then wait for the controller
/// to reconcile it: `ready_reason` is the reason `Ready` must end on.
fn patch_cluster(ns: &str, cluster: &str, spec: &Value, ready_reason: &str) {
    let bench = Bench::get();
    let patch = json!({ "spec": spec }).to_string();
    let out = bench.admin(&[
        "-n",
        ns,
        "patch",
        &format!("proxykubeapi/{cluster}"),
        "--type=merge",
        "-p",
        &patch,
    ]);
    assert!(
        out.status.success(),
        "patch: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    fixture::wait(
        ns,
        &[
            &format!("proxykubeapi/{cluster}"),
            &format!(
                "--for=jsonpath={{.status.conditions[?(@.type==\"Ready\")].reason}}={ready_reason}"
            ),
        ],
    );
    if ready_reason == "Reconciled" {
        // The Redis copy the proxy routes from follows the status.
        let generation = bench.admin(&[
            "-n",
            ns,
            "get",
            &format!("proxykubeapi/{cluster}"),
            "-o",
            "jsonpath={.metadata.generation}",
        ]);
        let generation = String::from_utf8_lossy(&generation.stdout).to_string();
        fixture::wait(
            ns,
            &[
                &format!("proxykubeapi/{cluster}"),
                &format!("--for=jsonpath={{.status.observedGeneration}}={generation}"),
            ],
        );
    }
}

/// The `auth_config` of the fixture, validated against Dex's `/userinfo`
/// through `mappings`.
fn oidc_mapped(mappings: &Value) -> Value {
    let bench = Bench::get();
    json!({
        "validate_against": "OidcProvider",
        "oidc_provider": {
            "enabled": true,
            "issuer_url": bench.dex_url(),
            "client_id": "kube_login",
            "extra_scope": "openid profile email groups",
            "claim_mappings": mappings,
        },
    })
}

/// The `user` of the `request` audit lines for `ns/cluster`.
fn audited_users(ns: &str, cluster: &str) -> Vec<String> {
    observe::audit_lines(ns)
        .into_iter()
        .filter(|line| line["event"] == "request" && line["ns"] == ns && line["cluster"] == cluster)
        .filter_map(|line| line["user"].as_str().map(str::to_string))
        .collect()
}

/// Point 1: a cluster in `Passthrough` whose `oidc_provider` maps the
/// username from `email` — kubectl reaches the pods, and the proxy's audit
/// names the user by the mapped claim, with `upstream_mode: Passthrough`.
#[tokio::test]
async fn p1_claim_mappings_name_the_user_in_passthrough() {
    let p = fixture::proxied("rfc0014-p1", json!(null)).await;
    let ns = p.ns.name.clone();
    patch_cluster(
        &ns,
        fixture::CLUSTER,
        &json!({ "auth_config": oidc_mapped(&json!({
            "username": { "claim": "email", "prefix": "" },
            "groups": { "claim": "groups", "prefix": "" },
        })) }),
        "Reconciled",
    );
    let out = p.reader.kubectl(&["-n", &ns, "get", "pods"]).await;
    assert!(out.success(), "kubectl get pods: {out:?}");
    let lines: Vec<Value> = observe::audit_lines(&ns)
        .into_iter()
        .filter(|line| line["event"] == "request" && line["outcome"] == "allowed")
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line["user"] == "reader@it.local" && line["upstream_mode"] == "Passthrough"),
        "no allowed request audited as reader@it.local in Passthrough: {lines:?}"
    );
}

/// Point 2 (refusal): a cluster asking for `FrontProxy` on an installation
/// whose `allowedModes` is the default — kubectl gets the proxy's `503`
/// with its message, and the Condition says why. The message is the
/// proxy's own: nothing was relayed, the apiserver has no such text.
#[tokio::test]
async fn p2_a_mode_outside_allowed_modes_answers_503_and_is_never_relayed() {
    let p = fixture::proxied("rfc0014-p2", json!(null)).await;
    let ns = p.ns.name.clone();
    patch_cluster(
        &ns,
        fixture::CLUSTER,
        &json!({
            "auth_config": oidc_mapped(&json!({ "username": { "claim": "sub", "prefix": "" } })),
            "upstream_auth": {
                "mode": "FrontProxy",
                "user_prefix": "proxyauth:",
                "group_prefix": "proxyauth:",
            },
        }),
        "UpstreamModeNotAllowed",
    );
    let bench = Bench::get();
    let reason = bench.admin(&[
        "-n",
        &ns,
        "get",
        &format!("proxykubeapi/{}", fixture::CLUSTER),
        "-o",
        "jsonpath={.status.conditions[?(@.type==\"Ready\")].reason}",
    ]);
    assert_eq!(
        String::from_utf8_lossy(&reason.stdout),
        "UpstreamModeNotAllowed"
    );
    let mode = bench.admin(&[
        "-n",
        &ns,
        "get",
        &format!("proxykubeapi/{}", fixture::CLUSTER),
        "-o",
        "jsonpath={.status.upstreamAuth.mode}",
    ]);
    assert_eq!(String::from_utf8_lossy(&mode.stdout), "FrontProxy");

    let token = p.reader.cluster_token(&ns, fixture::CLUSTER).await;
    let response = bench
        .proxied(
            reqwest::Method::GET,
            &ns,
            fixture::CLUSTER,
            &format!("/api/v1/namespaces/{ns}/pods"),
            &token,
        )
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 503);
    let body: Value = response.json().await.expect("a Kubernetes Status");
    let message = body["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("upstream mode FrontProxy is not enabled")
            && message.contains("upstreamAuth.allowedModes"),
        "{body}"
    );
    // The refusal is audited, with the mode that was asked for.
    assert!(
        observe::audit_lines(&ns)
            .iter()
            .any(|line| line["event"] == "request"
                && line["status"] == 503
                && line["upstream_mode"] == "FrontProxy"),
        "no audited 503 for the FrontProxy cluster"
    );
    let out = p.reader.kubectl(&["-n", &ns, "get", "pods"]).await;
    assert!(!out.success(), "kubectl must fail: {out:?}");
    assert!(
        out.stderr.contains("upstreamAuth.allowedModes"),
        "kubectl prints the proxy's message: {out:?}"
    );
}

/// Point 15 (refusal): two clusters on the same Dex client, one mapping the
/// username from `email`, the other mapping everyone to `admin` — the same
/// token sent to both, the second within the cache's lifetime, is audited
/// under each cluster's own mapping (decision 38).
#[tokio::test]
async fn p15_a_mapping_from_one_cluster_never_serves_another() {
    let p = fixture::proxied("rfc0014-p15", json!(null)).await;
    let ns = p.ns.name.clone();
    patch_cluster(
        &ns,
        fixture::CLUSTER,
        &json!({ "auth_config": oidc_mapped(&json!({
            "username": { "claim": "email", "prefix": "" },
            "groups": { "claim": "groups", "prefix": "" },
        })) }),
        "Reconciled",
    );
    let bench = Bench::get();
    let mut other: Value = serde_json::from_slice(
        &bench
            .admin(&[
                "-n",
                &ns,
                "get",
                &format!("proxykubeapi/{}", fixture::CLUSTER),
                "-o",
                "json",
            ])
            .stdout,
    )
    .expect("the fixture's cluster");
    other["metadata"] = json!({ "name": "admin-mapped", "namespace": ns });
    other.as_object_mut().expect("object").remove("status");
    other["spec"]["auth_config"] = oidc_mapped(&json!({
        "username": { "expression": "'admin'" },
        "groups": { "claim": "groups", "prefix": "" },
    }));
    bench.apply(&other.to_string());
    fixture::wait(
        &ns,
        &[
            "proxykubeapi/admin-mapped",
            "--for=jsonpath={.status.exposed}=true",
        ],
    );

    // One token, the one kubectl uses for `c`, sent to both clusters.
    let token = p.reader.cluster_token(&ns, fixture::CLUSTER).await;
    for cluster in ["admin-mapped", fixture::CLUSTER] {
        let response = bench
            .proxied(
                reqwest::Method::GET,
                &ns,
                cluster,
                &format!("/api/v1/namespaces/{ns}/pods"),
                &token,
            )
            .send()
            .await
            .expect("request");
        assert!(
            response.status().is_success(),
            "{cluster}: {}",
            response.status()
        );
    }
    assert!(
        audited_users(&ns, "admin-mapped")
            .iter()
            .any(|user| user == "admin"),
        "the admin-mapped cluster audits admin"
    );
    let users = audited_users(&ns, fixture::CLUSTER);
    assert!(
        users.iter().any(|user| user == "reader@it.local")
            && !users.iter().any(|user| user == "admin"),
        "the email-mapped cluster never sees the other's user: {users:?}"
    );
}
