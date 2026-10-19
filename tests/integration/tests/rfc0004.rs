//! RFC 0004 §6.P, on the bench: the exec and attach policy, decided on the
//! query before any byte reaches the cluster, and the channel filter on the
//! tunnel. kubectl where it can express the case, a raw client (through the
//! same Ingress) where the case is a forged request.

use std::time::Duration;

use integration::{
    Bench,
    fixture::{self, CLUSTER, Proxied},
    observe,
};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The rules of the fixture's cluster: discovery, reads on the pods of `ns`
/// (no `exec`, so they grant no exec while exec is restricted), then `extra`.
fn rules(ns: &str, extra: &[Value]) -> Value {
    let mut rules = fixture::discovery_rules();
    rules.push(json!({ "Path": {
        "path": format!("/api/v1/namespaces/{ns}/pods/**"),
        "parametised": true,
    } }));
    rules.extend_from_slice(extra);
    json!({ "enabled": true, "allowed_resources": rules })
}

/// A `Path` rule on `pods/*/{subresource}` of `ns` with `exec`.
fn exec_rule(ns: &str, subresource: &str, exec: Value) -> Value {
    json!({ "Path": {
        "path": format!("/api/v1/namespaces/{ns}/pods/*/{subresource}"),
        "parametised": true,
        "exec": exec,
    } })
}

/// `ls` (any arguments) in `app`, no stdin, no tty.
fn ls_in_app() -> Value {
    json!({ "commands": [["ls", "**"]], "containers": ["app"] })
}

async fn fixture_for(test: &str, extra: impl Fn(&str) -> Vec<Value>) -> Proxied {
    let ns = integration::bench::namespace_name(test);
    fixture::proxied(test, rules(&ns, &extra(&ns))).await
}

/// The audit lines of `ns` for `event`.
fn events(ns: &str, event: &str) -> Vec<Value> {
    observe::audit_lines(ns)
        .into_iter()
        .filter(|line| line["ns"] == ns && line["event"] == event)
        .collect()
}

/// The exec policy decisions of `ns`.
fn exec_decisions(ns: &str) -> Vec<Value> {
    events(ns, "policy_decision")
        .into_iter()
        .filter(|line| line["policy"] == "exec")
        .collect()
}

/// A raw websocket handshake to `path_query` (under the cluster), sent by
/// hand over HTTP/1.1: its status and body.
async fn handshake(p: &Proxied, path_query: &str) -> (StatusCode, String) {
    let bench = Bench::get();
    let token = p.reader.cluster_token(&p.ns.name, CLUSTER).await;
    let response = bench
        .http1()
        .request(
            Method::GET,
            format!(
                "{}/clusters/{}/{CLUSTER}{path_query}",
                bench.base_url(),
                p.ns.name
            ),
        )
        .bearer_auth(token)
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("sec-websocket-protocol", "v5.channel.k8s.io")
        .send()
        .await
        .expect("handshake");
    let status = response.status();
    (status, response.text().await.unwrap_or_default())
}

