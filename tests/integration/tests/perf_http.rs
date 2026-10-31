//! RFC 0016 §6.P points 3 and 4: the proxy under load, driven by k6
//! (`tests/perf/k6/scenarios`) through the bench's Ingress, while the back's
//! memory is sampled with `kubectl top`. `task perf:http` runs it alone; the
//! report lands in `PERF_OUT` (default `.it/run/perf`): one k6 summary per
//! scenario, `resources.json` and `summary.md`.

#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test harness: a panic is a failed test, with its message"
)]

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use integration::{
    Bench, Persona,
    bench::{HOST, STACK_NS},
    fixture::{self, CLUSTER},
};
use serde_json::{Value, json};

/// Requests per minute of the rate-limited cluster of point 4.
const RATE_LIMIT: u64 = 120;

fn out_dir() -> PathBuf {
    let dir =
        std::env::var_os("PERF_OUT").map_or_else(|| Bench::get().run.join("perf"), PathBuf::from);
    std::fs::create_dir_all(&dir).expect("PERF_OUT");
    dir
}

/// `k6 run` of one scenario with the bench in its environment; the summary
/// it exported. Panics when k6 fails (a threshold crossed, or worse).
fn k6(scenario: &str, ns: &str, token: &str, dashboard: &str, out: &Path) -> Value {
    let bench = Bench::get();
    let script = bench
        .root
        .join("tests/perf/k6/scenarios")
        .join(format!("{scenario}.js"));
    let summary = out.join(format!("{scenario}.json"));
    let status = std::process::Command::new("k6")
        .args(["run", "--quiet", "--summary-export"])
        .arg(&summary)
        .arg(&script)
        .env("SSL_CERT_FILE", bench.ca_path())
        .env("BASE_URL", bench.base_url())
        .env("HOST", HOST)
        .env("NS", ns)
        .env("CLUSTER", CLUSTER)
        .env("TOKEN", token)
        .env("DASHBOARD_TOKEN", dashboard)
        .status()
        .expect("k6 is on PATH");
    let text = std::fs::read_to_string(&summary)
        .unwrap_or_else(|e| panic!("{scenario}: no k6 summary ({e}), k6 exited {status}"));
    assert!(status.success(), "{scenario}: k6 exited {status}");
    serde_json::from_str(&text).expect("k6 summary JSON")
}

/// The back pods' memory in MiB, as metrics-server reports it.
fn back_memory_mib() -> Option<f64> {
    let out = Bench::get().admin(&[
        "-n",
        STACK_NS,
        "top",
        "pod",
        "-l",
        "app=proxyauth-proxyauthk8s-back",
        "--no-headers",
    ]);
    let text = String::from_utf8_lossy(&out.stdout);
    let total: f64 = text
        .lines()
        .filter_map(|line| line.split_whitespace().nth(2))
        .filter_map(|mem| mem.strip_suffix("Mi"))
        .filter_map(|mib| mib.parse::<f64>().ok())
        .sum();
    (out.status.success() && total > 0.0).then_some(total)
}

/// Samples [`back_memory_mib`] every 15 s until dropped.
struct Sampler {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Vec<f64>>>,
}

