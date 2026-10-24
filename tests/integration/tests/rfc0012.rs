//! RFC 0012 §6.P, on the bench: clusters reached through an agent. Each test
//! installs its agents (`deploy/chart-agent`) in its own namespace, in front
//! of the bench's own apiserver; the agent gateway runs in the stack's
//! namespace with 2 pods, its agents' Ingress on `agent.k8s.localhost`.
//! Points 8 to 11 are phase 2 (`OneShotToken`, CONNECT and SOCKS5 egress,
//! `passthrough`); point 14 restarts the back and lives in `rfc0012_restart`.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test harness: a panic is a failed test, with its message"
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use integration::{
    Bench,
    bench::STACK_NS,
    fixture::{self, CLUSTER, Proxied},
    observe,
};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

const AGENT_HOST: &str = "agent.k8s.localhost";
const GATEWAY: &str = "proxyauth-proxyauthk8s-gateway";
const AGENT_DEPLOYMENT: &str = "deployment/agent-proxyauthk8s-agent";

/// Discovery, reads on the pods of `ns`, then `extra`.
fn rules(ns: &str, extra: &[Value]) -> Value {
    let mut rules = fixture::discovery_rules();
    rules.push(json!({ "Path": {
        "path": format!("/api/v1/namespaces/{ns}/pods/**"),
        "parametised": true,
    } }));
    rules.extend_from_slice(extra);
    json!({ "enabled": true, "allowed_resources": rules })
}

async fn agent_fixture(test: &str, replicas: u32, extra: impl Fn(&str) -> Vec<Value>) -> Proxied {
    let ns = integration::bench::namespace_name(test);
    fixture::agent_proxied(test, rules(&ns, &extra(&ns)), replicas).await
}

/// The gateway pods' audit lines about the cluster `name` of `ns` written
/// since `started`: namespaces are named after their test, so an earlier
/// run's lines would otherwise count.
fn gateway_events(ns: &str, name: &str, event: &str, started: Instant) -> Vec<Value> {
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
        .filter(|line| line["event"] == event && line["ns"] == ns && line["cluster"] == name)
        .collect()
}

/// Poll `check` every second until it returns `Some`, for at most `timeout`.
async fn eventually<T>(timeout: Duration, what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now().checked_add(timeout).expect("deadline");
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn cluster_cr(ns: &str, name: &str) -> Value {
    let out = Bench::get().admin(&["-n", ns, "get", "proxykubeapi", name, "-o", "json"]);
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

fn ready_reason(cr: &Value) -> Option<String> {
    cr["status"]["conditions"]
        .as_array()?
        .iter()
        .find(|c| c["type"] == "Ready")
        .and_then(|c| c["reason"].as_str().map(ToString::to_string))
}

/// A client of the agents' Ingress, trusting the run's CA only.
fn agent_http() -> reqwest::Client {
    let bench = Bench::get();
    let ca = std::fs::read(bench.ca_path()).expect("run CA");
    reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(&ca).expect("run CA is PEM")])
        .resolve(AGENT_HOST, SocketAddr::from(([127, 0, 0, 1], bench.port)))
        .build()
        .expect("agent http client")
}

fn agent_url(path: &str) -> String {
    format!("https://{AGENT_HOST}:{}{path}", Bench::get().port)
}

