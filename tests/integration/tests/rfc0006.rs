//! RFC 0006 §6.P: Kubernetes verbs computed like the apiserver does, against
//! a real cluster. Configuration of RFC 0006 §1 in the test's namespace: pods
//! read-only, `create` on `pods/*/exec` (the RFC 0004 `exec` rule that implies
//! it is not written yet), and a `Crd` `ConfigMap` rule without `verbs`. The
//! apiserver lets `it-readers` do anything there: a refusal is the proxy's.

use std::time::Duration;

use integration::{
    Bench,
    fixture::{self, CLUSTER, Proxied},
};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

fn verbs_config(ns: &str, extra: &[Value]) -> Value {
    let mut rules = fixture::discovery_rules();
    rules.push(json!({ "Path": {
        "path": format!("/api/v1/namespaces/{ns}/pods/**"),
        "parametised": true,
        "verbs": ["get", "list", "watch"],
    } }));
    rules.push(json!({ "Path": {
        "path": format!("/api/v1/namespaces/{ns}/pods/*/exec"),
        "parametised": true,
        "verbs": ["create"],
    } }));
    rules.push(json!({ "Crd": {
        "group": "", "version": "v1", "kind": "ConfigMap", "plural": "configmaps",
        "namespace": { "enabled": true, "rule_kind": { "AllowedNamespaces": [ns] } },
    } }));
    rules.extend_from_slice(extra);
    json!({ "enabled": true, "allowed_resources": rules })
}

async fn fixture_for(test: &str, extra: &[Value]) -> Proxied {
    let ns = integration::bench::namespace_name(test);
    fixture::proxied(test, verbs_config(&ns, extra)).await
}

async fn raw(p: &Proxied, method: Method, path: &str) -> reqwest::RequestBuilder {
    let token = p.reader.cluster_token(&p.ns.name, CLUSTER).await;
    Bench::get().proxied(method, &p.ns.name, CLUSTER, path, &token)
}

fn pod_exists(ns: &str, name: &str) -> bool {
    Bench::get()
        .admin(&["-n", ns, "get", "pod", name])
        .status
        .success()
}

/// Point 1: reads pass, a watch included.
#[tokio::test]
async fn p1_reads_and_watches_pass() {
    let p = fixture_for("rfc0006-p1", &[]).await;
    let ns = p.ns.name.as_str();
    let pods = p
        .reader
        .kubectl(&["get", "pods", "-n", ns, "-o", "name"])
        .await;
    assert!(pods.success(), "{pods:?}");
    assert_eq!(pods.stdout.trim(), "pod/web");
    let pod = p
        .reader
        .kubectl(&["get", "pod", "web", "-n", ns, "-o", "yaml"])
        .await;
    assert!(pod.success() && pod.stdout.contains("name: web"), "{pod:?}");
    let watch = p
        .reader
        .kubectl_for(Duration::from_secs(5), &["get", "pods", "-w", "-n", ns])
        .await;
    assert!(watch.stdout.contains("web"), "{watch:?}");
    assert!(!watch.stderr.contains("Forbidden"), "{watch:?}");
}

/// Point 2: `delete` on a read-only rule is refused with the verb, and the
/// pod is still there.
#[tokio::test]
async fn p2_delete_is_refused_with_its_verb() {
    let p = fixture_for("rfc0006-p2", &[]).await;
    let ns = p.ns.name.as_str();
    let out = p
        .reader
        .kubectl(&["delete", "pod", "web", "-n", ns, "--wait=false"])
        .await;
    assert_eq!(out.code, Some(1), "{out:?}");
    assert!(
        out.stderr.contains("Error from server (Forbidden)"),
        "{out:?}"
    );
    assert!(
        out.stderr.contains("verb \"delete\" is not allowed"),
        "{out:?}"
    );
    assert!(pod_exists(ns, "web"));
}

/// Point 3: `kubectl debug` patches `ephemeralcontainers`: refused.
#[tokio::test]
async fn p3_debug_is_refused() {
    let p = fixture_for("rfc0006-p3", &[]).await;
    let ns = p.ns.name.as_str();
    let busybox = fixture::image("busybox");
    let out = p
        .reader
        .kubectl(&["debug", "web", "-n", ns, &format!("--image={busybox}")])
        .await;
    assert_ne!(out.code, Some(0), "{out:?}");
    assert!(
        out.stderr.contains("verb \"patch\" is not allowed"),
        "{out:?}"
    );
    let pod = Bench::get().admin(&[
        "-n",
        ns,
        "get",
        "pod",
        "web",
        "-o",
        "jsonpath={.spec.ephemeralContainers}",
    ]);
    assert!(String::from_utf8_lossy(&pod.stdout).trim().is_empty());
}

