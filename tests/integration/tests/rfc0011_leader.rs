//! RFC 0011 §6.P points 12 and 13: what moves the leader Lease. Both disturb
//! the bench's only leader (a foreign holder, a rollout), so `task it:test`
//! runs this binary alone, after the rest of the suite.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test harness: a panic is a failed test, with its message"
)]

use std::time::{Duration, Instant};

use integration::{Bench, bench::STACK_NS, observe};
use reqwest::StatusCode;
use serde_json::{Value, json};

const LEASE: &str = "proxy-auth-k8s-leader-election";

fn lease() -> Value {
    let out = Bench::get().admin(&["-n", STACK_NS, "get", "lease", LEASE, "-o", "json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("lease JSON")
}

fn holder() -> String {
    lease()["spec"]["holderIdentity"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn leader_gauge() -> f64 {
    observe::metric_value(&observe::metrics(), "proxyauth_leader", &[])
}

fn wait_for(what: &str, timeout: Duration, check: impl Fn() -> bool) -> Duration {
    let started = Instant::now();
    while !check() {
        assert!(started.elapsed() < timeout, "{what} within {timeout:?}");
        std::thread::sleep(Duration::from_millis(200));
    }
    started.elapsed()
}

/// Point 12: another holder writes the Lease; within 12.5 s the pod steps
/// down (`proxyauth_leader` 0), a CR created meanwhile gets no status, and
/// `/management/ready` stays 200. Once the foreign Lease expires, the pod
/// takes it back.
#[tokio::test]
async fn p12_a_foreign_holder_makes_the_leader_step_down() {
    let bench = Bench::get();
    let mut taken = lease();
    let now = "2026-01-01T00:00:00.000000Z";
    taken["spec"]["holderIdentity"] = json!("it-foreign-holder");
    taken["spec"]["acquireTime"] = json!(now);
    taken["spec"]["renewTime"] = json!(now);
    taken["spec"]["leaseDurationSeconds"] = json!(15);
    let out = bench.admin_with_stdin(&["replace", "-f", "-"], Some(&taken.to_string()));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    wait_for(
        "the leader to step down",
        Duration::from_millis(12_500),
        || leader_gauge() == 0.0,
    );
    let ns = bench.namespace("rfc0011-p12");
    bench.apply(
        &json!({
            "apiVersion": "weebo.si.rs/v1",
            "kind": "ProxyKubeApi",
            "metadata": { "name": "c", "namespace": ns.name },
            "spec": {
                "cert": { "ConfigMap": { "name": "kube-root-ca.crt", "key": "ca.crt" } },
                "service": { "KubernetesService": {
                    "name": "kubernetes", "namespace": "default", "port": 443 } },
            },
        })
        .to_string(),
    );
    std::thread::sleep(Duration::from_secs(3));
    let out = bench.admin(&[
        "-n",
        &ns.name,
        "get",
        "proxykubeapi",
        "c",
        "-o",
        "jsonpath={.status}",
    ]);
    assert!(
        out.stdout.is_empty(),
        "a follower reconciled: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let ready = bench
        .http()
        .get(format!("{}/management/ready", bench.base_url()))
        .send()
        .await
        .expect("ready");
    assert_eq!(ready.status(), StatusCode::OK);

    // The foreign holder never renews: one TTL later the pod leads again and
    // reconciles the CR it skipped.
    wait_for("the pod to lead again", Duration::from_secs(60), || {
        leader_gauge() == 1.0
    });
    let out = bench.admin(&[
        "-n",
        &ns.name,
        "wait",
        "proxykubeapi/c",
        "--for=condition=Ready",
        "--timeout=60s",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Point 13: on a rollout, the old leader releases the Lease (`step_down`)
/// and a new pod holds it less than 5 s after the old pod was told to stop,
/// instead of the 15 s TTL.
#[tokio::test]
async fn p13_a_rollout_hands_the_lease_over_without_waiting_for_the_ttl() {
    let bench = Bench::get();
    let old = holder();
    let out = bench.admin(&[
        "-n",
        STACK_NS,
        "rollout",
        "restart",
        "deployment/proxyauth-proxyauthk8s-back",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // When the old pod is told to stop (deletionTimestamp, or already gone).
    let stopping = || {
        let out = bench.admin(&["-n", STACK_NS, "get", "pod", &old, "-o", "json"]);
        !out.status.success()
            || serde_json::from_slice::<Value>(&out.stdout)
                .is_ok_and(|pod| !pod["metadata"]["deletionTimestamp"].is_null())
    };
    wait_for(
        "the old leader to be stopped",
        Duration::from_secs(180),
        stopping,
    );
    let gap = wait_for(
        "a new pod to hold the Lease",
        Duration::from_secs(30),
        || {
            let now = holder();
            !now.is_empty() && now != old
        },
    );
    // `acquire` retries every 5 s: at most one retry after the release.
    assert!(gap < Duration::from_secs(6), "handover took {gap:?}");
    let out = bench.admin(&[
        "-n",
        STACK_NS,
        "rollout",
        "status",
        "deployment/proxyauth-proxyauthk8s-back",
        "--timeout=180s",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
