//! RFC 0007 §6.P, on the bench: the server on axum and its upgrade tunnel on
//! hyper, driven by kubectl and raw clients through the Ingress. Points 1, 2
//! and 9 are proven off the bench (the contract, the `swagger.json` diff, a
//! manual throughput run).

use std::time::Duration;

use integration::{
    Bench,
    fixture::{self, CLUSTER, Proxied},
    observe,
};
use reqwest::{Method, StatusCode};
use serde_json::json;

/// No restriction: every kubectl operation of point 3 is allowed.
async fn unrestricted(test: &str) -> Proxied {
    fixture::proxied(test, json!({ "enabled": true, "allowed_resources": [] })).await
}

/// A pod that prints a numbered line every second, for `logs -f`.
fn start_ticker(ns: &str) {
    let busybox = fixture::image("busybox");
    let pod = json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": "ticker", "namespace": ns },
        "spec": { "containers": [{
            "name": "ticker",
            "image": busybox,
            "command": ["sh", "-c", "i=0; while true; do echo tick-$i; i=$((i+1)); sleep 1; done"],
        }] },
    });
    let bench = Bench::get();
    bench.apply(&pod.to_string());
    let out = bench.admin(&[
        "-n",
        ns,
        "wait",
        "pod/ticker",
        "--for=condition=Ready",
        "--timeout=120s",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Point 3: exec, attach, cp both ways, port-forward, `logs -f`, and a watch
/// that delivers its events as they happen.
#[tokio::test]
async fn p3_kubectl_streams_work_through_the_tunnel() {
    let p = unrestricted("rfc0007-p3").await;
    let ns = p.ns.name.as_str();

    let exec = p
        .reader
        .kubectl(&["exec", "web", "-n", ns, "-c", "app", "--", "ls", "/"])
        .await;
    assert!(
        exec.success() && exec.stdout.lines().any(|l| l == "bin"),
        "exec: {exec:?}"
    );

    // `attach` to a quiet container: the session opens and stays up.
    let attach = p
        .reader
        .kubectl_for(
            Duration::from_secs(4),
            &["attach", "web", "-n", ns, "-c", "app"],
        )
        .await;
    assert!(attach.code.is_none(), "attach ended early: {attach:?}");
    assert!(
        !attach.stderr.to_lowercase().contains("error"),
        "attach: {attach:?}"
    );

    // cp there and back: tar over exec, both directions.
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("payload.txt");
    std::fs::write(&local, "through the tunnel\n").unwrap();
    let local = local.to_str().unwrap();
    let up = p
        .reader
        .kubectl(&[
            "cp",
            local,
            &format!("{ns}/web:/tmp/payload.txt"),
            "-c",
            "app",
        ])
        .await;
    assert!(up.success(), "cp up: {up:?}");
    let back = dir.path().join("back.txt");
    let down = p
        .reader
        .kubectl(&[
            "cp",
            &format!("{ns}/web:/tmp/payload.txt"),
            back.to_str().unwrap(),
            "-c",
            "app",
        ])
        .await;
    assert!(down.success(), "cp down: {down:?}");
    assert_eq!(
        std::fs::read_to_string(&back).unwrap(),
        "through the tunnel\n"
    );

    let forward = p
        .reader
        .port_forward(ns, "pod/web", 8080)
        .await
        .expect("port-forward 8080");
    let answered = reqwest::get(format!("http://127.0.0.1:{}/", forward.local)).await;
    assert!(answered.is_ok(), "port-forward: {answered:?}");
    drop(forward);

    // The watch is open before the pod exists: its line can only come from
    // the stream, event by event.
    let watch_args = ["get", "pods", "-w", "-n", ns];
    let watch = p.reader.kubectl_for(Duration::from_secs(10), &watch_args);
    let late = async {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let busybox = fixture::image("busybox");
        Bench::get().apply(
            &json!({
                "apiVersion": "v1",
                "kind": "Pod",
                "metadata": { "name": "late", "namespace": ns },
                "spec": { "containers": [{ "name": "c", "image": busybox, "command": ["sleep", "3600"] }] },
            })
            .to_string(),
        );
    };
    let (watch, ()) = tokio::join!(watch, late);
    assert!(
        watch.stdout.contains("late"),
        "the watch missed the new pod: {watch:?}"
    );

    // Last: `logs -f` needs inotify instances on the node (it:up warns).
    start_ticker(ns);
    let logs = p
        .reader
        .kubectl_for(Duration::from_secs(4), &["logs", "-f", "ticker", "-n", ns])
        .await;
    let ticks = logs
        .stdout
        .lines()
        .filter(|l| l.starts_with("tick-"))
        .count();
    assert!(ticks >= 2, "logs -f did not stream: {logs:?}");
}

/// Point 4: a port outside the policy is refused and never reached; the
/// audit names it (the kubectl message is RFC 0003 §17's).
#[tokio::test]
async fn p4_a_port_forward_outside_the_policy_is_refused() {
    let test = "rfc0007-p4";
    let ns = integration::bench::namespace_name(test);
    let mut rules = fixture::discovery_rules();
    rules.push(json!({ "Path": {
        "path": format!("/api/v1/namespaces/{ns}/pods/**"),
        "parametised": true,
        "allowed_ports": ["8080"],
    } }));
    let p = fixture::proxied(test, json!({ "enabled": true, "allowed_resources": rules })).await;
    let refused = match p.reader.port_forward(&p.ns.name, "pod/web", 9090).await {
        Err(out) => out,
        Ok(forward) => {
            let reached = reqwest::get(format!("http://127.0.0.1:{}/", forward.local)).await;
            assert!(reached.is_err(), "9090 answered: {reached:?}");
            forward.exit(Duration::from_secs(15)).await
        }
    };
    assert_ne!(refused.code, Some(0), "{refused:?}");
    let denied = observe::audit_lines(&p.ns.name).into_iter().any(|e| {
        e["fields"]["ns"] == p.ns.name.as_str()
            && e["fields"]["decision"] == "port_denied"
            && e["fields"]["port"] == 9090
    });
    assert!(denied, "no port_denied port=9090 in the audit");
}

/// An upgrade handshake to the exec of `web`, sent by hand over HTTP/1.1.
async fn upgrade(p: &Proxied, protocol: &str, body: Option<&'static str>) -> (StatusCode, String) {
    let bench = Bench::get();
    let token = p.reader.cluster_token(&p.ns.name, CLUSTER).await;
    let path = format!(
        "/clusters/{}/{CLUSTER}/api/v1/namespaces/{}/pods/web/exec?command=ls&container=app&stdout=true",
        p.ns.name, p.ns.name
    );
    let mut request = bench
        .http1()
        .request(Method::GET, format!("{}{path}", bench.base_url()))
        .bearer_auth(token)
        .header("connection", "Upgrade")
        .header("upgrade", protocol)
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("sec-websocket-protocol", "v5.channel.k8s.io");
    if let Some(body) = body {
        request = request.body(body);
    }
    let response = request.send().await.expect("upgrade request");
    let status = response.status();
    (status, response.text().await.unwrap_or_default())
}

/// Point 5: an upgrade that declares a body is refused (400): no second
/// request can ride on the handshake.
#[tokio::test]
async fn p5_an_upgrade_with_a_body_is_refused() {
    let p = unrestricted("rfc0007-p5").await;
    let (status, body) = upgrade(&p, "websocket", Some("0123456789")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("must not carry a body"), "{body}");
}

/// Point 6: a SPDY upgrade within the policies is refused with its reason.
#[tokio::test]
async fn p6_a_spdy_upgrade_is_refused() {
    let p = unrestricted("rfc0007-p6").await;
    let (status, body) = upgrade(&p, "SPDY/3.1", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("SPDY is not supported"), "{body}");
}

/// Point 7: a CORS preflight from an origin that is not listed is refused,
/// and carries no `Access-Control-Allow-Origin`.
#[tokio::test]
async fn p7_a_foreign_cors_preflight_is_refused() {
    let bench = Bench::get();
    let response = bench
        .http()
        .request(
            Method::OPTIONS,
            format!("{}/api/v1/clusters", bench.base_url()),
        )
        .header("origin", "https://evil.example")
        .header("access-control-request-method", "GET")
        .send()
        .await
        .expect("preflight");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
}

/// Point 8: `/management/metrics` keeps the HTTP server histogram and the
/// proxy route's `http.route` label as it was under actix.
#[tokio::test]
async fn p8_the_http_metrics_keep_their_route_label() {
    let p = unrestricted("rfc0007-p8").await;
    let pods = p
        .reader
        .kubectl(&["get", "pods", "-n", &p.ns.name, "-o", "name"])
        .await;
    assert!(pods.success(), "{pods:?}");
    let metrics = observe::metrics();
    let line = metrics
        .lines()
        .find(|l| {
            l.starts_with("http_server_request_duration_seconds_count")
                && l.contains(r#"http_route="/clusters/{ns}/{cluster}/{path:.*}""#)
                && l.contains(r#"http_request_method="GET""#)
        })
        .unwrap_or_else(|| panic!("no GET series on the proxy route:\n{metrics}"));
    assert!(line.contains("http_response_status_code"), "{line}");
}