/// Point 1: `kubectl get pods` through the agent; the request line says so;
/// both agent replicas hold a tunnel.
#[tokio::test]
async fn p1_a_cluster_is_reached_through_its_agents() {
    let p = agent_fixture("rfc0012-p1", 2, |_| Vec::new()).await;
    let ns = p.ns.name.as_str();
    let pods = p.reader.kubectl(&["get", "pods", "-n", ns]).await;
    assert!(pods.success() && pods.stdout.contains("web"), "{pods:?}");

    let line = observe::audit_lines(ns)
        .into_iter()
        .find(|line| line["event"] == "request" && line["ns"] == ns && line["status"] == 200)
        .expect("a request line");
    assert_eq!(line["upstream_via"], "agent", "{line}");
    let tunnels = eventually(Duration::from_secs(120), "status.agent.tunnels = 2", || {
        let cr = cluster_cr(ns, CLUSTER);
        (cr["status"]["agent"]["tunnels"] == 2).then_some(cr)
    })
    .await;
    assert!(
        !tunnels["status"]["agent"]["versions"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{tunnels}"
    );
}

/// Point 2: exec and port-forward cross the tunnel, under the policies of
/// RFC 0003 and 0004.
#[tokio::test]
async fn p2_exec_and_port_forward_cross_the_tunnel_under_their_policies() {
    let p = agent_fixture("rfc0012-p2", 1, |ns| {
        vec![
            json!({ "Path": {
                "path": format!("/api/v1/namespaces/{ns}/pods/*/exec"),
                "parametised": true,
                "exec": { "commands": [["echo", "**"]], "containers": ["app"] },
            } }),
            json!({ "Path": {
                "path": format!("/api/v1/namespaces/{ns}/pods/*/portforward"),
                "parametised": true,
                "allowed_ports": ["8080"],
            } }),
        ]
    })
    .await;
    let ns = p.ns.name.as_str();
    let echo = p
        .reader
        .kubectl(&["exec", "web", "-n", ns, "-c", "app", "--", "echo", "ok"])
        .await;
    assert!(echo.success() && echo.stdout.trim() == "ok", "{echo:?}");
    let refused = p
        .reader
        .kubectl(&["exec", "web", "-n", ns, "-c", "app", "--", "id"])
        .await;
    assert!(!refused.success(), "{refused:?}");
    assert!(
        refused.stderr.contains("Forbidden") || refused.stderr.contains("403"),
        "{refused:?}"
    );

    let forward = p
        .reader
        .port_forward(ns, "pod/web", 8080)
        .await
        .expect("port-forward 8080");
    let response = reqwest::get(format!("http://127.0.0.1:{}/", forward.local))
        .await
        .expect("HTTP through the port-forward");
    assert!(response.status().as_u16() < 500, "{}", response.status());
}

/// Point 3: a two-minute watch through the tunnel still gets the event of a
/// pod created at its end.
#[tokio::test]
async fn p3_a_long_watch_gets_its_event() {
    let p = agent_fixture("rfc0012-p3", 1, |_| Vec::new()).await;
    let ns = p.ns.name.clone();
    let args = ["get", "pods", "-n", ns.as_str(), "-w"];
    let watch = p.reader.kubectl_for(Duration::from_secs(130), &args);
    let create = async {
        tokio::time::sleep(Duration::from_secs(115)).await;
        let busybox = fixture::image("busybox");
        Bench::get().admin(&[
            "-n",
            &ns,
            "run",
            "late",
            "--image",
            &busybox,
            "--restart=Never",
            "--",
            "sleep",
            "60",
        ]);
    };
    let (out, ()) = tokio::join!(watch, create);
    assert!(out.stdout.contains("late"), "{out:?}");
}

/// Point 4: without an agent, `503 agent unavailable`, `Ready` false for
/// `AgentNoTunnel`, never a direct connection.
#[tokio::test]
async fn p4_no_agent_is_refused_and_never_dialled_directly() {
    let p = agent_fixture("rfc0012-p4", 1, |_| Vec::new()).await;
    let ns = p.ns.name.clone();
    let bench = Bench::get();
    let scaled = bench.admin(&["-n", &ns, "scale", AGENT_DEPLOYMENT, "--replicas=0"]);
    assert!(scaled.status.success());
    let token = p.reader.cluster_token(&ns, CLUSTER).await;
    let path = format!("/api/v1/namespaces/{ns}/pods");
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(90))
        .expect("deadline");
    let body = loop {
        let response = bench
            .proxied(Method::GET, &ns, CLUSTER, &path, &token)
            .send()
            .await
            .expect("proxied request");
        if response.status() == StatusCode::SERVICE_UNAVAILABLE {
            let body: Value = response.json().await.unwrap_or_default();
            if body["message"] == "agent unavailable" {
                break body;
            }
        }
        assert!(Instant::now() < deadline, "never 503 agent unavailable");
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    assert_eq!(body["kind"], "Status");
    eventually(Duration::from_secs(120), "Ready: AgentNoTunnel", || {
        (ready_reason(&cluster_cr(&ns, CLUSTER)).as_deref() == Some("AgentNoTunnel")).then_some(())
    })
    .await;
    // Every request line of the cluster went (or was refused) through the agent.
    assert!(
        observe::audit_lines(&ns)
            .iter()
            .filter(|line| line["event"] == "request" && line["ns"] == ns.as_str())
            .all(|line| line["upstream_via"] == "agent")
    );
}

/// Point 5: raising `epoch` drops the tunnels within 30 s; the agent enrols
/// again at the new epoch and the cluster is served again.
#[tokio::test]
async fn p5_an_epoch_bump_revokes_and_the_agent_comes_back() {
    let started = Instant::now();
    let p = agent_fixture("rfc0012-p5", 1, |_| Vec::new()).await;
    let ns = p.ns.name.clone();
    let patched = Bench::get().admin(&[
        "-n",
        &ns,
        "patch",
        "proxykubeapi",
        CLUSTER,
        "--type=merge",
        "-p",
        r#"{"spec":{"service":{"Agent":{"epoch":1}}}}"#,
    ]);
    assert!(patched.status.success(), "{patched:?}");
    let revoked = eventually(
        Duration::from_secs(30),
        "agent_disconnected: revoked",
        || {
            gateway_events(&ns, CLUSTER, "agent_disconnected", started)
                .into_iter()
                .find(|line| line["reason"] == "revoked")
        },
    )
    .await;
    assert_eq!(revoked["agent_epoch"], 0, "{revoked}");
    eventually(Duration::from_secs(120), "an enrolment at epoch 1", || {
        gateway_events(&ns, CLUSTER, "agent_connected", started)
            .into_iter()
            .find(|line| line["agent_epoch"] == 1)
    })
    .await;
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(60))
        .expect("deadline");
    loop {
        let pods = p.reader.kubectl(&["get", "pods", "-n", &ns]).await;
        if pods.success() {
            break;
        }
        assert!(Instant::now() < deadline, "never served again: {pods:?}");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Point 6: another ServiceAccount's token gets a bare `401` and an
/// `agent_refused` line with its reason.
#[tokio::test]
async fn p6_another_service_accounts_token_is_refused() {
    let started = Instant::now();
    let p = agent_fixture("rfc0012-p6", 1, |_| Vec::new()).await;
    let ns = p.ns.name.clone();
    let token = Bench::get().admin(&[
        "-n",
        &ns,
        "create",
        "token",
        "default",
        "--audience",
        "proxyauth",
    ]);
    assert!(token.status.success());
    let token = String::from_utf8_lossy(&token.stdout).trim().to_string();
    let response = agent_http()
        .post(agent_url("/agent/v1/enroll"))
        .json(&json!({
            "cluster": format!("{ns}/{CLUSTER}"),
            "csr": "not used: the proof fails first",
            "proof": { "service_account_token": token },
        }))
        .send()
        .await
        .expect("enrolment request");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.text().await.unwrap_or_default().is_empty());
    eventually(Duration::from_secs(10), "agent_refused: subject", || {
        gateway_events(&ns, CLUSTER, "agent_refused", started)
            .into_iter()
            .find(|line| line["reason"] == "subject")
    })
    .await;
}

/// Point 7: an agent pointed at cluster B with A's ServiceAccount is refused,
/// and B stays without a tunnel.
#[tokio::test]
async fn p7_an_agent_cannot_serve_another_cluster() {
    let started = Instant::now();
    let p = agent_fixture("rfc0012-p7", 1, |_| Vec::new()).await;
    let ns = p.ns.name.clone();
    let mut service = fixture::agent_service(&ns);
    service["Agent"]["bootstrap"]["ServiceAccountToken"]["subject"] =
        json!(format!("system:serviceaccount:{ns}:b-agent"));
    Bench::get().apply(
        &json!({
            "apiVersion": "weebo.si.rs/v1",
            "kind": "ProxyKubeApi",
            "metadata": { "name": "b", "namespace": ns },
            "spec": {
                "cert": { "ConfigMap": { "name": "kube-root-ca.crt", "key": "ca.crt" } },
                "service": service,
            },
        })
        .to_string(),
    );
    // A's agent, told to serve B.
    fixture::install_agent(&ns, &format!("{ns}/b"), 1, &[]);
    eventually(Duration::from_secs(90), "agent_refused for b", || {
        gateway_events(&ns, "b", "agent_refused", started)
            .into_iter()
            .find(|line| line["reason"] == "subject")
    })
    .await;
    eventually(Duration::from_secs(120), "b: AgentNoTunnel", || {
        (ready_reason(&cluster_cr(&ns, "b")).as_deref() == Some("AgentNoTunnel")).then_some(())
    })
    .await;
}

/// Point 12: a client without an agent certificate gets no tunnel, and the
/// agents' listener serves none of the user API.
#[tokio::test]
async fn p12_no_agent_certificate_no_tunnel_and_no_user_route() {
    let bench = Bench::get();
    // The agent CA, as the gateway serves the tunnel's TLS under it.
    let ca = bench.admin(&[
        "-n",
        STACK_NS,
        "get",
        "secret",
        "proxy-auth-k8s-agent-ca",
        "-o",
        r#"go-template={{index .data "tls.crt" | base64decode}}"#,
    ]);
    assert!(ca.status.success(), "the agent CA Secret");
    let ca_pem = ca.stdout;

    let before = tls_refusals();
    let refused = client_without_certificate(&ca_pem).await;
    assert!(
        refused,
        "the tunnel TLS completed without a client certificate"
    );
    // The gateway's own word for it, not only the client's.
    eventually(Duration::from_secs(15), "agent_refused: tls", || {
        (tls_refusals() > before).then_some(())
    })
    .await;

    for path in [
        "/clusters/default/c/api/v1/pods",
        "/agent/v1/clusters",
        "/api/clusters",
    ] {
        let status = agent_http()
            .get(agent_url(path))
            .send()
            .await
            .expect("request to the agents' host")
            .status();
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
}

/// Point 13: a `jwks_from` ConfigMap without the cluster's key refuses the
/// agent; fixed (an old key and the real one), the agent enrols on its own,
/// the `ProxyKubeApi` untouched.
#[tokio::test]
async fn p13_a_jwks_from_without_the_key_refuses_then_recovers() {
    let started = Instant::now();
    let bench = Bench::get();
    let guard = bench.namespace("rfc0012-p13");
    let ns = guard.name.clone();
    let real: Value =
        serde_json::from_slice(&bench.admin(&["get", "--raw", "/openid/v1/jwks"]).stdout)
            .expect("the bench's JWKS");
    let mut stale = real["keys"][0].clone();
    stale["kid"] = json!("rotated-away");
    let configmap = |keys: Vec<Value>| {
        json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": { "name": "remote-jwks", "namespace": ns },
            "data": { "jwks.json": json!({ "keys": keys }).to_string() },
        })
        .to_string()
    };
    bench.apply(&configmap(vec![stale.clone()]));
    let mut service = fixture::agent_service(&ns);
    let bootstrap = &mut service["Agent"]["bootstrap"]["ServiceAccountToken"];
    bootstrap.as_object_mut().expect("bootstrap").remove("jwks");
    bootstrap["jwks_from"] = json!({ "ConfigMap": { "name": "remote-jwks", "key": "jwks.json" } });
    bench.apply(
        &json!({
            "apiVersion": "weebo.si.rs/v1",
            "kind": "ProxyKubeApi",
            "metadata": { "name": CLUSTER, "namespace": ns },
            "spec": {
                "cert": { "ConfigMap": { "name": "kube-root-ca.crt", "key": "ca.crt" } },
                "service": service,
            },
        })
        .to_string(),
    );
    fixture::install_agent(&ns, &format!("{ns}/{CLUSTER}"), 1, &[]);
    eventually(
        Duration::from_secs(90),
        "agent_refused: unknown key",
        || {
            gateway_events(&ns, CLUSTER, "agent_refused", started)
                .into_iter()
                .find(|line| line["reason"] == "unknown key")
        },
    )
    .await;
    let generation = cluster_cr(&ns, CLUSTER)["metadata"]["generation"].clone();

    bench.apply(&configmap(vec![stale, real["keys"][0].clone()]));
    // The agent backs off up to 60 s between attempts.
    eventually(Duration::from_secs(90), "the agent enrolled", || {
        gateway_events(&ns, CLUSTER, "agent_enrolled", started)
            .into_iter()
            .next()
    })
    .await;
    assert_eq!(
        cluster_cr(&ns, CLUSTER)["metadata"]["generation"],
        generation
    );
    bench.helm(&["uninstall", fixture::AGENT_RELEASE, "-n", &ns]);
}

