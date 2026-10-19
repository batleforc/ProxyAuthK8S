//! RFC 0003 §6.P: port policy on every path that opens a port, against a real
//! kubelet. Configuration of RFC 0003 §1 in the test's namespace: port-forward
//! limited to 8080, a `Crd` `Pod` rule without `allowed_ports`, and a `Crd`
//! `Service` rule allowing port 80.

use integration::{
    Bench, Output,
    fixture::{self, CLUSTER, Proxied},
    observe,
};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use std::time::Duration;

fn restricted(ns: &str, extra: &[Value]) -> Value {
    let mut rules = fixture::discovery_rules();
    rules.push(json!({ "Path": {
        "path": format!("/api/v1/namespaces/{ns}/pods/*/portforward"),
        "parametised": true,
        "allowed_ports": ["8080"],
    } }));
    rules.push(json!({ "Crd": {
        "group": "", "version": "v1", "kind": "Pod", "plural": "pods",
        "namespace": { "enabled": true, "rule_kind": { "AllowedNamespaces": [ns] } },
    } }));
    rules.push(json!({ "Crd": {
        "group": "", "version": "v1", "kind": "Service", "plural": "services",
        "namespace": { "enabled": true, "rule_kind": { "AllowedNamespaces": [ns] } },
        "allowed_ports": ["80"],
    } }));
    rules.extend_from_slice(extra);
    json!({ "enabled": true, "allowed_resources": rules })
}

/// The namespace the test will get, to name it in its own rules.
fn ns_of(test: &str) -> String {
    integration::bench::namespace_name(test)
}

async fn get(p: &Proxied, path: &str) -> (StatusCode, String) {
    let token = p.reader.cluster_token(&p.ns.name, CLUSTER).await;
    let response = Bench::get()
        .proxied(Method::GET, &p.ns.name, CLUSTER, path, &token)
        .send()
        .await
        .expect("proxied request");
    let status = response.status();
    (status, response.text().await.unwrap_or_default())
}

/// The audit events of the port policy for this test's cluster.
fn port_decisions(ns: &str) -> Vec<Value> {
    observe::audit_lines(ns)
        .into_iter()
        .filter(|e| e["event"] == "policy_decision" && e["policy"] == "port")
        .filter(|e| e["ns"] == ns)
        .collect()
}

fn refused_by_proxy(out: &Output, needle: &str) {
    assert_ne!(out.code, Some(0), "{out:?}");
    assert!(out.stderr.contains(needle), "{needle:?} not in {out:?}");
    // The real reason, not the SPDY fallback hiding it (RFC 0003 §6.P 2).
    assert!(!out.stderr.contains("SPDY is not supported"), "{out:?}");
}

/// Point 1: port-forward on the allowed port connects, and the audit says so.
#[tokio::test]
async fn p1_port_forward_on_an_allowed_port_connects() {
    let test = "rfc0003-p1";
    let p = fixture::proxied(test, restricted(&ns_of(test), &[])).await;
    let forward = p
        .reader
        .port_forward(&p.ns.name, "pod/web", 8080)
        .await
        .expect("port-forward 8080");
    // busybox httpd answers through the tunnel: the connection is real.
    let response = reqwest::get(format!("http://127.0.0.1:{}/", forward.local))
        .await
        .expect("HTTP through the port-forward");
    assert!(response.status().as_u16() < 500, "{}", response.status());
    drop(forward);

    let decisions = port_decisions(&p.ns.name);
    assert!(
        decisions
            .iter()
            .any(|e| e["decision"] == "allowed" && e["port"] == 8080),
        "no port_allowed port=8080 in {decisions:?}"
    );
}

/// Point 2: another port is refused, and the audit records the refused port.
/// kubectl 1.37 tunnels SPDY in a websocket: the port travels in each
/// stream's header, so kubectl starts forwarding, and the refusal comes when
/// the first local connection opens a stream. The session is already
/// upgraded then: the proxy can only close it, and kubectl reports a lost
/// connection, not the policy's reason (RFC 0003 §17).
#[tokio::test]
async fn p2_port_forward_on_another_port_is_refused() {
    let test = "rfc0003-p2";
    let p = fixture::proxied(test, restricted(&ns_of(test), &[])).await;
    let refused = match p.reader.port_forward(&p.ns.name, "pod/web", 9090).await {
        // Refused at the upgrade (ports in the URL, SPDY fallback): the reason.
        Err(out) => {
            refused_by_proxy(&out, "port 9090 is not allowed on this cluster");
            out
        }
        Ok(forward) => {
            let reached = reqwest::get(format!("http://127.0.0.1:{}/", forward.local)).await;
            assert!(reached.is_err(), "9090 answered: {reached:?}");
            forward.exit(Duration::from_secs(15)).await
        }
    };
    assert_ne!(refused.code, Some(0), "{refused:?}");
    assert!(
        !refused.stderr.contains("SPDY is not supported"),
        "{refused:?}"
    );
    let decisions = port_decisions(&p.ns.name);
    assert!(
        decisions
            .iter()
            .any(|e| e["decision"] == "denied" && e["port"] == 9090),
        "no port_denied port=9090 in {decisions:?}"
    );
}

