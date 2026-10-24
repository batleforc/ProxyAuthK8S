//! RFC 0012 §6.P point 11, alone (it switches the stack's one gateway to
//! `tlsMode: passthrough`, and the back with it, then back): the agent's
//! certificate rides in the TLS the gateway ends itself, reached through its
//! Service, no ingress.

use std::process::Command;

use integration::{Bench, bench::STACK_NS, fixture, observe};
use serde_json::json;

const GATEWAY: &str = "proxyauth-proxyauthk8s-gateway";
const TLS_SECRET: &str = "it-gateway-passthrough";

/// `helm upgrade` of the stack with `.it/values.yaml` and `sets`, then the
/// gateway and the back rolled out.
fn upgrade_stack(sets: &[&str]) {
    let bench = Bench::get();
    let mut args = vec![
        "upgrade",
        "proxyauth",
        "deploy/chart",
        "-n",
        STACK_NS,
        "-f",
        ".it/values.yaml",
    ];
    for set in sets {
        args.extend(["--set", set]);
    }
    let out = bench.helm(&args);
    assert!(
        out.status.success(),
        "helm upgrade: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    for deployment in [GATEWAY, "proxyauth-proxyauthk8s-back"] {
        let out = bench.admin(&[
            "-n",
            STACK_NS,
            "rollout",
            "status",
            &format!("deployment/{deployment}"),
            "--timeout=180s",
        ]);
        assert!(
            out.status.success(),
            "rollout of {deployment}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// Puts the stack back in `tlsMode: inner`, whatever the test did.
struct Inner;

impl Drop for Inner {
    fn drop(&mut self) {
        upgrade_stack(&[]);
    }
}

/// Point 11: point 1 in `passthrough`, the agent dialling the gateway's
/// Service with its certificate in the outer TLS.
#[tokio::test]
async fn p11_a_cluster_is_reached_through_a_passthrough_gateway() {
    let bench = Bench::get();
    let host = format!("{GATEWAY}.{STACK_NS}.svc");
    let dir = tempfile::tempdir().expect("temp dir");
    let (cert, key) = (dir.path().join("tls.crt"), dir.path().join("tls.key"));
    let issued = Command::new("mkcert")
        .env("CAROOT", bench.run.join("ca"))
        .arg("-cert-file")
        .arg(&cert)
        .arg("-key-file")
        .arg(&key)
        .arg(&host)
        .output()
        .expect("mkcert is on PATH");
    assert!(issued.status.success(), "{issued:?}");
    let secret = json!({
        "apiVersion": "v1", "kind": "Secret", "type": "kubernetes.io/tls",
        "metadata": { "name": TLS_SECRET, "namespace": STACK_NS },
        "stringData": {
            "tls.crt": std::fs::read_to_string(&cert).expect("cert"),
            "tls.key": std::fs::read_to_string(&key).expect("key"),
        },
    });
    bench.apply(&secret.to_string());

    let _inner = Inner;
    upgrade_stack(&[
        "agentGateway.tlsMode=passthrough",
        &format!("agentGateway.tls.secretName={TLS_SECRET}"),
    ]);
    let test = "rfc0012-p11";
    let ns = integration::bench::namespace_name(test);
    let mut rules = fixture::discovery_rules();
    rules.push(json!({ "Path": {
        "path": format!("/api/v1/namespaces/{ns}/pods/**"),
        "parametised": true,
    } }));
    let url = format!("proxy.url=wss://{host}:8444/agent/v1/connect");
    let p = fixture::agent_proxied_with(
        test,
        fixture::agent_service(&ns),
        json!({ "enabled": true, "allowed_resources": rules }),
        1,
        |_| vec!["tlsMode=passthrough".to_string(), url.clone()],
    )
    .await;
    let pods = p.reader.kubectl(&["get", "pods", "-n", &ns]).await;
    assert!(pods.success() && pods.stdout.contains("web"), "{pods:?}");
    let line = observe::audit_lines(&ns)
        .into_iter()
        .find(|line| {
            line["event"] == "request" && line["ns"] == ns.as_str() && line["status"] == 200
        })
        .expect("a request line");
    assert_eq!(line["upstream_via"], "agent", "{line}");
}
