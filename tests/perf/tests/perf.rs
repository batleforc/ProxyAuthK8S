//! What the `server` binary costs to run (RFC 0011, phase 5): the release
//! binary against the testkit's envtest, with a Redis and `ProxyKubeApi`s
//! whose probe answers, measured by weebo-kit-perf against a budget.
//! `task perf` runs it (`--ignored`); the `perf` workflow, once a week.

use std::path::Path;
use std::time::Duration;

use kube::{Api, CustomResource};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use weebo_kit_perf::run::{BASELINE, FINAL};
use weebo_kit_perf::workload::names;
use weebo_kit_perf::{Budget, OperatorMetrics, PerfRun};
use weebo_kit_testkit::envtest::EnvTestCluster;

/// The fields of a `ProxyKubeApi` this measure writes and reads; the CRD
/// installed in envtest is the real one (`deploy/crd/`).
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "weebo.si.rs",
    version = "v1",
    kind = "ProxyKubeApi",
    namespaced,
    status = "ProxyKubeApiStatus"
)]
struct ProxyKubeApiSpec {
    cert: serde_json::Value,
    service: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
struct ProxyKubeApiStatus {
    #[serde(default)]
    conditions: Vec<Condition>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
struct Condition {
    #[serde(rename = "type")]
    type_: String,
    status: String,
}

/// `ProxyKubeApi`s of a load phase.
const OBJECTS: usize = 100;

/// shortcut: the kit's small-cluster defaults until the first measure on the
/// `perf` workflow's runner (it runs with `WEEBO_PERF_REPORT_ONLY=1` until
/// then); then the measure + 25 % (decision 38). A first run on a laptop
/// (2026-10-23): idle RSS 26.6 → 29.5 MiB, 33 API req/min at rest, 4 per
/// object, p95 ≤ 0.08 s; only the idle RSS growth (10.8 % > 10 %) was over.
fn budget() -> Budget {
    Budget::small_cluster()
}

/// A target whose probe answers 200: the reconcile's reachability check
/// needs one, and a plain-HTTP listener keeps TLS out of the measure.
async fn target() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("target bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(async move {
        loop {
            let Ok((mut conn, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0_u8; 1024];
                if conn.read(&mut buf).await.is_ok() {
                    let _written = conn
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                        )
                        .await;
                }
            });
        }
    });
    url
}

fn proxy(name: &str, url: &str) -> ProxyKubeApi {
    let mut proxy = ProxyKubeApi::new(
        name,
        ProxyKubeApiSpec {
            cert: serde_json::json!({ "SystemRoots": true }),
            service: serde_json::json!({ "ExternalService": { "url": url } }),
        },
    );
    proxy.metadata.namespace = Some("default".to_string());
    proxy
}

fn ready(proxy: &ProxyKubeApi) -> bool {
    proxy.status.as_ref().is_some_and(|status| {
        status
            .conditions
            .iter()
            .any(|c| c.type_ == "Ready" && c.status == "True")
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "a perf run on a release build: task perf"]
async fn proxyauthk8s_costs_within_its_budget() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cluster = EnvTestCluster::start(&root.join("deploy/crd")).await;
    let binary = std::env::var("PROXYAUTH_SERVER_BIN")
        .expect("PROXYAUTH_SERVER_BIN: the release `server` binary (task perf sets it)");
    let mut command = tokio::process::Command::new(binary);
    command
        .env(
            "REDIS_URL",
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
        )
        // Discovery retries in the background and only gates readiness.
        .env("OIDC_ISSUER_URL", "http://127.0.0.1:1")
        .env("SERVER_PORT", "0")
        .env("LEASE_NAMESPACE", "default")
        .env("RUST_LOG", "warn");

    let mut run = PerfRun::start(
        "proxyauthk8s",
        &cluster,
        command,
        OperatorMetrics::new("proxyauth"),
    )
    .await;
    let url = target().await;
    let proxies = run.workload(Api::<ProxyKubeApi>::namespaced(run.client(), "default"));

    run.idle("startup", Duration::from_secs(30)).await;
    let warm = names("warm", 10);
    run.load("warm-up", warm.len(), async {
        proxies
            .create(&warm.iter().map(|n| proxy(n, &url)).collect::<Vec<_>>())
            .await;
        proxies
            .wait_all(&warm, Duration::from_secs(120), ready)
            .await
    })
    .await;
    run.idle(BASELINE, Duration::from_secs(60)).await;

    let burst = names("burst", OBJECTS);
    run.load("burst", OBJECTS, async {
        proxies
            .create(&burst.iter().map(|n| proxy(n, &url)).collect::<Vec<_>>())
            .await;
        proxies
            .wait_all(&burst, Duration::from_secs(300), ready)
            .await
    })
    .await;
    run.load("delete", OBJECTS, async {
        proxies.delete_all(&burst).await;
        proxies.wait_gone(&burst, Duration::from_secs(300)).await
    })
    .await;
    run.idle(FINAL, Duration::from_secs(60)).await;

    run.finish().await.assert_within(&budget());
}