/// Point 15: from a pod of the relay cluster, a `CONNECT` to the gateway
/// without the token, or with a wrong one, gets `407`, while a real tunnel
/// for that cluster is up.
#[tokio::test]
async fn p15_a_connect_without_the_proxy_token_is_refused() {
    let p = agent_fixture("rfc0012-p15", 1, |_| Vec::new()).await;
    let ns = p.ns.name.clone();
    let uid = cluster_cr(&ns, CLUSTER)["metadata"]["uid"]
        .as_str()
        .expect("uid")
        .to_string();
    let target = format!("{GATEWAY}-pods.{STACK_NS}");
    // `sleep 2` first: what a pod prints before `kubectl run -i` attaches is
    // lost. stdin held open: `nc` stops reading once its input ends.
    let script = format!(
        "sleep 2; for auth in '' 'Proxy-Authorization: Bearer wrong\\r\\n'; do \
           (printf \"CONNECT kubernetes.default.svc:443 HTTP/1.1\\r\\nHost: x\\r\\n${{auth}}X-Proxyauth-Agent: {uid}\\r\\n\\r\\n\"; sleep 2) \
           | nc {target} 8445 | head -1; done"
    );
    // Any pinned image with a shell and `nc`: the Redis one (Alpine's busybox).
    let alpine = fixture::image("redis");
    let out = Bench::get().admin(&[
        "-n",
        &ns,
        "run",
        "intruder",
        "--image",
        &alpine,
        "--restart=Never",
        "--rm",
        "-i",
        "--quiet",
        "--command",
        "--",
        "sh",
        "-c",
        &script,
    ]);
    let answers = String::from_utf8_lossy(&out.stdout);
    let statuses: Vec<&str> = answers
        .lines()
        .filter(|line| line.starts_with("HTTP/"))
        .collect();
    assert_eq!(statuses.len(), 2, "{answers}");
    assert!(
        statuses.iter().all(|line| line.contains(" 407")),
        "{answers}"
    );
}