impl Sampler {
    fn start() -> Sampler {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let mut samples = Vec::new();
            while !flag.load(Ordering::Relaxed) {
                if let Some(mib) = back_memory_mib() {
                    samples.push(mib);
                }
                for _ in 0..15 {
                    if flag.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
            samples
        });
        Sampler {
            stop,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> Vec<f64> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default()
    }
}

fn metric(summary: &Value, name: &str, field: &str) -> f64 {
    summary["metrics"][name][field].as_f64().unwrap_or(0.0)
}

/// Point 3: every scenario holds its load with fewer than 1 % failed
/// requests (the threshold k6 enforces), and the report is written: p95,
/// p99, throughput per scenario, the back's memory before, at its peak and
/// after.
#[tokio::test]
async fn p3_the_proxy_holds_the_load_without_errors() {
    let p = fixture::proxied("perf-http", Value::Null).await;
    let ns = p.ns.name.clone();
    let configmaps: Vec<Value> = (0..50)
        .map(|i| {
            json!({ "apiVersion": "v1", "kind": "ConfigMap",
                    "metadata": { "name": format!("load-{i}"), "namespace": ns },
                    "data": { "value": "x".repeat(512) } })
        })
        .collect();
    Bench::get()
        .apply(&json!({ "apiVersion": "v1", "kind": "List", "items": configmaps }).to_string());
    let token = p.reader.cluster_token(&ns, CLUSTER).await;
    let dashboard = Persona::named("reader").dashboard_token().await;
    let out = out_dir();

    let before = back_memory_mib();
    let sampler = Sampler::start();
    let mut report = String::from(
        "### HTTP load (RFC 0016)\n\n| Scenario | req/s | p95 ms | p99 ms | failed |\n| --- | --- | --- | --- | --- |\n",
    );
    for scenario in [
        "01_proxied_get",
        "02_proxied_list",
        "03_discovery",
        "04_dashboard_clusters",
    ] {
        let summary = k6(scenario, &ns, &token, &dashboard, &out);
        report.push_str(&format!(
            "| {scenario} | {:.1} | {:.1} | {:.1} | {:.2} % |\n",
            metric(&summary, "http_reqs", "rate"),
            metric(&summary, "http_req_duration", "p(95)"),
            metric(&summary, "http_req_duration", "p(99)"),
            metric(&summary, "http_req_failed", "value") * 100.0,
        ));
    }
    let samples = sampler.finish();
    let after = back_memory_mib();
    let peak = samples.iter().copied().fold(0.0_f64, f64::max);
    let resources = json!({
        "back_memory_mib": { "before": before, "peak": peak, "after": after, "samples": samples },
    });
    std::fs::write(out.join("resources.json"), resources.to_string()).expect("resources.json");
    report.push_str(&format!(
        "\nBack memory (MiB, every pod): before {before:?}, peak {peak:.0}, after {after:?}.\n"
    ));
    std::fs::write(out.join("summary.md"), &report).expect("summary.md");
    println!("{report}");
}

/// Point 4, a refusal under load: a cluster limited to [`RATE_LIMIT`]
/// requests per minute, hammered for 30 s by 10 clients, accepts no more
/// than the limit and answers the rest `429`: the Redis counter holds under
/// concurrency, across the back's requests.
#[tokio::test]
async fn p4_the_rate_limit_holds_under_concurrency() {
    let security = json!({ "rate_limiting": {
        "enabled": true, "max_requests_per_minute": RATE_LIMIT } });
    let p = fixture::proxied("perf-rate-limit", security).await;
    let ns = p.ns.name.clone();
    let token = p.reader.cluster_token(&ns, CLUSTER).await;
    // A fresh window: the login's own requests may have opened one.
    tokio::time::sleep(Duration::from_secs(61)).await;
    let out = out_dir();
    let summary = k6("05_rate_limit", &ns, &token, "", &out);
    let accepted = metric(&summary, "accepted", "count");
    let limited = metric(&summary, "limited", "count");
    let line = format!(
        "\nRate limit {RATE_LIMIT}/min under 30 s of load: {accepted} accepted, {limited} refused with 429.\n"
    );
    let summary_md = out.join("summary.md");
    let mut report = std::fs::read_to_string(&summary_md).unwrap_or_default();
    report.push_str(&line);
    std::fs::write(&summary_md, report).expect("summary.md");
    println!("{line}");
    #[expect(
        clippy::cast_precision_loss,
        reason = "a small request count, exact in an f64"
    )]
    let limit = RATE_LIMIT as f64;
    assert!(
        accepted <= limit,
        "{accepted} requests accepted, over the {RATE_LIMIT}/min limit"
    );
    assert!(limited > 0.0, "nothing was refused");
}
