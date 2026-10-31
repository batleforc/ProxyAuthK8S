//! RFC 0010 `TokenExchange`, in process (§14): the proxy's IdP, the
//! cluster's IdP and the apiserver are three wiremock servers. One test per
//! STRIDE row of §7 that the proxy path enforces; T8 and T9 are unit tests
//! (`crd::upstream_auth::token_exchange`, `common::token_exchange`), T12
//! lands with the session routes.
//!
//! Each test installs the process-wide configuration (`TokenExchange`
//! allowed, audience enforced), which holds because nextest runs every test
//! in its own process.
#![expect(
    clippy::expect_used,
    reason = "test helpers: a panic is a failed test, with its message"
)]

mod harness;

use std::time::{SystemTime, UNIX_EPOCH};

use api::cluster::redirect::audit::capture;
use axum::http::StatusCode;
use crd::ProxyKubeApi;
use crd::upstream_auth::{TokenExchangeSettings, UpstreamAuth, UpstreamAuthMode};
use harness::{
    memory_state, memory_store, mount_oidc_provider, oidc_auth_config, proxy_fixture, seed_proxy,
    sign_claims, test, unique_cluster,
};
use serde_json::json;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const EXCHANGED: &str = "exchanged-for-the-cluster";

fn allow_token_exchange() {
    let config = common::config::init(common::config::Config::from_lookup(|key| match key {
        "UPSTREAM_AUTH_ALLOWED_MODES" => Some("Passthrough,TokenExchange".to_string()),
        _ => None,
    }));
    assert!(
        config
            .proxy
            .upstream_allowed_modes
            .contains(&UpstreamAuthMode::TokenExchange),
        "another configuration was installed first; run under nextest"
    );
}

/// A token of the proxy's IdP for this proxy (`aud` is the harness's
/// `OIDC_AUDIENCE`), valid ten minutes.
fn proxy_token(audience: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_secs();
    sign_claims(&json!({
        "iss": "https://proxy-idp.example.com",
        "sub": "alice-sub",
        "aud": audience,
        "exp": now.saturating_add(600),
        "iat": now,
    }))
}

struct Bench {
    proxy_idp: MockServer,
    cluster_idp: MockServer,
    apiserver: MockServer,
    store: common::kv_memory::InMemoryKv,
    ns: String,
    cluster: String,
}

impl Bench {
    /// Alice, in group `dev`, on a `TokenExchange` cluster.
    async fn new(customise: impl FnOnce(&mut ProxyKubeApi)) -> Self {
        allow_token_exchange();
        let proxy_idp = MockServer::start().await;
        mount_oidc_provider(&proxy_idp, "alice", &["dev"]).await;
        let cluster_idp = MockServer::start().await;
        mount_oidc_provider(&cluster_idp, "unused", &[]).await;
        let apiserver = MockServer::start().await;
        let store = memory_store();
        let (ns, cluster) = unique_cluster();
        let mut proxy = proxy_fixture(&ns, &cluster, &apiserver.uri());
        proxy.metadata.uid = Some(format!("uid-{ns}-{cluster}"));
        proxy.spec.auth_config = Some(oidc_auth_config(&cluster_idp.uri()));
        proxy.spec.upstream_auth = Some(UpstreamAuth {
            mode: UpstreamAuthMode::TokenExchange,
            token_exchange: Some(TokenExchangeSettings {
                audience: "kube-prod".to_string(),
                ..TokenExchangeSettings::default()
            }),
            ..UpstreamAuth::default()
        });
        customise(&mut proxy);
        seed_proxy(&store, &proxy).await;
        Self {
            proxy_idp,
            cluster_idp,
            apiserver,
            store,
            ns,
            cluster,
        }
    }