/// The gateway's `agent_refused` lines for a TLS failure (no cluster known).
fn tls_refusals() -> usize {
    let out = Bench::get().admin(&[
        "-n",
        STACK_NS,
        "logs",
        "-l",
        &format!("app={GATEWAY}"),
        "--tail=-1",
        "--max-log-requests=10",
    ]);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line["event"] == "agent_refused" && line["reason"] == "tls")
        .count()
}

/// A WebSocket to `/agent/v1/connect` through the Ingress, then a TLS client
/// under the agent CA that presents no certificate. `true` when the gateway
/// ends it before any application data.
async fn client_without_certificate(agent_ca_pem: &[u8]) -> bool {
    use rustls::pki_types::{CertificateDer, ServerName, pem::PemObject as _};
    use tokio_tungstenite::tungstenite::Message;

    // reqwest brings its own provider; these raw configs need the process one.
    drop(rustls::crypto::ring::default_provider().install_default());
    let bench = Bench::get();
    let mut outer_roots = rustls::RootCertStore::empty();
    for cert in CertificateDer::pem_file_iter(bench.ca_path()).expect("run CA") {
        outer_roots
            .add(cert.expect("run CA cert"))
            .expect("add run CA");
    }
    let outer = rustls::ClientConfig::builder()
        .with_root_certificates(outer_roots)
        .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", bench.port))
        .await
        .expect("connect to the Ingress");
    let tls = tokio_rustls::TlsConnector::from(Arc::new(outer))
        .connect(ServerName::try_from(AGENT_HOST).expect("name"), tcp)
        .await
        .expect("TLS to the Ingress");
    let (mut ws, _) = tokio_tungstenite::client_async(
        format!("wss://{AGENT_HOST}:{}/agent/v1/connect", bench.port),
        tls,
    )
    .await
    .expect("the WebSocket upgrade");

    let mut roots = rustls::RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(agent_ca_pem) {
        roots
            .add(cert.expect("agent CA cert"))
            .expect("add agent CA");
    }
    let inner = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let mut conn = rustls::ClientConnection::new(
        Arc::new(inner),
        ServerName::try_from("agent-tunnel.proxyauth.internal").expect("name"),
    )
    .expect("TLS client");
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(15))
        .expect("deadline");
    loop {
        while conn.wants_write() {
            let mut out = Vec::new();
            conn.write_tls(&mut out).expect("TLS bytes");
            if ws.send(Message::Binary(out.into())).await.is_err() {
                return true;
            }
        }
        let next = tokio::time::timeout_at(deadline.into(), ws.next()).await;
        let Ok(Some(Ok(message))) = next else {
            // Closed or reset by the gateway, or nothing at all: no tunnel.
            return next.is_ok();
        };
        let Message::Binary(data) = message else {
            return true;
        };
        let mut slice = data.as_ref();
        while !slice.is_empty() {
            if conn.read_tls(&mut slice).is_err() {
                return true;
            }
        }
        match conn.process_new_packets() {
            // The server refused the handshake (certificate required).
            Err(_) => return true,
            Ok(state) if state.plaintext_bytes_to_read() > 0 => return false,
            Ok(_) => {}
        }
    }
}