/// Point 4: `kubectl run` (`POST pods`, `create`) is refused.
#[tokio::test]
async fn p4_run_is_refused() {
    let p = fixture_for("rfc0006-p4", &[]).await;
    let ns = p.ns.name.as_str();
    let busybox = fixture::image("busybox");
    let out = p
        .reader
        .kubectl(&["run", "x", "-n", ns, &format!("--image={busybox}")])
        .await;
    assert_ne!(out.code, Some(0), "{out:?}");
    assert!(
        out.stderr.contains("verb \"create\" is not allowed"),
        "{out:?}"
    );
    assert!(!pod_exists(ns, "x"));
}

/// Point 5: exec passes on the rule granting `create` on `pods/*/exec`.
#[tokio::test]
async fn p5_exec_passes_on_its_rule() {
    let p = fixture_for("rfc0006-p5", &[]).await;
    let out = p
        .reader
        .kubectl(&[
            "exec", "web", "-n", &p.ns.name, "-c", "app", "--", "ls", "/",
        ])
        .await;
    assert!(out.success(), "{out:?}");
    assert!(out.stdout.lines().any(|l| l == "bin"), "{out:?}");
}

/// Point 6: a rule without `verbs` grants reads only once verbs restrict the
/// user: `get configmap` passes, `delete configmap` is refused.
#[tokio::test]
async fn p6_a_rule_without_verbs_is_read_only() {
    let p = fixture_for("rfc0006-p6", &[]).await;
    let ns = p.ns.name.as_str();
    let get = p
        .reader
        .kubectl(&["get", "configmap", "app", "-n", ns])
        .await;
    assert!(get.success(), "{get:?}");
    let delete = p
        .reader
        .kubectl(&["delete", "configmap", "app", "-n", ns])
        .await;
    assert_ne!(delete.code, Some(0), "{delete:?}");
    assert!(delete.stderr.contains("is not allowed"), "{delete:?}");
    let still = Bench::get().admin(&["-n", ns, "get", "configmap", "app"]);
    assert!(still.status.success(), "the configmap was deleted");
}

/// Point 7: under a verb restriction alone, the kubelet's exec through
/// `nodes/proxy` is refused, even with a broad `nodes/**` rule.
#[tokio::test]
async fn p7_kubelet_exec_through_nodes_proxy_is_refused() {
    let broad = json!({ "Path": { "path": "/api/v1/nodes/**", "parametised": true } });
    let p = fixture_for("rfc0006-p7", &[broad]).await;
    fixture::allow_nodes_proxy();
    let path = format!(
        "/api/v1/nodes/{}/proxy/exec/{}/web/app?command=sh",
        fixture::node(),
        p.ns.name
    );
    let response = raw(&p, Method::GET, &path)
        .await
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("sec-websocket-protocol", "v5.channel.k8s.io")
        .send()
        .await
        .expect("proxied request");
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

/// Point 8: `OPTIONS` is refused and never relayed. The proxy routes only
/// the methods that carry a Kubernetes verb, so it is refused before the verb
/// check, with a 404 rather than the 403 RFC 0006 wrote (RFC 0006 §17).
#[tokio::test]
async fn p8_options_is_refused() {
    let p = fixture_for("rfc0006-p8", &[]).await;
    let path = format!("/api/v1/namespaces/{}/pods", p.ns.name);
    let response = raw(&p, Method::OPTIONS, &path)
        .await
        .send()
        .await
        .expect("proxied request");
    let status = response.status();
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "OPTIONS answered {status}"
    );
    let relayed = integration::observe::audit_lines(&p.ns.name)
        .into_iter()
        .any(|e| e["http_method"] == "OPTIONS");
    assert!(!relayed, "an OPTIONS request reached the proxy handler");
}

/// Point 9: `X-HTTP-Method-Override: GET` does not turn a `POST` into a read:
/// the request is a `create`, refused, and no pod is created.
#[tokio::test]
async fn p9_method_override_does_not_change_the_verb() {
    let p = fixture_for("rfc0006-p9", &[]).await;
    let ns = p.ns.name.as_str();
    let busybox = fixture::image("busybox");
    let pod = json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": "override" },
        "spec": { "containers": [{ "name": "c", "image": busybox }] },
    });
    let response = raw(&p, Method::POST, &format!("/api/v1/namespaces/{ns}/pods"))
        .await
        .header("x-http-method-override", "GET")
        .json(&pod)
        .send()
        .await
        .expect("proxied request");
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let message: Value = serde_json::from_str(&body).expect("a Kubernetes Status");
    assert!(
        message["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("verb \"create\" is not allowed")),
        "{body}"
    );
    assert!(!pod_exists(ns, "override"));
}
