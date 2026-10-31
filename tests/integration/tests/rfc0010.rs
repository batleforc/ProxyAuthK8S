//! RFC 0010 §6.P: the token exchange against two real IdPs — Dex A, the
//! proxy's, and Dex B, the cluster's (`.it/dex/config-b.yaml`), federated to
//! Dex A — with the real kubectl and the CLI of the tree.
//!
//! `it:test` runs this binary alone, one test at a time: point 6 stops Dex
//! B, and the points that count Dex B's exchanges read its shared log.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test harness: a panic is a failed test, with its message"
)]

use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use integration::{
    Bench, Persona, Session,
    bench::STACK_NS,
    cli::browse_to_loopback,
    fixture::{self, CLUSTER, DEX_B_CLIENT, DEX_B_SECRET},
    observe,
};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use url::Url;

/// Dex B's issuer, as the apiserver and the proxy see it.
const DEX_B_ISSUER: &str = "https://dex-b.k8s.localhost:8443";
const EXCHANGE: &str = "proxyauth_token_exchange_total";

/// The audit lines of `ns` for `event`.
fn events(ns: &str, event: &str) -> Vec<Value> {
    observe::audit_lines(ns)
        .into_iter()
        .filter(|line| line["ns"] == ns && line["event"] == event)
        .collect()
}

/// The `token_exchanged` lines of `ns` whose `result` is `result`.
fn exchanges(ns: &str, result: &str) -> usize {
    events(ns, "token_exchanged")
        .iter()
        .filter(|line| line["result"] == result)
        .count()
}