/// §10.3, a measurement, not a proof (`--ignored`): the median `kubectl get
/// pods` with and without the agent, and an exec's stdout throughput through
/// the tunnel. Prints its numbers.
#[tokio::test]
#[ignore = "a measurement for RFC 0012 §10.3, run by hand"]
async fn measure_the_agent_overhead() {
    async fn median_ms(p: &Proxied) -> u128 {
        let ns = p.ns.name.clone();
        let mut samples = Vec::new();
        for _ in 0..21 {
            let started = Instant::now();
            let out = p.reader.kubectl(&["get", "pods", "-n", &ns]).await;
            assert!(out.success(), "{out:?}");
            samples.push(started.elapsed().as_millis());
        }
        samples.sort_unstable();
        samples[samples.len() / 2]
    }
    let open = json!({ "enabled": false });
    let direct = fixture::proxied("rfc0012-measure-direct", open.clone()).await;
    let agent = fixture::agent_proxied("rfc0012-measure-agent", open, 1).await;
    let (direct_ms, agent_ms) = (median_ms(&direct).await, median_ms(&agent).await);

    let mut rates = Vec::new();
    for p in [&direct, &agent] {
        let ns = p.ns.name.clone();
        let started = Instant::now();
        let (bytes, code) = p
            .reader
            .kubectl_count(&[
                "exec",
                "web",
                "-n",
                &ns,
                "-c",
                "app",
                "--",
                "dd",
                "if=/dev/zero",
                "bs=1048576",
                "count=256",
            ])
            .await;
        assert_eq!(code, Some(0));
        let seconds = started.elapsed().as_secs_f64();
        rates.push(bytes as f64 / 1_048_576.0 / seconds);
    }
    println!(
        "kubectl get pods, median of 21: direct {direct_ms} ms, through the agent {agent_ms} ms; \
         exec stdout: direct {:.1} MiB/s, through the agent {:.1} MiB/s",
        rates[0], rates[1]
    );
}