    /// The cluster's IdP answers every exchange with `response`, and must
    /// receive exactly `times` of them.
    async fn exchange_answers(&self, response: ResponseTemplate, times: u64) {
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains(
                "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange",
            ))
            .respond_with(response)
            .expect(times)
            .mount(&self.cluster_idp)
            .await;
    }

    async fn exchange_succeeds(&self, times: u64) {
        self.exchange_answers(
            ResponseTemplate::new(200).set_body_json(json!({
                "access_token": EXCHANGED,
                "issued_token_type": "urn:ietf:params:oauth:token-type:access_token",
                "token_type": "Bearer",
                "expires_in": 300,
            })),
            times,
        )
        .await;
    }

    /// The apiserver answers the exchanged token only, `times` times.
    async fn apiserver_expects(&self, times: u64) {
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/dev/pods"))
            .and(header(
                "authorization",
                format!("Bearer {EXCHANGED}").as_str(),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"kind": "PodList", "items": []})),
            )
            .expect(times)
            .mount(&self.apiserver)
            .await;
    }

    async fn get(&self, token: &str) -> (StatusCode, String, String) {
        let app = test::init_service(harness::app(memory_state(
            &self.store,
            self.proxy_idp.uri(),
        )))
        .await;
        let request = test::TestRequest::get()
            .uri(&format!(
                "/clusters/{}/{}/api/v1/namespaces/dev/pods",
                self.ns, self.cluster
            ))
            .insert_header(("authorization", format!("Bearer {token}")))
            .to_request();
        let response = test::call_service(&app, request).await;
        let status = response.status();
        let headers = format!("{:?}", response.headers());
        let body = String::from_utf8_lossy(&test::read_body(response).await).into_owned();
        (status, body, headers)
    }

    fn requests_from(server_requests: Option<Vec<Request>>) -> Vec<Request> {
        server_requests.expect("wiremock records requests")
    }
}

