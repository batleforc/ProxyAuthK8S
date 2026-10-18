//! RFC 0008 §6.P, on the bench: the audit trail as a collector reads it from
//! the server's stdout. The bench runs the server with `RUST_LOG=warn`
//! (`.it/values.yaml`): every audit line read here got through that filter.
//! Point 8 (`session_closed`) is phase 4.

use std::time::Duration;

use integration::{
    Bench,
    fixture::{self, CLUSTER, Proxied},
    observe,
};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

async fn unrestricted(test: &str) -> Proxied {
    fixture::proxied(test, json!({ "enabled": true, "allowed_resources": [] })).await
}

/// The audit lines of `ns` for `event`.
fn events(ns: &str, event: &str) -> Vec<Value> {
    observe::audit_lines(ns)
        .into_iter()
        .filter(|line| line["ns"] == ns && line["event"] == event)
        .collect()
}

/// Every line the server printed (audit and logs).
fn server_output() -> String {
    let out = Bench::get().admin(&[
        "-n",
        "proxyauth",
        "logs",
        "deployment/proxyauth-proxyauthk8s-back",
        "--tail=-1",
    ]);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Point 1: with `RUST_LOG=warn`, `kubectl get pods` still writes its
/// `request` line, while the ordinary `info` line of the same request is
/// filtered out.
#[tokio::test]
async fn p1_rust_log_does_not_silence_the_audit() {
    let p = unrestricted("rfc0008-p1").await;
    let ns = p.ns.name.as_str();
    let pods = p.reader.kubectl(&["get", "pods", "-n", ns]).await;
    assert!(pods.success(), "{pods:?}");

    let list = events(ns, "request").into_iter().find(|line| {
        line["k8s_verb"] == "list" && line["path"] == format!("/api/v1/namespaces/{ns}/pods")
    });
    let list = list.expect("no request line for the list");
    assert_eq!(list["audit_schema"], 1);
    assert_eq!(list["outcome"], "allowed");
    assert_eq!(list["status"], 200);
    let forwarded = server_output()
        .lines()
        .filter(|line| line.contains("Forwarding request") && line.contains(ns))
        .count();
    assert_eq!(forwarded, 0, "an info line got through RUST_LOG=warn");
}

/// Point 2: a watch is audited as `watch` on `GET`, with the request id the
/// client received, and a `trace_id` field.
#[tokio::test]
async fn p2_a_watch_is_audited_with_its_ids() {
    let p = unrestricted("rfc0008-p2").await;
    let ns = p.ns.name.as_str();
    let bench = Bench::get();
    let token = p.reader.cluster_token(ns, CLUSTER).await;
    let response = bench
        .http()
        .get(format!(
            "{}/clusters/{ns}/{CLUSTER}/api/v1/namespaces/{ns}/pods?watch=1&timeoutSeconds=1",
            bench.base_url()
        ))
        .bearer_auth(token)
        .send()
        .await
        .expect("watch");
    assert_eq!(response.status(), StatusCode::OK);
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .expect("x-request-id")
        .to_string();
    let _ = response.text().await;

    let watch = events(ns, "request")
        .into_iter()
        .find(|line| line["request_id"] == request_id.as_str())
        .unwrap_or_else(|| panic!("no request line with request_id {request_id}"));
    assert_eq!(watch["k8s_verb"], "watch");
    assert_eq!(watch["http_method"], "GET");
    // The bench runs with `RUST_LOG=warn`, which drops the info spans: there
    // is no trace, and `trace_id` says so with `null` rather than a made-up id.
    assert!(watch.get("trace_id").is_some(), "{watch}");
}

/// Point 3: the SSO login of the fixture (`kubectl proxyauth login`) left an
/// `auth_login` and an `auth_callback`, both allowed.
#[tokio::test]
async fn p3_a_login_is_audited() {
    let p = unrestricted("rfc0008-p3").await;
    let ns = p.ns.name.as_str();
    let logins = events(ns, "auth_login");
    assert!(
        logins
            .iter()
            .any(|line| line["outcome"] == "allowed" && line["user"] != "-"),
        "{logins:?}"
    );
    let callbacks = events(ns, "auth_callback");
    assert!(
        callbacks
            .iter()
            .any(|line| line["outcome"] == "allowed" && line["user"] != "-"),
        "{callbacks:?}"
    );
}

/// Point 4: `/oauth/token` with an invalid code is a `400` and an
/// `oauth_token_denied` line.
#[tokio::test]
async fn p4_an_invalid_code_is_denied() {
    let p = unrestricted("rfc0008-p4").await;
    let ns = p.ns.name.as_str();
    let patch = json!({ "spec": { "auth_config": { "oidc_provider": {
        "expose_oauth_authorization_server": true } } } });
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
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let bench = Bench::get();
    let token_url = format!("{}/clusters/{ns}/{CLUSTER}/oauth/token", bench.base_url());
    let mut status = StatusCode::NOT_FOUND;
    // The controller copies the patched spec into Redis asynchronously.
    for _ in 0..30 {
        status = bench
            .http()
            .post(&token_url)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", "not-a-code"),
                ("redirect_uri", "http://127.0.0.1:8000/"),
                (
                    "code_verifier",
                    "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
                ),
            ])
            .send()
            .await
            .expect("token request")
            .status();
        if status != StatusCode::NOT_FOUND {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let denied = events(ns, "oauth_token_denied");
    assert!(
        denied.iter().any(|line| line["reason"] == "invalid_grant"),
        "{denied:?}"
    );
}

/// Point 5: past `max_failed_logins`, a `client_banned` line, then the
/// requests are `429`.
#[tokio::test]
async fn p5_a_ban_is_audited() {
    let p = fixture::proxied(
        "rfc0008-p5",
        json!({
            "enabled": true,
            "allowed_resources": [],
            "fail2login_equal_ban": { "enabled": true, "max_failed_logins": 2, "ban_duration": 60 },
        }),
    )
    .await;
    let ns = p.ns.name.as_str();
    let bench = Bench::get();
    let url = format!(
        "{}/clusters/{ns}/{CLUSTER}/api/v1/namespaces/{ns}/pods",
        bench.base_url()
    );
    let mut statuses = Vec::new();
    for _ in 0..3 {
        let status = bench
            .http()
            .get(&url)
            .bearer_auth("not-a-token")
            .send()
            .await
            .expect("request")
            .status();
        statuses.push(status);
    }
    assert_eq!(
        statuses,
        [
            StatusCode::UNAUTHORIZED,
            StatusCode::UNAUTHORIZED,
            StatusCode::TOO_MANY_REQUESTS
        ]
    );
    let bans = events(ns, "client_banned");
    assert_eq!(bans.len(), 1, "{bans:?}");
    assert_eq!(bans[0]["ban_seconds"], 60);
    assert!(
        bans[0]["client_ip"]
            .as_str()
            .is_some_and(|ip| !ip.is_empty())
    );
    assert!(
        events(ns, "request")
            .iter()
            .any(|line| line["status"] == 429),
        "no 429 request line"
    );
}

/// Point 6: `kubectl delete pod` under a read-only rule: a `policy_decision`
/// (verb, denied, `delete`) and a `request` in `403`.
#[tokio::test]
async fn p6_a_refused_verb_is_audited() {
    let test = "rfc0008-p6";
    let ns = integration::bench::namespace_name(test);
    let mut rules = fixture::discovery_rules();
    rules.push(json!({ "Path": {
        "path": format!("/api/v1/namespaces/{ns}/pods/**"),
        "parametised": true,
        "verbs": ["get", "list", "watch"],
    } }));
    let p = fixture::proxied(test, json!({ "enabled": true, "allowed_resources": rules })).await;
    let delete = p.reader.kubectl(&["delete", "pod", "web", "-n", &ns]).await;
    assert!(!delete.success(), "{delete:?}");

    let decision = events(&ns, "policy_decision")
        .into_iter()
        .find(|line| line["policy"] == "verb")
        .expect("no verb policy_decision");
    assert_eq!(decision["decision"], "denied");
    assert_eq!(decision["outcome"], "denied");
    assert_eq!(decision["k8s_verb"], "delete");
    assert!(
        events(&ns, "request")
            .iter()
            .any(|line| line["status"] == 403 && line["k8s_verb"] == "delete"),
        "no 403 request line"
    );
}

/// Point 7: `kubectl exec` on a `Pending` pod: kubectl prints the proxy's
/// pointer to `-v=6`, and the websocket attempt's `request` line carries the
/// apiserver's reason.
#[tokio::test]
async fn p7_the_upstream_refusal_reason_is_kept() {
    let p = unrestricted("rfc0008-p7").await;
    let ns = p.ns.name.as_str();
    let pod = json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": "pending", "namespace": ns },
        "spec": {
            "nodeSelector": { "proxyauth-it": "nowhere" },
            "containers": [{ "name": "c", "image": fixture::image("busybox"), "command": ["sleep", "3600"] }],
        },
    });
    Bench::get().apply(&pod.to_string());

    let exec = p
        .reader
        .kubectl(&["exec", "pending", "-n", ns, "--", "ls"])
        .await;
    assert!(!exec.success(), "{exec:?}");
    assert!(
        exec.stderr
            .contains("kubectl -v=6 shows the upstream reason"),
        "{exec:?}"
    );
    let refused = events(ns, "request")
        .into_iter()
        .find(|line| {
            line["path"] == format!("/api/v1/namespaces/{ns}/pods/pending/exec")
                && line["upstream_reason"].is_string()
        })
        .expect("no request line with an upstream_reason");
    assert!(
        refused["upstream_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("does not have a host assigned")),
        "{refused}"
    );
}

/// Point 9: the caller's bearer token never reaches the server's output.
#[tokio::test]
async fn p9_the_token_never_reaches_the_output() {
    let p = unrestricted("rfc0008-p9").await;
    let ns = p.ns.name.as_str();
    let token = p.reader.cluster_token(ns, CLUSTER).await;
    let bench = Bench::get();
    let response = bench
        .http()
        .request(
            Method::GET,
            format!(
                "{}/clusters/{ns}/{CLUSTER}/api/v1/namespaces/{ns}/pods",
                bench.base_url()
            ),
        )
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!events(ns, "request").is_empty());
    assert!(
        !server_output().contains(&token),
        "the bearer token reached the server's output"
    );
}
