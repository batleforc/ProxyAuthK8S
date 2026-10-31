//! What the heavy suite adds to the bench (RFC 0016 §6.1), per test: the
//! upstream echo, an HTTPS server standing where an apiserver would and
//! writing every request it receives (`.it/heavy/echo.py`), so a test reads
//! what the proxy relayed; and the previous release's server, started beside
//! the current one on the same Redis.

use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::{
    Bench,
    bench::{BACK_SERVICE, STACK_NS, namespace_name},
    fixture::{self, CLUSTER, Proxied},
};

/// The echo's Service, Deployment and port.
pub const ECHO: &str = "echo";
const ECHO_PORT: u16 = 8443;

/// The echo's URL, as the cluster's `ExternalService` names it.
#[must_use]
pub fn echo_url(ns: &str) -> String {
    format!("https://{ECHO}.{ns}.svc:{ECHO_PORT}")
}

/// [`fixture::proxied`] in front of the echo of its namespace instead of the
/// bench's apiserver: `c` is an `ExternalService` on [`echo_url`], verified
/// against the run CA, in `Passthrough`; `reader` is logged in.
pub async fn echoed(test: &str) -> Proxied {
    let ns = namespace_name(test);
    let service = json!({ "ExternalService": { "url": echo_url(&ns) } });
    fixture::proxied_through(test, service, Value::Null, |ns| {
        deploy_echo(ns);
        let patch = json!({ "spec": {
            "cert": { "ConfigMap": { "name": "echo-ca", "key": "ca.crt" } } } });
        let out = Bench::get().admin(&[
            "-n",
            ns,
            "patch",
            &format!("proxykubeapi/{CLUSTER}"),
            "--type=merge",
            "-p",
            &patch.to_string(),
        ]);
        assert!(
            out.status.success(),
            "point the cluster at the echo's CA: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    })
    .await
}

/// The echo in `ns`: a certificate for `echo.<ns>.svc` from the run CA
/// (mkcert, as `it:up` makes the bench's), the script, its Deployment and
/// Service, available.
pub fn deploy_echo(ns: &str) {
    let bench = Bench::get();
    let dir = bench.run.join("heavy").join(ns);
    std::fs::create_dir_all(&dir).expect("heavy run directory");
    let (crt, key) = (dir.join("echo.crt"), dir.join("echo.key"));
    let host = format!("{ECHO}.{ns}.svc");
    let out = std::process::Command::new("mkcert")
        .env("CAROOT", bench.run.join("ca"))
        .arg("-cert-file")
        .arg(&crt)
        .arg("-key-file")
        .arg(&key)
        .arg(&host)
        .output()
        .expect("mkcert is on PATH");
    assert!(
        out.status.success(),
        "mkcert {host}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = |p: &PathBuf| p.display().to_string();
    let script = bench.root.join(".it/heavy/echo.py");
    let ca = bench.ca_path();
    for args in [
        vec![
            "create".to_string(),
            "secret".into(),
            "tls".into(),
            "echo-tls".into(),
            format!("--cert={}", path(&crt)),
            format!("--key={}", path(&key)),
        ],
        vec![
            "create".to_string(),
            "configmap".into(),
            "echo".into(),
            format!("--from-file=echo.py={}", path(&script)),
        ],
        vec![
            "create".to_string(),
            "configmap".into(),
            "echo-ca".into(),
            format!("--from-file=ca.crt={}", path(&ca)),
        ],
    ] {
        let mut full = vec!["-n", ns];
        full.extend(args.iter().map(String::as_str));
        let out = bench.admin(&full);
        assert!(
            out.status.success(),
            "kubectl {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let items = json!({
        "apiVersion": "v1",
        "kind": "List",
        "items": [
            {
                "apiVersion": "apps/v1",
                "kind": "Deployment",
                "metadata": { "name": ECHO, "namespace": ns },
                "spec": {
                    "selector": { "matchLabels": { "app": ECHO } },
                    "template": {
                        "metadata": { "labels": { "app": ECHO } },
                        "spec": {
                            "containers": [{
                                "name": ECHO,
                                "image": fixture::image("python"),
                                "command": ["python3", "-u", "/app/echo.py"],
                                "ports": [{ "containerPort": ECHO_PORT }],
                                "readinessProbe": { "tcpSocket": { "port": ECHO_PORT } },
                                "volumeMounts": [
                                    { "name": "app", "mountPath": "/app", "readOnly": true },
                                    { "name": "tls", "mountPath": "/tls", "readOnly": true },
                                ],
                            }],
                            "volumes": [
                                { "name": "app", "configMap": { "name": ECHO } },
                                { "name": "tls", "secret": { "secretName": "echo-tls" } },
                            ],
                        },
                    },
                },
            },
            {
                "apiVersion": "v1",
                "kind": "Service",
                "metadata": { "name": ECHO, "namespace": ns },
                "spec": {
                    "selector": { "app": ECHO },
                    "ports": [{ "port": ECHO_PORT, "targetPort": ECHO_PORT }],
                },
            },
        ],
    });
    bench.apply(&items.to_string());
    fixture::wait(
        ns,
        &[&format!("deployment/{ECHO}"), "--for=condition=Available"],
    );
}

/// One request the echo received.
#[derive(Debug, Clone)]
pub struct Relayed {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl Relayed {
    /// Every value of the header `name`, matched without case.
    #[must_use]
    pub fn header(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

/// Every request the echo of `ns` has received so far, in order.
#[must_use]
pub fn transcript(ns: &str) -> Vec<Relayed> {
    let out = Bench::get().admin(&["-n", ns, "logs", &format!("deployment/{ECHO}")]);
    assert!(
        out.status.success(),
        "read the echo's log: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line.get("method").is_some())
        .map(|line| Relayed {
            method: line["method"].as_str().unwrap_or_default().to_string(),
            path: line["path"].as_str().unwrap_or_default().to_string(),
            headers: line["headers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|pair| {
                    Some((pair[0].as_str()?.to_string(), pair[1].as_str()?.to_string()))
                })
                .collect(),
        })
        .collect()
}

/// The requests of the echo of `ns` whose path contains `marker`.
#[must_use]
pub fn relayed(ns: &str, marker: &str) -> Vec<Relayed> {
    transcript(ns)
        .into_iter()
        .filter(|r| r.path.contains(marker))
        .collect()
}

/// A WebSocket handshake on `path` of the cluster of `ns`, on HTTP/1.1
/// through the Ingress, with `token` in `Authorization` and `protocols` in
/// `Sec-WebSocket-Protocol`: the status the client got.
pub async fn websocket_handshake(ns: &str, path: &str, token: &str, protocols: &str) -> u16 {
    Bench::get()
        .http1()
        .get(format!(
            "{}/clusters/{ns}/{CLUSTER}{path}",
            Bench::get().base_url()
        ))
        .bearer_auth(token)
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("sec-websocket-protocol", protocols)
        .send()
        .await
        .expect("handshake through the proxy")
        .status()
        .as_u16()
}

/// The `Deployment` of the previous release's server.
const PREVIOUS: &str = "proxyauth-heavy-previous";

/// The previous release's server (`server-previous` of `.it/images.lock`),
/// beside the current one: the chart's back Deployment with that image, one
/// replica, its own label so the Service never routes to it, and a
/// port-forward to its HTTP port. Deleted on drop.
pub struct Previous {
    /// The local port forwarded to its HTTP port (plain HTTP, 127.0.0.1).
    pub local: u16,
    _forward: tokio::process::Child,
}

impl Previous {
    pub async fn start() -> Previous {
        let bench = Bench::get();
        let out = bench.admin(&[
            "-n",
            STACK_NS,
            "get",
            "deployment",
            BACK_SERVICE,
            "-o",
            "json",
        ]);
        assert!(out.status.success(), "read the back's Deployment");
        let current: Value = serde_json::from_slice(&out.stdout).expect("Deployment JSON");
        let mut template = current["spec"]["template"].clone();
        template["metadata"] = json!({ "labels": { "app": PREVIOUS } });
        let container = &mut template["spec"]["containers"][0];
        container["image"] = json!(fixture::image("server-previous"));
        container["imagePullPolicy"] = json!("IfNotPresent");
        let port = container["ports"]
            .as_array()
            .and_then(|ports| ports.iter().find(|p| p["name"] == "http"))
            .and_then(|p| p["containerPort"].as_u64())
            .expect("the back's http port");
        let deployment = json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": { "name": PREVIOUS, "namespace": STACK_NS },
            "spec": {
                "replicas": 1,
                "selector": { "matchLabels": { "app": PREVIOUS } },
                "template": template,
            },
        });
        bench.apply(&deployment.to_string());
        let out = bench.admin(&[
            "-n",
            STACK_NS,
            "rollout",
            "status",
            &format!("deployment/{PREVIOUS}"),
            "--timeout=300s",
        ]);
        assert!(
            out.status.success(),
            "the previous server did not start: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let local = std::net::TcpListener::bind(("127.0.0.1", 0))
            .and_then(|l| l.local_addr())
            .expect("free local port")
            .port();
        let mut child = tokio::process::Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(bench.admin_kubeconfig())
            .args([
                "-n",
                STACK_NS,
                "port-forward",
                &format!("deployment/{PREVIOUS}"),
                &format!("{local}:{port}"),
                "--address",
                "127.0.0.1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn kubectl port-forward");
        let mut lines = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
        let ready = tokio::time::timeout(Duration::from_secs(30), async {
            while let Ok(Some(line)) = lines.next_line().await {
                if line.starts_with("Forwarding from") {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        assert!(ready, "port-forward to the previous server");
        // Keep draining stdout so kubectl never blocks on a full pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        Previous {
            local,
            _forward: child,
        }
    }

    /// A GET of `path` on the cluster `ns/c`, straight to the previous
    /// server: its status and body.
    pub async fn get(&self, ns: &str, path: &str, token: &str) -> (u16, String) {
        let response = reqwest::Client::new()
            .get(format!(
                "http://127.0.0.1:{}/clusters/{ns}/{CLUSTER}{path}",
                self.local
            ))
            .bearer_auth(token)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .expect("request to the previous server");
        let status = response.status().as_u16();
        (status, response.text().await.unwrap_or_default())
    }
}

impl Drop for Previous {
    fn drop(&mut self) {
        if std::env::var_os("IT_KEEP").is_none() {
            Bench::get().admin(&[
                "-n",
                STACK_NS,
                "delete",
                "deployment",
                PREVIOUS,
                "--wait=false",
            ]);
        }
    }
}

/// Retry `check` every 2 s for up to `timeout`; `true` once it holds.
pub async fn eventually(timeout: Duration, mut check: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    check()
}