/// The success path: the exchange carries the proxy token and the cluster's
/// client credentials, the apiserver receives the exchanged token, and ten
/// requests cost one exchange (§6.P point 2, in process).
#[tokio::test]
async fn the_proxy_token_is_exchanged_once_then_served_from_the_cache() {
    let bench = Bench::new(|_| {}).await;
    bench.exchange_succeeds(1).await;
    bench.apiserver_expects(10).await;
    let token = proxy_token("proxyauthk8s");
    for _ in 0..10 {
        let (status, body, _) = bench.get(&token).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let exchanges = Bench::requests_from(bench.cluster_idp.received_requests().await);
    let exchange = exchanges
        .iter()
        .find(|request| request.url.path() == "/token")
        .expect("one exchange");
    let form: Vec<(String, String)> =
        serde_urlencoded::from_bytes(&exchange.body).expect("a form body");
    assert!(form.contains(&("subject_token".to_string(), token.clone())));
    assert!(form.contains(&("audience".to_string(), "kube-prod".to_string())));
    // `client_secret_basic` with the cluster's client (`proxyauthk8s:secret`).
    assert_eq!(
        exchange
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Basic cHJveHlhdXRoazhzOnNlY3JldA==")
    );
}

/// A cache hit never waits on the cluster IdP's discovery: once exchanged,
/// the token is served while that IdP is gone.
#[tokio::test]
async fn a_cache_hit_survives_the_cluster_idp_going_away() {
    let bench = Bench::new(|_| {}).await;
    bench.exchange_succeeds(1).await;
    bench.apiserver_expects(2).await;
    let token = proxy_token("proxyauthk8s");
    assert_eq!(bench.get(&token).await.0, StatusCode::OK);
    bench.cluster_idp.reset().await;
    let (status, body, _) = bench.get(&token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// T3: the exchanged token never comes back, in a body or a header.
#[tokio::test]
async fn the_exchanged_token_never_reaches_the_client() {
    let bench = Bench::new(|_| {}).await;
    bench.exchange_succeeds(1).await;
    bench.apiserver_expects(1).await;
    let (status, body, headers) = bench.get(&proxy_token("proxyauthk8s")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains(EXCHANGED));
    assert!(!headers.contains(EXCHANGED));
}

/// T1: a token the proxy's IdP does not accept (here: one of the cluster's
/// IdP) is refused at gate 4; nothing reaches the exchange or the apiserver.
#[tokio::test]
async fn a_cluster_token_is_refused_on_an_exchange_cluster() {
    let bench = Bench::new(|_| {}).await;
    Mock::given(method("GET"))
        .and(path("/userinfo"))
        .respond_with(ResponseTemplate::new(401))
        .with_priority(1)
        .mount(&bench.proxy_idp)
        .await;
    bench.exchange_succeeds(0).await;
    bench.apiserver_expects(0).await;
    let (status, _, _) = bench.get("a-token-of-the-cluster-idp").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// T11: a token of the proxy's IdP issued to another audience is refused at
/// gate 4, never sent to the exchange.
#[tokio::test]
async fn only_a_validated_token_is_sent_to_the_exchange() {
    let bench = Bench::new(|_| {}).await;
    bench.exchange_succeeds(0).await;
    bench.apiserver_expects(0).await;
    let (status, _, _) = bench.get(&proxy_token("another-client")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// T2: a caller outside `proxy_group` is refused before any exchange.
#[tokio::test]
async fn a_non_member_never_reaches_the_idp() {
    let bench = Bench::new(|proxy| proxy.spec.proxy_group = Some("ops".to_string())).await;
    bench.exchange_succeeds(0).await;
    bench.apiserver_expects(0).await;
    let (status, _, _) = bench.get(&proxy_token("proxyauthk8s")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// T6: a rate-limited request costs no exchange (cache off, so each allowed
/// request would exchange).
#[tokio::test]
async fn a_rate_limited_caller_never_reaches_the_idp() {
    let bench = Bench::new(|proxy| {
        proxy.spec.security_config = Some(harness::rate_limited_config(1));
        if let Some(upstream) = proxy.spec.upstream_auth.as_mut()
            && let Some(settings) = upstream.token_exchange.as_mut()
        {
            settings.max_cache_seconds = 0;
        }
    })
    .await;
    bench.exchange_succeeds(1).await;
    bench.apiserver_expects(1).await;
    let token = proxy_token("proxyauthk8s");
    assert_eq!(bench.get(&token).await.0, StatusCode::OK);
    assert_eq!(bench.get(&token).await.0, StatusCode::TOO_MANY_REQUESTS);
}

/// T10, decision 7: an IdP down is `503`, a refusal `401`, garbage `502`;
/// the proxy token never reaches the apiserver, and none counts toward a ban.
#[tokio::test]
async fn an_exchange_failure_never_relays_the_proxy_token() {
    for (answer, expected) in [
        (ResponseTemplate::new(503), StatusCode::SERVICE_UNAVAILABLE),
        (
            ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})),
            StatusCode::UNAUTHORIZED,
        ),
        (
            ResponseTemplate::new(200).set_body_string("<html>"),
            StatusCode::BAD_GATEWAY,
        ),
    ] {
        let bench = Bench::new(|proxy| {
            proxy.spec.security_config = Some(harness::fail2login_config(1, 300));
        })
        .await;
        bench.exchange_answers(answer, 3).await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&bench.apiserver)
            .await;
        let token = proxy_token("proxyauthk8s");
        // Three in a row with a ban after one failure: none is a 429.
        for _ in 0..3 {
            let (status, body, _) = bench.get(&token).await;
            assert_eq!(status, expected, "{body}");
            assert!(!body.contains(&token));
        }
    }
}

/// T7: one `token_exchanged` line per real exchange, none on a cache hit,
/// and never a token in it.
#[tokio::test]
async fn an_exchange_writes_one_audit_event() {
    capture::start();
    let bench = Bench::new(|_| {}).await;
    bench.exchange_succeeds(1).await;
    bench.apiserver_expects(2).await;
    let token = proxy_token("proxyauthk8s");
    assert_eq!(bench.get(&token).await.0, StatusCode::OK);
    assert_eq!(bench.get(&token).await.0, StatusCode::OK);
    let lines: Vec<serde_json::Value> = capture::lines()
        .into_iter()
        .filter(|line| line["cluster"] == bench.cluster.as_str())
        .collect();
    let exchanges: Vec<&serde_json::Value> = lines
        .iter()
        .filter(|line| line["event"] == "token_exchanged")
        .collect();
    assert_eq!(exchanges.len(), 1, "{lines:?}");
    assert_eq!(exchanges[0]["outcome"], "allowed");
    assert_eq!(exchanges[0]["result"], "exchanged");
    assert_eq!(exchanges[0]["user"], "alice");
    assert_eq!(exchanges[0]["upstream_mode"], "TokenExchange");
    let all = serde_json::to_string(&lines).expect("lines serialise");
    assert!(!all.contains(EXCHANGED));
    assert!(!all.contains(&token));
}
