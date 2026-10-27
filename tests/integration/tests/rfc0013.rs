//! RFC 0013 §6.P, on the bench of RFC 0012: the agents' versions in the
//! status, the skew advisory, and the protocol refusal. Three real clients:
//! the branch's `agent:it`, the published 0.4.0 agent (`agent-v0.4.0` in
//! `.it/images.lock`), and `agent:it-announce`, built with the feature that
//! overrides what it announces.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test harness: a panic is a failed test, with its message"
)]

use std::time::{Duration, Instant};

use integration::{
    Bench,
    bench::STACK_NS,
    fixture::{self, AGENT_RELEASE, CLUSTER, Proxied},
};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

const GATEWAY: &str = "proxyauth-proxyauthk8s-gateway";
const AGENT_DEPLOYMENT: &str = "deployment/agent-proxyauthk8s-agent";
/// `P`: the proxy's version. cog bumps every crate of the workspace together,
/// so the suite's own is the controller's and the branch agent's.
const P: &str = env!("CARGO_PKG_VERSION");

fn rules(ns: &str) -> Value {
    let mut rules = fixture::discovery_rules();
    rules.push(json!({ "Path": {
        "path": format!("/api/v1/namespaces/{ns}/pods/**"),
        "parametised": true,
    } }));
    json!({ "enabled": true, "allowed_resources": rules })
}

/// The fixture with `replicas` agents installed with `sets` (helm `--set`s).
async fn agent_fixture(test: &str, replicas: u32, sets: &[&str]) -> Proxied {
    let ns = integration::bench::namespace_name(test);
    let sets: Vec<String> = sets.iter().map(ToString::to_string).collect();
    fixture::agent_proxied_with(
        test,
        fixture::agent_service(&ns),
        rules(&ns),
        replicas,
        |_| sets,
    )
    .await
}

/// `--set`s of the bench's `agent:it-announce`, announcing `env`.
fn announcing(env: &[(&str, &str)]) -> Vec<String> {
    let mut sets = vec!["image.tag=it-announce".to_string()];
    for (index, (name, value)) in env.iter().enumerate() {
        sets.push(format!("env[{index}].name={name}"));
        sets.push(format!("env[{index}].value={value}"));
    }
    sets
}

fn strs(sets: &[String]) -> Vec<&str> {
    sets.iter().map(String::as_str).collect()
}