/// Point 3: the pod `proxy` subresource is under the policy too: no rule
/// with `allowed_ports` covers it, so no port is granted there.
#[tokio::test]
async fn p3_pod_proxy_on_a_port_is_refused() {
    let test = "rfc0003-p3";
    let p = fixture::proxied(test, restricted(&ns_of(test), &[])).await;
    for port in [9090, 8080] {
        let path = format!("/api/v1/namespaces/{}/pods/web:{port}/proxy/", p.ns.name);
        let (status, body) = get(&p, &path).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
    }
}

/// Point 4: a named Service port is refused under restriction, even on a
/// Service rule that allows the port it names; the numeric port passes.
#[tokio::test]
async fn p4_service_proxy_on_a_named_port_is_refused() {
    let test = "rfc0003-p4";
    let p = fixture::proxied(test, restricted(&ns_of(test), &[])).await;
    let ns = &p.ns.name;
    let (status, body) = get(
        &p,
        &format!("/api/v1/namespaces/{ns}/services/web:http/proxy/"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("named or missing port"), "{body}");
    let (status, body) = get(
        &p,
        &format!("/api/v1/namespaces/{ns}/services/web:80/proxy/"),
    )
    .await;
    assert_ne!(status, StatusCode::FORBIDDEN, "{body}");
}

/// Point 5: `nodes/proxy` reaches the kubelet past the apiserver: refused
/// under restriction even with a broad `nodes/**` rule, which is flagged in
/// `status.warnings`.
#[tokio::test]
async fn p5_nodes_proxy_is_refused_under_restriction() {
    let test = "rfc0003-p5";
    let ns = ns_of(test);
    let broad = json!({ "Path": { "path": "/api/v1/nodes/**", "parametised": true } });
    let p = fixture::proxied(test, restricted(&ns, &[broad])).await;
    fixture::allow_nodes_proxy();
    let path = format!(
        "/api/v1/nodes/{}/proxy/portForward/{ns}/web?port=8080",
        fixture::node()
    );
    let (status, body) = get(&p, &path).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("nodes/proxy is not allowed"), "{body}");

    let out = Bench::get().admin(&[
        "-n",
        &ns,
        "get",
        &format!("proxykubeapi/{CLUSTER}"),
        "-o",
        "jsonpath={.status.warnings}",
    ]);
    let warnings = String::from_utf8_lossy(&out.stdout);
    assert!(warnings.contains("nodes/"), "status.warnings: {warnings}");
}

/// Point 6: a `Path` rule naming `nodes/*/proxy` explicitly lets the request
/// of point 5 through to the kubelet.
#[tokio::test]
async fn p6_an_explicit_nodes_proxy_rule_lets_it_through() {
    let test = "rfc0003-p6";
    let ns = ns_of(test);
    let explicit = json!({ "Path": { "path": "/api/v1/nodes/*/proxy/**", "parametised": true } });
    let p = fixture::proxied(test, restricted(&ns, &[explicit])).await;
    fixture::allow_nodes_proxy();
    let path = format!(
        "/api/v1/nodes/{}/proxy/portForward/{ns}/web?port=8080",
        fixture::node()
    );
    let (status, body) = get(&p, &path).await;
    // The kubelet answers (it wants an upgrade); the proxy did not refuse.
    assert_ne!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(!body.contains("is not allowed"), "{body}");
}

/// Point 7: without any port restriction, the requests of points 2 to 5 pass
/// as before the RFC.
#[tokio::test]
async fn p7_without_restriction_everything_passes() {
    let p = fixture::proxied(
        "rfc0003-p7",
        json!({ "enabled": true, "allowed_resources": [] }),
    )
    .await;
    fixture::allow_nodes_proxy();
    let ns = p.ns.name.clone();
    let forward = p.reader.port_forward(&ns, "pod/web", 9090).await;
    assert!(forward.is_ok(), "port-forward 9090: {:?}", forward.err());
    for path in [
        format!("/api/v1/namespaces/{ns}/pods/web:9090/proxy/"),
        format!("/api/v1/namespaces/{ns}/services/web:http/proxy/"),
        format!(
            "/api/v1/nodes/{}/proxy/portForward/{ns}/web?port=8080",
            fixture::node()
        ),
    ] {
        let (status, body) = get(&p, &path).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "{path}: {body}");
    }
}