/// Point 1: an allowed command runs; its `request` line names it, and its
/// `session_closed` counts stdout bytes.
#[tokio::test]
async fn p1_an_allowed_command_runs() {
    let p = fixture_for("rfc0004-p1", |ns| vec![exec_rule(ns, "exec", ls_in_app())]).await;
    let ns = p.ns.name.as_str();
    let ls = p
        .reader
        .kubectl(&["exec", "web", "-n", ns, "-c", "app", "--", "ls", "/"])
        .await;
    assert!(
        ls.success() && ls.stdout.lines().any(|l| l == "bin"),
        "{ls:?}"
    );

    let opened = events(ns, "request")
        .into_iter()
        .find(|line| line["status"] == 101)
        .expect("no 101 request line");
    assert_eq!(opened["exec"]["argv"], json!(["ls", "/"]), "{opened}");
    assert_eq!(opened["exec"]["container"], "app", "{opened}");
    let mut closed = None;
    for _ in 0..20 {
        closed = events(ns, "session_closed").into_iter().next();
        if closed.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let closed = closed.expect("no session_closed line");
    assert!(
        closed["channel_bytes"]["1"].as_u64().is_some_and(|b| b > 0),
        "no stdout bytes: {closed}"
    );
    assert!(
        exec_decisions(ns)
            .iter()
            .any(|line| line["decision"] == "allowed"),
        "no allowed exec decision"
    );
}

/// Point 2, T1: a command outside the list is refused with its reason, also
/// after kubectl's SPDY fallback, and never reaches the cluster.
#[tokio::test]
async fn p2_a_command_outside_the_list_is_refused() {
    let p = fixture_for("rfc0004-p2", |ns| vec![exec_rule(ns, "exec", ls_in_app())]).await;
    let ns = p.ns.name.as_str();
    let sh = p
        .reader
        .kubectl(&["exec", "web", "-n", ns, "-c", "app", "--", "sh", "-c", "id"])
        .await;
    assert!(!sh.success(), "{sh:?}");
    assert!(
        sh.stderr.contains("command [sh -c id] is not allowed"),
        "{sh:?}"
    );
    assert!(!sh.stderr.contains("SPDY is not supported"), "{sh:?}");
    let decisions = exec_decisions(ns);
    assert!(
        decisions
            .iter()
            .any(|line| line["decision"] == "denied" && line["reason"] == "command"),
        "{decisions:?}"
    );
    // Neither the websocket attempt nor the SPDY fallback was relayed.
    assert!(
        !events(ns, "request").iter().any(|line| line["path"]
            .as_str()
            .is_some_and(|p| p.ends_with("/exec"))
            && line["outcome"] == "allowed"),
        "an exec request was relayed"
    );
}

/// Point 3: a tty while the rule allows none is refused before connecting.
#[tokio::test]
async fn p3_a_tty_is_refused() {
    let p = fixture_for("rfc0004-p3", |ns| {
        vec![exec_rule(
            ns,
            "exec",
            json!({ "commands": [["ls", "**"]], "containers": ["app"], "stdin": true }),
        )]
    })
    .await;
    let ns = p.ns.name.clone();
    let (status, body) = handshake(
        &p,
        &format!(
            "/api/v1/namespaces/{ns}/pods/web/exec?command=ls&container=app&stdin=true&tty=true&stdout=true"
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("tty is not allowed"), "{body}");
}

/// Point 4, T6: no container named while containers are restricted.
#[tokio::test]
async fn p4_an_unnamed_container_is_refused() {
    let p = fixture_for("rfc0004-p4", |ns| vec![exec_rule(ns, "exec", ls_in_app())]).await;
    let ns = p.ns.name.clone();
    let (status, body) = handshake(
        &p,
        &format!("/api/v1/namespaces/{ns}/pods/web/exec?command=ls&stdout=true"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("specify -c"), "{body}");
}

/// Point 5: `kubectl cp` to the pod needs stdin (`tar -xmf -`): refused
/// while the rule allows no stdin, even though `tar` is listed.
#[tokio::test]
async fn p5_cp_to_the_pod_is_refused() {
    let p = fixture_for("rfc0004-p5", |ns| {
        vec![exec_rule(
            ns,
            "exec",
            json!({ "commands": [["ls", "**"], ["test", "**"], ["tar", "**"]], "containers": ["app"] }),
        )]
    })
    .await;
    let ns = p.ns.name.as_str();
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("f.txt");
    std::fs::write(&local, "x").unwrap();
    let cp = p
        .reader
        .kubectl(&[
            "cp",
            local.to_str().unwrap(),
            &format!("{ns}/web:/tmp/f.txt"),
            "-c",
            "app",
        ])
        .await;
    assert!(!cp.success(), "{cp:?}");
    assert!(
        exec_decisions(ns)
            .iter()
            .any(|line| line["decision"] == "denied" && line["reason"] == "stdin"),
        "no stdin refusal"
    );
}

/// Point 6, T4: a client that sends a stdin message on an exec without
/// stdin sees its session closed; the audit names the channel.
#[tokio::test]
async fn p6_a_forbidden_channel_closes_the_session() {
    let p = fixture_for("rfc0004-p6", |ns| {
        vec![exec_rule(
            ns,
            "exec",
            json!({ "commands": [["sleep", "**"]], "containers": ["app"] }),
        )]
    })
    .await;
    let ns = p.ns.name.clone();
    let bench = Bench::get();
    let token = p.reader.cluster_token(&ns, CLUSTER).await;
    let response = bench
        .http1()
        .get(format!(
            "{}/clusters/{ns}/{CLUSTER}/api/v1/namespaces/{ns}/pods/web/exec?command=sleep&command=30&container=app&stdout=true",
            bench.base_url()
        ))
        .bearer_auth(token)
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("sec-websocket-protocol", "v5.channel.k8s.io")
        .send()
        .await
        .expect("handshake");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    let mut stream = response.upgrade().await.expect("upgraded connection");
    // A masked binary message on channel 0 (stdin).
    let mask = [1u8, 2, 3, 4];
    let body = [0u8, b'i', b'd', b'\n'];
    let mut frame = vec![0x82, 0x80 | body.len() as u8];
    frame.extend_from_slice(&mask);
    frame.extend(body.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    stream.write_all(&frame).await.unwrap();
    let mut rest = Vec::new();
    let closed = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut rest)).await;
    assert!(closed.is_ok(), "the session stayed open");

    let mut line = None;
    for _ in 0..20 {
        line = events(&ns, "session_closed").into_iter().next();
        if line.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let line = line.expect("no session_closed line");
    assert_eq!(line["close_reason"], "filter", "{line}");
    assert_eq!(line["channel"], 0, "{line}");
}

/// Point 7, T3: a repeated parameter is refused (400).
#[tokio::test]
async fn p7_a_repeated_parameter_is_refused() {
    let p = fixture_for("rfc0004-p7", |ns| vec![exec_rule(ns, "exec", ls_in_app())]).await;
    let ns = p.ns.name.clone();
    let (status, body) = handshake(
        &p,
        &format!(
            "/api/v1/namespaces/{ns}/pods/web/exec?command=ls&container=app&container=sidecar&stdout=true"
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// Point 8, T3: `stdin=` (empty, true for the apiserver) is a stdin.
#[tokio::test]
async fn p8_an_empty_boolean_is_true() {
    let p = fixture_for("rfc0004-p8", |ns| vec![exec_rule(ns, "exec", ls_in_app())]).await;
    let ns = p.ns.name.clone();
    let (status, body) = handshake(
        &p,
        &format!(
            "/api/v1/namespaces/{ns}/pods/web/exec?command=ls&container=app&stdin=&stdout=true"
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("stdin is not allowed"), "{body}");
}

/// Point 9, T8: two rules never combine: `ls` in `app` and `env` in
/// `sidecar` do not allow `env` in `app`.
#[tokio::test]
async fn p9_rules_never_combine() {
    let p = fixture_for("rfc0004-p9", |ns| {
        vec![
            exec_rule(
                ns,
                "exec",
                json!({ "commands": [["ls", "**"]], "containers": ["app"] }),
            ),
            exec_rule(
                ns,
                "exec",
                json!({ "commands": [["env"]], "containers": ["sidecar"] }),
            ),
        ]
    })
    .await;
    let ns = p.ns.name.as_str();
    let env = p
        .reader
        .kubectl(&["exec", "web", "-n", ns, "-c", "app", "--", "env"])
        .await;
    assert!(!env.success(), "{env:?}");
    assert!(
        env.stderr.contains("command [env] is not allowed"),
        "{env:?}"
    );
    let ok = p
        .reader
        .kubectl(&["exec", "web", "-n", ns, "-c", "sidecar", "--", "env"])
        .await;
    assert!(ok.success(), "{ok:?}");
}

/// Point 10, T8: a broad rule without `exec` grants no exec while exec is
/// restricted, in the exec rule's namespace and in any other.
#[tokio::test]
async fn p10_a_broad_rule_grants_no_exec() {
    let p = fixture_for("rfc0004-p10", |ns| {
        vec![
            json!({ "Path": { "path": "/api/v1/namespaces/**", "parametised": true } }),
            exec_rule(ns, "exec", ls_in_app()),
        ]
    })
    .await;
    let ns = p.ns.name.as_str();
    let sh = p
        .reader
        .kubectl(&["exec", "web", "-n", ns, "-c", "app", "--", "sh"])
        .await;
    assert!(!sh.success() && sh.stderr.contains("not allowed"), "{sh:?}");
    let (status, body) = handshake(
        &p,
        "/api/v1/namespaces/kube-system/pods/any/exec?command=sh&container=c&stdout=true",
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("no rule allows exec"), "{body}");
}

/// Point 11: attach needs a rule on `pods/*/attach` with `attach: true`; an
/// exec rule alone does not grant it.
#[tokio::test]
async fn p11_attach_needs_its_own_rule() {
    let refused = fixture_for("rfc0004-p11a", |ns| {
        vec![exec_rule(ns, "exec", ls_in_app())]
    })
    .await;
    let ns = refused.ns.name.clone();
    let (status, body) = handshake(
        &refused,
        &format!("/api/v1/namespaces/{ns}/pods/web/attach?container=app&stdout=true"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let allowed = fixture_for("rfc0004-p11b", |ns| {
        vec![exec_rule(
            ns,
            "attach",
            json!({ "containers": ["app"], "attach": true }),
        )]
    })
    .await;
    let ns = allowed.ns.name.as_str();
    let attach = allowed
        .reader
        .kubectl_for(
            Duration::from_secs(4),
            &["attach", "web", "-n", ns, "-c", "app"],
        )
        .await;
    assert!(attach.code.is_none(), "attach ended early: {attach:?}");
    assert!(
        !attach.stderr.lines().any(|l| l.starts_with("error:")),
        "{attach:?}"
    );
}

/// Point 12, T7: under an exec restriction alone, the kubelet's exec through
/// `nodes/proxy` is refused.
#[tokio::test]
async fn p12_the_kubelet_exec_is_refused() {
    let p = fixture_for("rfc0004-p12", |ns| {
        vec![
            // The allow-list lets nodes/ through: only the exec restriction
            // can close the kubelet's exec.
            json!({ "Path": { "path": "/api/v1/nodes/**", "parametised": true } }),
            exec_rule(ns, "exec", ls_in_app()),
        ]
    })
    .await;
    let node = fixture::node();
    let (status, body) = handshake(
        &p,
        &format!(
            "/api/v1/nodes/{node}/proxy/exec/{}/web/app?command=sh",
            p.ns.name
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body.contains("nodes/proxy is not allowed while ports, verbs or exec are restricted"),
        "{body}"
    );
}

/// Point 13: without any exec rule nothing changes, and the `request` line
/// still names the command.
#[tokio::test]
async fn p13_without_an_exec_rule_nothing_changes() {
    let p = fixture::proxied(
        "rfc0004-p13",
        json!({ "enabled": true, "allowed_resources": [] }),
    )
    .await;
    let ns = p.ns.name.as_str();
    let sh = p
        .reader
        .kubectl(&[
            "exec", "web", "-n", ns, "-c", "app", "--", "sh", "-c", "echo ok",
        ])
        .await;
    assert!(sh.success() && sh.stdout.trim() == "ok", "{sh:?}");
    let opened = events(ns, "request")
        .into_iter()
        .find(|line| line["status"] == 101)
        .expect("no 101 request line");
    assert_eq!(
        opened["exec"]["argv"],
        json!(["sh", "-c", "echo ok"]),
        "{opened}"
    );
    assert!(
        exec_decisions(ns).is_empty(),
        "an exec decision without a policy"
    );
}