/// The exchanges Dex B served to `email` since `since`: its "token exchange
/// successful" lines (`server/grants/tokenexchange.go`, info level), which
/// carry the user's email.
fn dex_b_exchanges(email: &str, since: Instant) -> usize {
    let since = format!("--since={}s", since.elapsed().as_secs().saturating_add(2));
    let out = Bench::get().admin(&["-n", STACK_NS, "logs", "deployment/dex-b", &since]);
    assert!(
        out.status.success(),
        "Dex B's log: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.contains("token exchange successful") && line.contains(email))
        .count()
}

/// The `token_refreshed` lines with `outcome` (they name no cluster).
fn refreshed(outcome: &str) -> usize {
    observe::audit_lines("token_refreshed")
        .iter()
        .filter(|line| line["event"] == "token_refreshed" && line["outcome"] == outcome)
        .count()
}

/// `redis-cli <args>` in the bench's Redis, as the admin.
fn redis(args: &[&str]) -> String {
    let mut full = vec![
        "-n",
        STACK_NS,
        "exec",
        "deployment/redis",
        "--",
        "redis-cli",
    ];
    full.extend_from_slice(args);
    let out = Bench::get().admin(&full);
    assert!(
        out.status.success(),
        "redis-cli {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The failed authentications counted under `pattern` (fail2login keys,
/// `proxyk8sauth:fail2login:<scope>:<address>`), summed over the addresses.
fn failures(pattern: &str) -> u64 {
    redis(&["--scan", "--pattern", pattern])
        .lines()
        .filter(|key| !key.is_empty())
        .map(|key| redis(&["GET", key]).trim().parse::<u64>().unwrap_or(0))
        .sum()
}

/// A token of Dex B, straight from it: what a user who logged in to the
/// cluster's IdP directly holds. Got by exchanging `subject` (a Dex A
/// token) with the clusters' own client, as the proxy would.
async fn dex_b_token(subject: &str) -> String {
    let bench = Bench::get();
    let response = bench
        .http()
        .post(format!("{}/token", bench.dex_b_url()))
        .basic_auth(DEX_B_CLIENT, Some(DEX_B_SECRET))
        .form(&[
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:token-exchange",
            ),
            ("subject_token", subject),
            (
                "subject_token_type",
                "urn:ietf:params:oauth:token-type:access_token",
            ),
            (
                "requested_token_type",
                "urn:ietf:params:oauth:token-type:access_token",
            ),
            ("connector_id", "proxy"),
            ("scope", "openid email groups"),
        ])
        .send()
        .await
        .expect("Dex B token endpoint");
    let status = response.status();
    let body: Value = response.json().await.expect("Dex B's answer");
    assert!(status.is_success(), "Dex B refused the exchange: {body}");
    body["access_token"]
        .as_str()
        .expect("access_token")
        .to_string()
}

/// The JWTs in `text` whose issuer is Dex B: tokens of the cluster.
fn dex_b_jwts(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .filter(|word| word.starts_with("eyJ") && word.matches('.').count() == 2)
        .filter(|jwt| {
            jwt.split('.')
                .nth(1)
                .and_then(|payload| URL_SAFE_NO_PAD.decode(payload).ok())
                .and_then(|payload| serde_json::from_slice::<Value>(&payload).ok())
                .is_some_and(|claims| claims["iss"] == DEX_B_ISSUER)
        })
        .map(str::to_string)
        .collect()
}

/// What the apiserver says the bearer of `token` is, through the proxy.
async fn self_subject_review(ns: &str, token: &str) -> Value {
    let response = Bench::get()
        .proxied(
            Method::POST,
            ns,
            CLUSTER,
            "/apis/authentication.k8s.io/v1/selfsubjectreviews",
            token,
        )
        .json(&json!({ "apiVersion": "authentication.k8s.io/v1", "kind": "SelfSubjectReview" }))
        .send()
        .await
        .expect("SelfSubjectReview");
    assert!(response.status().is_success(), "{}", response.status());
    response.json().await.expect("JSON")
}

/// RFC 7636 appendix B: a verifier and its `S256` challenge.
const PKCE_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const PKCE_CHALLENGE: &str = "E9Melhoa2OwvFrdMTVr7sydbeXhRQ96HUMEUPNzPMsM";

/// The SSO session to the server (§6.5) over HTTP, the test playing both
/// the browser and the CLI: `/auth/login`, Dex A's form, the code on the
/// loopback redirect, redeemed at `/auth/token`. The session's JSON.
async fn http_session(persona: &Persona) -> Value {
    let bench = Bench::get();
    // Nothing listens there: the redirect is read, not followed.
    let port = 18_099;
    let redirect = format!("http://localhost:{port}/");
    let mut login = Url::parse(&format!("{}/api/v1/auth/login", bench.base_url())).expect("URL");
    login
        .query_pairs_mut()
        .append_pair("redirect_uri", &redirect)
        .append_pair("state", "rfc0010")
        .append_pair("code_challenge", PKCE_CHALLENGE);
    let landed = browse_to_loopback(persona, &login, port).await;
    let code = landed
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, code)| code.into_owned())
        .unwrap_or_else(|| panic!("no code on the loopback redirect: {landed}"));
    let response = bench
        .http()
        .post(format!("{}/api/v1/auth/token", bench.base_url()))
        .form(&[
            ("code", code.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("code_verifier", PKCE_VERIFIER),
        ])
        .send()
        .await
        .expect("/api/v1/auth/token");
    assert_eq!(response.status(), StatusCode::OK);
    response.json().await.expect("the session")
}

/// `POST /api/v1/auth/refresh` with `refresh_token`: its status and body.
async fn refresh(refresh_token: &str) -> (StatusCode, Value) {
    let bench = Bench::get();
    let response = bench
        .http()
        .post(format!("{}/api/v1/auth/refresh", bench.base_url()))
        .form(&[("refresh_token", refresh_token)])
        .send()
        .await
        .expect("/api/v1/auth/refresh");
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

/// Dex B scaled to zero until dropped, then back and ready, so a failed
/// point 6 never leaves the next test without its IdP.
struct DexBStopped;

impl DexBStopped {
    fn stop() -> Self {
        let bench = Bench::get();
        let out = bench.admin(&["-n", STACK_NS, "scale", "deployment/dex-b", "--replicas=0"]);
        assert!(out.status.success(), "scale Dex B to 0");
        bench.admin(&[
            "-n",
            STACK_NS,
            "wait",
            "--for=delete",
            "pod",
            "-l",
            "app=dex-b",
            "--timeout=120s",
        ]);
        DexBStopped
    }
}

impl Drop for DexBStopped {
    fn drop(&mut self) {
        let bench = Bench::get();
        bench.admin(&["-n", STACK_NS, "scale", "deployment/dex-b", "--replicas=1"]);
        bench.admin(&[
            "-n",
            STACK_NS,
            "rollout",
            "status",
            "deployment/dex-b",
            "--timeout=180s",
        ]);
    }
}

/// Point 1: `login --server-url` without `--token` (the SSO session), then
/// `kubectl get pods` on an exchange cluster — it succeeds, the apiserver
/// sees Dex B's user, Dex B logged an exchange, and the persona's keyring
/// holds the server session only, no token of the cluster.
#[tokio::test]
async fn p1_an_sso_session_reaches_an_exchange_cluster() {
    let started = Instant::now();
    let guard = fixture::token_exchange("rfc0010-p1", Value::Null, 300);
    let ns = guard.name.as_str();
    let reader = Session::sso_server_login(&Persona::named("reader")).await;
    reader
        .cluster_login(ns, CLUSTER)
        .await
        .expect("cluster login of an exchange cluster, without SSO");
    let out = reader.kubectl(&["-n", ns, "get", "pods"]).await;
    assert!(out.success() && out.stdout.contains("web"), "{out:?}");

    // The token kubectl gets for the cluster is the server's; the apiserver
    // sees the exchanged one (decision 11: Dex B's `sub`, the groups).
    let token = reader.cluster_token(ns, CLUSTER).await;
    let review = self_subject_review(ns, &token).await;
    let user = &review["status"]["userInfo"];
    assert!(
        user["username"]
            .as_str()
            .is_some_and(|name| name.starts_with("dexb:")),
        "{review}"
    );
    assert!(
        user["groups"]
            .as_array()
            .is_some_and(|groups| groups.contains(&json!("dexb:it-readers"))),
        "{review}"
    );
    assert!(
        dex_b_exchanges(&reader.persona.email, started) >= 1,
        "Dex B logged no exchange for {}",
        reader.persona.email
    );
    let entries = reader.keyring_entries();
    assert!(!entries.is_empty(), "no session in the keyring");
    assert!(
        entries
            .iter()
            .all(|entry| !entry.contains(&format!("::{ns}/"))),
        "a cluster token was stored: {entries:?}"
    );
}

/// Point 2: ten `kubectl get pods` in a row — one exchange (one
/// `token_exchanged` line, one exchange in Dex B's log), the rest served by
/// the cache.
#[tokio::test]
async fn p2_ten_requests_cost_one_exchange() {
    let guard = fixture::token_exchange("rfc0010-p2", Value::Null, 300);
    let ns = guard.name.as_str();
    let reader = Session::server_login(&Persona::named("reader")).await;
    reader
        .cluster_login(ns, CLUSTER)
        .await
        .expect("cluster login");
    let started = Instant::now();
    let (hits, ()) = observe::metric_delta(EXCHANGE, &[("result", "cache_hit")], async {
        for _ in 0..10 {
            let out = reader.kubectl(&["-n", ns, "get", "pods"]).await;
            assert!(out.success(), "{out:?}");
        }
    })
    .await;
    assert_eq!(exchanges(ns, "exchanged"), 1);
    assert_eq!(dex_b_exchanges(&reader.persona.email, started), 1);
    // At least the nine other `get pods`; kubectl's discovery adds more.
    assert!(hits >= 9.0, "cache hits: {hits}");
}

/// Point 3 (refusal): a persona outside `proxy_group` — `403` from the
/// proxy, and not a single exchange, at the proxy or in Dex B's log.
#[tokio::test]
async fn p3_a_non_member_is_refused_before_any_exchange() {
    let started = Instant::now();
    let guard = fixture::token_exchange("rfc0010-p3", Value::Null, 300);
    let ns = guard.name.as_str();
    let nogroup = Persona::named("nogroup");
    let token = nogroup.dashboard_token().await;
    let response = Bench::get()
        .proxied(
            Method::GET,
            ns,
            CLUSTER,
            &format!("/api/v1/namespaces/{ns}/pods"),
            &token,
        )
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(events(ns, "token_exchanged").is_empty());
    assert_eq!(dex_b_exchanges(&nogroup.email, started), 0);
}

/// Point 4 (refusal): a token of Dex B (the cluster's IdP) presented to the
/// proxy for an exchange cluster — `401` at gate 4: no exchange, and every
/// request of the namespace stopped there, so none reached the apiserver.
#[tokio::test]
async fn p4_a_cluster_token_is_refused() {
    let guard = fixture::token_exchange("rfc0010-p4", Value::Null, 300);
    let ns = guard.name.as_str();
    let reader = Persona::named("reader");
    let cluster_token = dex_b_token(&reader.dashboard_token().await).await;
    let response = Bench::get()
        .proxied(
            Method::GET,
            ns,
            CLUSTER,
            &format!("/api/v1/namespaces/{ns}/pods"),
            &cluster_token,
        )
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(events(ns, "token_exchanged").is_empty());
    let requests = events(ns, "request");
    assert!(
        !requests.is_empty()
            && requests
                .iter()
                .all(|line| line["status"] == 401 && line["outcome"] == "denied"),
        "a request went past gate 4: {requests:?}"
    );
}

/// Point 5: `kubectl exec` and `kubectl port-forward` on an exchange
/// cluster — the tunnels open, and the session's audit names the user of
/// the proxy token, not Dex B's.
#[tokio::test]
async fn p5_exec_and_port_forward_go_through_the_exchange() {
    let guard = fixture::token_exchange("rfc0010-p5", Value::Null, 300);
    let ns = guard.name.as_str();
    let reader = Session::server_login(&Persona::named("reader")).await;
    reader
        .cluster_login(ns, CLUSTER)
        .await
        .expect("cluster login");
    let out = reader
        .kubectl(&[
            "-n",
            ns,
            "exec",
            "web",
            "-c",
            "app",
            "--",
            "echo",
            "through-exchange",
        ])
        .await;
    assert!(
        out.success() && out.stdout.contains("through-exchange"),
        "{out:?}"
    );

    let forward = reader
        .port_forward(ns, "pod/web", 8080)
        .await
        .expect("port-forward 8080");
    let response = reqwest::get(format!("http://127.0.0.1:{}/", forward.local))
        .await
        .expect("HTTP through the port-forward");
    assert!(response.status().as_u16() < 500, "{}", response.status());
    drop(forward);

    let mut closed = None;
    for _ in 0..20 {
        closed = events(ns, "session_closed").into_iter().next();
        if closed.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let closed = closed.expect("no session_closed line");
    let persona = &reader.persona;
    assert!(
        closed["user"] == persona.name.as_str() || closed["user"] == persona.email.as_str(),
        "{closed}"
    );
    assert!(exchanges(ns, "exchanged") >= 1);
}

/// Point 6: Dex B stopped — `kubectl get pods` gets `503` (not `401`), and
/// the failures never count toward the cluster's fail2login: past
/// `max_failed_logins`, still `503`, never `429`, and no counter in Redis.
#[tokio::test]
async fn p6_a_stopped_idp_is_503_and_never_bans() {
    let guard = fixture::token_exchange(
        "rfc0010-p6",
        json!({
            "enabled": true,
            "allowed_resources": [],
            "fail2login_equal_ban": { "enabled": true, "max_failed_logins": 2, "ban_duration": 60 },
        }),
        // Every request exchanges: the cache never hides the IdP.
        0,
    );
    let ns = guard.name.as_str();
    let reader = Session::server_login(&Persona::named("reader")).await;
    reader
        .cluster_login(ns, CLUSTER)
        .await
        .expect("cluster login");
    let out = reader.kubectl(&["-n", ns, "get", "pods"]).await;
    assert!(out.success(), "before Dex B stops: {out:?}");

    let stopped = DexBStopped::stop();
    let token = reader.cluster_token(ns, CLUSTER).await;
    let pods = format!("/api/v1/namespaces/{ns}/pods");
    let mut statuses = Vec::new();
    for _ in 0..3 {
        let status = Bench::get()
            .proxied(Method::GET, ns, CLUSTER, &pods, &token)
            .send()
            .await
            .expect("request")
            .status();
        statuses.push(status);
    }
    let out = reader.kubectl(&["-n", ns, "get", "pods"]).await;
    drop(stopped);
    assert_eq!(statuses, [StatusCode::SERVICE_UNAVAILABLE; 3]);
    assert!(
        !out.success() && out.stderr.contains("(ServiceUnavailable)"),
        "kubectl must fail with the 503: {out:?}"
    );
    assert!(exchanges(ns, "unavailable") >= 3);
    assert_eq!(
        failures(&format!("proxyk8sauth:fail2login:{ns}/{CLUSTER}:*")),
        0
    );
}

/// Point 7: the exchanged token never shows — not in what kubectl received
/// (`-v=9` prints every response, headers and bodies), not in the proxy's
/// log. A token of Dex B is recognised by its issuer; the scan is checked on
/// one first.
#[tokio::test]
async fn p7_the_exchanged_token_never_leaves_the_proxy() {
    let guard = fixture::token_exchange("rfc0010-p7", Value::Null, 300);
    let ns = guard.name.as_str();
    let reader = Session::server_login(&Persona::named("reader")).await;
    reader
        .cluster_login(ns, CLUSTER)
        .await
        .expect("cluster login");
    let sample = dex_b_token(&reader.persona.dashboard_token().await).await;
    assert_eq!(dex_b_jwts(&format!("Bearer {sample}\n")), [sample]);

    let out = reader.kubectl(&["-n", ns, "get", "pods", "-v=9"]).await;
    assert!(out.success(), "{out:?}");
    let token = reader.cluster_token(ns, CLUSTER).await;
    let review = self_subject_review(ns, &token).await;
    assert_eq!(exchanges(ns, "exchanged"), 1);

    let log = Bench::get().admin(&[
        "-n",
        STACK_NS,
        "logs",
        &format!("deployment/{}", integration::bench::BACK_SERVICE),
        "--tail=-1",
    ]);
    for (what, text) in [
        ("kubectl's output", format!("{}{}", out.stdout, out.stderr)),
        ("the relayed review", review.to_string()),
        (
            "the proxy's log",
            String::from_utf8_lossy(&log.stdout).into_owned(),
        ),
    ] {
        assert!(dex_b_jwts(&text).is_empty(), "a token of Dex B in {what}");
    }
}

/// Point 8: the session's access token renewed without a browser — the
/// refresh token of the SSO session buys a new access token
/// (`token_refreshed` allowed), which the exchange cluster accepts. Over
/// HTTP: a short access-token lifetime on Dex A would expire the tokens of
/// every other suite.
#[tokio::test]
async fn p8_the_session_is_renewed_without_a_browser() {
    let guard = fixture::token_exchange("rfc0010-p8", Value::Null, 300);
    let ns = guard.name.as_str();
    let session = http_session(&Persona::named("reader")).await;
    let refresh_token = session["refresh_token"]
        .as_str()
        .expect("a refresh token (OIDC_SCOPES has offline_access)");
    let before = refreshed("allowed");
    let (status, renewed) = refresh(refresh_token).await;
    assert_eq!(status, StatusCode::OK, "{renewed}");
    assert_eq!(refreshed("allowed"), before + 1);
    let access = renewed["access_token"].as_str().expect("access_token");
    assert_ne!(access, session["access_token"].as_str().unwrap_or_default());
    let response = Bench::get()
        .proxied(
            Method::GET,
            ns,
            CLUSTER,
            &format!("/api/v1/namespaces/{ns}/pods"),
            access,
        )
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
}

/// Point 9 (refusal): `/api/v1/auth/refresh` with a refresh token Dex A
/// does not know — `400 invalid_grant`, a `token_refreshed` denied line,
/// and one more failure on the address's unscoped fail2login counter.
#[tokio::test]
async fn p9_an_invalid_refresh_token_is_refused_and_counted() {
    let unscoped = "proxyk8sauth:fail2login:_unscoped:*";
    let (before_lines, before_failures) = (refreshed("denied"), failures(unscoped));
    let (status, body) = refresh("not-a-refresh-token").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_grant");
    assert_eq!(refreshed("denied"), before_lines + 1);
    assert_eq!(failures(unscoped), before_failures + 1);
}