fn cluster_cr(ns: &str) -> Value {
    let out = Bench::get().admin(&["-n", ns, "get", "proxykubeapi", CLUSTER, "-o", "json"]);
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

fn condition(cr: &Value, type_: &str) -> Option<Value> {
    cr["status"]["conditions"]
        .as_array()?
        .iter()
        .find(|c| c["type"] == type_)
        .cloned()
}

async fn eventually<T>(timeout: Duration, what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now().checked_add(timeout).expect("deadline");
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// `status.agent.agents` once its tunnels add up to `tunnels`.
async fn agents(ns: &str, tunnels: u64) -> Vec<Value> {
    eventually(Duration::from_secs(120), "status.agent.agents", || {
        let rows = cluster_cr(ns)["status"]["agent"]["agents"]
            .as_array()
            .cloned()?;
        let sum: u64 = rows.iter().filter_map(|row| row["tunnels"].as_u64()).sum();
        (sum == tunnels).then_some(rows)
    })
    .await
}

/// The `events.k8s.io` Events about the cluster.
fn events(ns: &str) -> Vec<Value> {
    let out = Bench::get().admin(&["-n", ns, "get", "events.v1.events.k8s.io", "-o", "json"]);
    serde_json::from_slice::<Value>(&out.stdout).unwrap_or_default()["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e["regarding"]["kind"] == "ProxyKubeApi" && e["regarding"]["name"] == CLUSTER)
        .collect()
}

/// The gateway pods' audit lines `event` about the cluster of `ns` since
/// `started`.
fn gateway_events(ns: &str, event: &str, started: Instant) -> Vec<Value> {
    let since = format!("--since={}s", started.elapsed().as_secs().saturating_add(2));
    let out = Bench::get().admin(&[
        "-n",
        STACK_NS,
        "logs",
        "-l",
        &format!("app={GATEWAY}"),
        "--tail=-1",
        "--max-log-requests=10",
        &since,
    ]);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line.get("audit_schema").is_some())
        .filter(|line| line["event"] == event && line["ns"] == ns && line["cluster"] == CLUSTER)
        .collect()
}

/// The skew advisory, its condition `True` and its `Normal` Event, both
/// naming `version` and `words`.
async fn skew_is_advised(ns: &str, version: &str, words: &str) {
    let skew = eventually(Duration::from_secs(120), "AgentVersionSkew", || {
        condition(&cluster_cr(ns), "AgentVersionSkew").filter(|c| c["status"] == "True")
    })
    .await;
    let message = skew["message"].as_str().unwrap_or_default();
    assert!(
        message.contains(version) && message.contains(words),
        "{skew}"
    );
    eventually(Duration::from_secs(60), "an AgentVersionSkew Event", || {
        events(ns).into_iter().find(|e| {
            e["type"] == "Normal"
                && e["reason"] == "AgentVersionSkew"
                && e["note"]
                    .as_str()
                    .is_some_and(|note| note.contains(version) && note.contains(words))
        })
    })
    .await;
}

/// Point 1: two branch agents; the status names `P` on both sides, no
/// advisory, and `-o wide` shows the columns.
#[tokio::test]
async fn p1_branch_agents_are_counted_without_advisory() {
    let p = agent_fixture("rfc0013-p1", 2, &[]).await;
    let ns = p.ns.name.as_str();
    let pods = p.reader.kubectl(&["get", "pods", "-n", ns]).await;
    assert!(pods.success() && pods.stdout.contains("web"), "{pods:?}");

    let rows = agents(ns, 2).await;
    assert_eq!(
        rows,
        vec![json!({ "version": P, "protocol": 1, "tunnels": 2 })]
    );
    let cr = cluster_cr(ns);
    assert_eq!(cr["status"]["agent"]["proxyVersion"], P, "{cr}");
    assert!(condition(&cr, "AgentVersionSkew").is_none(), "{cr}");

    let wide = Bench::get().admin(&["-n", ns, "get", "proxykubeapi", CLUSTER, "-o", "wide"]);
    let wide = String::from_utf8_lossy(&wide.stdout);
    let (header, row) = wide.split_once('\n').expect("a header and a row");
    let column = |name: &str| {
        let at = header
            .find(name)
            .unwrap_or_else(|| panic!("no {name}: {wide}"));
        row.get(at..).unwrap_or_default().split_whitespace().next()
    };
    assert_eq!(column("TUNNELS"), Some("2"), "{wide}");
    assert!(
        column("AGENTS").is_some_and(|agents| agents.contains(P)),
        "{wide}"
    );
}

/// Point 2: the published 0.4.0 agent, which sends neither tag 3 nor 4,
/// opens a tunnel at protocol 1.
#[tokio::test]
async fn p2_the_published_0_4_0_agent_is_served_at_protocol_1() {
    let started = Instant::now();
    let published = fixture::image("agent-v0.4.0");
    let (repository, tag) = published.split_once(':').expect("repository:tag@digest");
    let sets = [
        format!("image.repository={repository}"),
        format!("image.tag={tag}"),
        "image.pullPolicy=IfNotPresent".to_string(),
    ];
    let p = agent_fixture("rfc0013-p2", 1, &strs(&sets)).await;
    let ns = p.ns.name.as_str();
    let pods = p.reader.kubectl(&["get", "pods", "-n", ns]).await;
    assert!(pods.success() && pods.stdout.contains("web"), "{pods:?}");

    let connected = gateway_events(ns, "agent_connected", started);
    assert!(
        connected
            .iter()
            .any(|line| line["agent_version"] == "0.4.0"),
        "{connected:?}"
    );
    let rows = agents(ns, 1).await;
    assert_eq!(
        rows,
        vec![json!({ "version": "0.4.0", "protocol": 1, "tunnels": 1 })]
    );
    // The patch does not count: 0.4.0 is inside the window of 0.4.x.
    if P.starts_with("0.4.") {
        assert!(condition(&cluster_cr(ns), "AgentVersionSkew").is_none());
    }
}

/// Point 3: an agent announcing 0.1.0 is served, and advised as behind.
#[tokio::test]
async fn p3_an_old_agent_is_served_and_advised_behind() {
    let sets = announcing(&[("AGENT_IT_ANNOUNCE_VERSION", "0.1.0")]);
    let p = agent_fixture("rfc0013-p3", 1, &strs(&sets)).await;
    let ns = p.ns.name.as_str();
    let pods = p.reader.kubectl(&["get", "pods", "-n", ns]).await;
    assert!(pods.success(), "{pods:?}");
    skew_is_advised(ns, "0.1.0", "behind").await;
}

/// Point 4: an agent announcing 99.0.0, advised as newer.
#[tokio::test]
async fn p4_a_newer_agent_is_served_and_advised_newer() {
    let sets = announcing(&[("AGENT_IT_ANNOUNCE_VERSION", "99.0.0")]);
    let p = agent_fixture("rfc0013-p4", 1, &strs(&sets)).await;
    let ns = p.ns.name.as_str();
    let pods = p.reader.kubectl(&["get", "pods", "-n", ns]).await;
    assert!(pods.success(), "{pods:?}");
    skew_is_advised(ns, "99.0.0", "newer").await;
}

/// The agent pod's `/metrics`, through the pod proxy on its `probes` port.
fn agent_metrics(ns: &str) -> String {
    let bench = Bench::get();
    let out = bench.admin(&[
        "-n",
        ns,
        "get",
        "pods",
        "-l",
        &format!("app={AGENT_RELEASE}-proxyauthk8s-agent"),
        "--field-selector=status.phase=Running",
        "-o",
        "jsonpath={.items[0].metadata.name}",
    ]);
    let pod = String::from_utf8_lossy(&out.stdout).into_owned();
    let path = format!("/api/v1/namespaces/{ns}/pods/{pod}:9090/proxy/metrics");
    String::from_utf8_lossy(&bench.admin(&["get", "--raw", &path]).stdout).into_owned()
}

/// Point 5: the only agent speaks 2-3: 503, `AgentProtocolUnsupported`,
/// the refusal in the status and the audit, never a tunnel; back to the
/// branch agent, `Ready` again and the refusal gone.
#[tokio::test]
async fn p5_a_protocol_without_overlap_is_refused_then_recovers() {
    let p = agent_fixture("rfc0013-p5", 1, &[]).await;
    let ns = p.ns.name.clone();
    let bench = Bench::get();
    // A refused agent is never Ready (`/readyz` is the tunnel): a rolling
    // update would keep the branch agent serving. Drop it first.
    drop_agents(&ns);
    let started = Instant::now();
    let sets = announcing(&[("AGENT_IT_ANNOUNCE_PROTOCOLS", "2-3")]);
    fixture::install_agent(&ns, &format!("{ns}/{CLUSTER}"), 1, &strs(&sets));

    let token = p.reader.cluster_token(&ns, CLUSTER).await;
    let path = format!("/api/v1/namespaces/{ns}/pods");
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(120))
        .expect("deadline");
    loop {
        let response = bench
            .proxied(Method::GET, &ns, CLUSTER, &path, &token)
            .send()
            .await
            .expect("proxied request");
        if response.status() == StatusCode::SERVICE_UNAVAILABLE {
            let body: Value = response.json().await.unwrap_or_default();
            if body["message"] == "agent unavailable" {
                break;
            }
        }
        assert!(Instant::now() < deadline, "never 503 agent unavailable");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let cr = eventually(
        Duration::from_secs(120),
        "Ready: AgentProtocolUnsupported",
        || {
            let cr = cluster_cr(&ns);
            condition(&cr, "Ready")
                .filter(|c| c["status"] == "False" && c["reason"] == "AgentProtocolUnsupported")
                .map(|_| cr)
        },
    )
    .await;
    let refused = &cr["status"]["agent"]["refused"];
    assert_eq!(refused["reason"], "protocol", "{cr}");
    assert_eq!(refused["protocols"], "2-3", "{cr}");
    assert_eq!(refused["version"], P, "{cr}");

    let lines = gateway_events(&ns, "agent_refused", started);
    assert!(
        lines
            .iter()
            .any(|line| line["reason"] == "protocol" && line["agent_version"] == P),
        "{lines:?}"
    );
    // The branch agent was gone before `started`: no tunnel at all since.
    assert!(gateway_events(&ns, "agent_connected", started).is_empty());

    let metrics = agent_metrics(&ns);
    assert!(
        integration::observe::metric_value(
            &metrics,
            "proxyauth_agent_reconnects_total",
            &[("reason", "protocol")]
        ) >= 1.0,
        "{metrics}"
    );
    let logs = bench.admin(&["-n", &ns, "logs", AGENT_DEPLOYMENT, "--tail=-1"]);
    let logs = String::from_utf8_lossy(&logs.stdout);
    assert!(
        logs.lines()
            .any(|line| line.contains("ERROR") && line.contains("2-3") && line.contains("1-1")),
        "{logs}"
    );

    // Dropped first too: the refusal key lives 5 min after the last attempt,
    // and a rolling update would let the refused agent go on trying.
    drop_agents(&ns);
    fixture::install_agent(&ns, &format!("{ns}/{CLUSTER}"), 1, &[]);
    eventually(Duration::from_secs(120), "Ready again", || {
        condition(&cluster_cr(&ns), "Ready").filter(|c| c["status"] == "True")
    })
    .await;
    // The key's 5 min, then one agent requeue of at most 60 s.
    eventually(Duration::from_secs(420), "refused gone", || {
        cluster_cr(&ns)["status"]["agent"]["refused"]
            .is_null()
            .then_some(())
    })
    .await;
}

/// Scales the test's agents to 0 and waits for their pods to be gone.
fn drop_agents(ns: &str) {
    let scaled = Bench::get().admin(&["-n", ns, "scale", AGENT_DEPLOYMENT, "--replicas=0"]);
    assert!(scaled.status.success());
    fixture::wait(
        ns,
        &[
            "--for=delete",
            "pod",
            "-l",
            &format!("app={AGENT_RELEASE}-proxyauthk8s-agent"),
        ],
    );
}

/// Point 6: a version with a newline and JSON after it is served as
/// `invalid`, and its audit line stays one JSON line without the key.
#[tokio::test]
async fn p6_a_forged_version_is_recorded_invalid() {
    let p = agent_fixture("rfc0013-p6", 1, &["image.tag=it-announce"]).await;
    let ns = p.ns.name.clone();
    let started = Instant::now();
    let forged = "AGENT_IT_ANNOUNCE_VERSION=0.5.0\n{\"forged\":1}";
    let out = Bench::get().admin(&["-n", &ns, "set", "env", AGENT_DEPLOYMENT, forged]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    fixture::wait(&ns, &["--for=condition=Available", AGENT_DEPLOYMENT]);

    let rows = eventually(Duration::from_secs(120), "an invalid row", || {
        let rows = cluster_cr(&ns)["status"]["agent"]["agents"]
            .as_array()
            .cloned()?;
        rows.iter()
            .any(|row| row["version"] == "invalid")
            .then_some(rows)
    })
    .await;
    assert!(
        rows.iter().all(|row| row["version"] == "invalid"),
        "{rows:?}"
    );
    let pods = p.reader.kubectl(&["get", "pods", "-n", &ns]).await;
    assert!(pods.success(), "{pods:?}");

    let connected = eventually(Duration::from_secs(60), "agent_connected", || {
        let lines = gateway_events(&ns, "agent_connected", started);
        (!lines.is_empty()).then_some(lines)
    })
    .await;
    for line in &connected {
        assert!(line.get("forged").is_none(), "{line}");
        assert_ne!(line["agent_version"], "0.5.0", "{line}");
    }
}

/// Point 7: the publishable `agent:it` ignores `AGENT_IT_ANNOUNCE_VERSION`
/// and announces `P`.
#[tokio::test]
async fn p7_the_publishable_image_has_no_announce_override() {
    let p = agent_fixture(
        "rfc0013-p7",
        1,
        &[
            "env[0].name=AGENT_IT_ANNOUNCE_VERSION",
            "env[0].value=0.1.0",
        ],
    )
    .await;
    let ns = p.ns.name.as_str();
    let rows = agents(ns, 1).await;
    assert_eq!(
        rows,
        vec![json!({ "version": P, "protocol": 1, "tunnels": 1 })]
    );
    assert!(condition(&cluster_cr(ns), "AgentVersionSkew").is_none());
}
