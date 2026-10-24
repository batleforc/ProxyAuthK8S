//! RFC 0012 §6.P point 14, alone (it restarts the back, like
//! `rfc0011_leader`): the tunnels live in the gateway, so a rolling restart
//! of the proxy leaves them up.

#![expect(
    clippy::indexing_slicing,
    reason = "test harness: a panic is a failed test, with its message"
)]

use std::time::{Duration, Instant};

use integration::{
    Bench,
    bench::{BACK_SERVICE, STACK_NS},
    fixture::{self, CLUSTER},
};
use serde_json::{Value, json};

fn tunnels(ns: &str) -> Value {
    let out = Bench::get().admin(&["-n", ns, "get", "proxykubeapi", CLUSTER, "-o", "json"]);
    let cr: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    cr["status"]["agent"]["tunnels"].clone()
}

fn disconnections(ns: &str) -> usize {
    let out = Bench::get().admin(&[
        "-n",
        STACK_NS,
        "logs",
        "-l",
        "app=proxyauth-proxyauthk8s-gateway",
        "--tail=-1",
        "--max-log-requests=10",
    ]);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line["event"] == "agent_disconnected" && line["ns"] == ns)
        .count()
}

/// Point 14: `rollout restart` of the proxy keeps both tunnels, writes no
/// `agent_disconnected`, and the cluster is served by the new pod.
#[tokio::test]
async fn p14_a_proxy_restart_keeps_the_tunnels() {
    let ns = integration::bench::namespace_name("rfc0012-p14");
    let mut rules = fixture::discovery_rules();
    rules.push(json!({ "Path": {
        "path": format!("/api/v1/namespaces/{ns}/pods/**"),
        "parametised": true,
    } }));
    let p = fixture::agent_proxied(
        "rfc0012-p14",
        json!({ "enabled": true, "allowed_resources": rules }),
        2,
    )
    .await;
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(120))
        .expect("deadline");
    while tunnels(&ns) != json!(2) {
        assert!(Instant::now() < deadline, "never 2 tunnels");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let before = disconnections(&ns);

    let bench = Bench::get();
    let deployment = format!("deployment/{BACK_SERVICE}");
    assert!(
        bench
            .admin(&["-n", STACK_NS, "rollout", "restart", &deployment])
            .status
            .success()
    );
    let rolled = bench.admin(&[
        "-n",
        STACK_NS,
        "rollout",
        "status",
        &deployment,
        "--timeout=180s",
    ]);
    assert!(rolled.status.success(), "{rolled:?}");

    // Traefik follows the new pod a moment after the rollout: a 502 until then.
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(30))
        .expect("deadline");
    loop {
        let pods = p.reader.kubectl(&["get", "pods", "-n", &ns]).await;
        if pods.success() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "not served after the restart: {pods:?}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    assert_eq!(
        disconnections(&ns),
        before,
        "a tunnel dropped with the proxy"
    );
    // The controller restarted too: wait for its next status.
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(120))
        .expect("deadline");
    while tunnels(&ns) != json!(2) {
        assert!(Instant::now() < deadline, "tunnels: {}", tunnels(&ns));
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
