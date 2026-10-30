//! Guard (RFC 0015 §5.3, §6.3): every route the server serves has a row here,
//! with its access class, and every row is a served route. Each row of the
//! main application is then called: the refusal of its class, and a positive
//! control, since a refusal alone also passes when the route is broken.
//!
//! Adding a route? Add a row to `MAIN` (or `LISTENER`), with its class:
//! - `Public`: no caller identity required (probes, login and OAuth flows,
//!   discovery documents); only under a prefix of `PUBLIC_PREFIXES`;
//! - `Authenticated`: a bearer token from the service-wide provider;
//! - `ClusterScoped`: a token for the cluster, and its rules (proxy group);
//! - `AgentEnrolled`: the agents' listener, by certificate or enrolment token.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test helpers: a panic is a failed test, with its message"
)]

mod harness;

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use harness::{
    memory_state, memory_store, mount_oidc_provider, oidc_auth_config, proxy_fixture, seed_proxy,
    unique_cluster,
};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Public,
    Authenticated,
    ClusterScoped,
    AgentEnrolled,
}
use Class::{AgentEnrolled, Authenticated, ClusterScoped, Public};

/// The routes of the application (`api::server::app`).
const MAIN: &[(&str, &str, Class)] = &[
    ("GET", "/management/health", Public),
    ("GET", "/management/ready", Public),
    ("GET", "/management/metrics", Public),
    ("GET", "/api/v1/clusters", Authenticated),
    ("GET", "/api/docs", Public),
    ("GET", "/clusters/{ns}/{cluster}/auth/login", Authenticated),
    ("GET", "/clusters/{ns}/{cluster}/auth/callback", Public),
    (
        "GET",
        "/clusters/{ns}/{cluster}/.well-known/oauth-authorization-server",
        Public,
    ),
    ("GET", "/clusters/{ns}/{cluster}/oauth/authorize", Public),
    ("GET", "/clusters/{ns}/{cluster}/oauth/callback", Public),
    ("GET", "/clusters/{ns}/{cluster}/oauth/jwks", Public),
    ("POST", "/clusters/{ns}/{cluster}/oauth/token", Public),
    ("GET", "/clusters/{ns}/{cluster}/{path}", ClusterScoped),
    ("POST", "/clusters/{ns}/{cluster}/{path}", ClusterScoped),
    ("PUT", "/clusters/{ns}/{cluster}/{path}", ClusterScoped),
    ("PATCH", "/clusters/{ns}/{cluster}/{path}", ClusterScoped),
    ("DELETE", "/clusters/{ns}/{cluster}/{path}", ClusterScoped),
    (
        "GET",
        "/issuer/{ns}/{cluster}/.well-known/openid-configuration",
        Public,
    ),
    ("GET", "/issuer/{ns}/{cluster}/jwks", Public),
];

/// The routes of the agents' listener (`api::agent::listener::router`), with
/// the `agent_tunnel` tests that prove their refusal and their positive control
/// (the listener needs a gateway, CA and registry that live there).
const LISTENER: &[(&str, &str, Class, &str, &str)] = &[
    (
        "GET",
        "/agent/v1/connect",
        AgentEnrolled,
        "a_cert_from_another_ca_is_refused",
        "the_real_agent_opens_a_passthrough_tunnel",
    ),
    (
        "POST",
        "/agent/v1/enroll",
        AgentEnrolled,
        "a_token_for_another_subject_or_audience_is_refused",
        "a_valid_service_account_token_enrols_the_agent",
    ),
    ("GET", "/healthz", Public, "", ""),
    ("GET", "/readyz", Public, "", ""),
];

/// The only places a route may be `Public`: a sensitive route declared public
/// elsewhere fails the guard (RFC 0015 §5.3).
const PUBLIC_PREFIXES: &[&str] = &[
    "/management/",
    "/api/docs",
    "/clusters/{ns}/{cluster}/auth/",
    "/clusters/{ns}/{cluster}/oauth/",
    "/clusters/{ns}/{cluster}/.well-known/",
    "/issuer/",
    "/healthz",
    "/readyz",
];

type Route = (String, String);

/// The application's routes the OpenAPI document marks with `security`: a
/// bearer token is required, so none of them may be `Public`.
fn secured_main() -> BTreeSet<Route> {
    openapi_routes()
        .into_iter()
        .filter_map(|(route, secured)| secured.then_some(route))
        .collect()
}

/// Every operation of the OpenAPI document, and whether it declares
/// `security`. The catch-all is `{path}` in the document and `{*path}` in the
/// router; both are read as `{path}`.
fn openapi_routes() -> Vec<(Route, bool)> {
    let document = serde_json::to_value(api::gen_openapi()).expect("the document serializes");
    let mut routes = Vec::new();
    for (path, item) in document["paths"].as_object().expect("paths") {
        for (verb, operation) in item.as_object().expect("path item") {
            if ["get", "post", "put", "patch", "delete", "head", "options"].contains(&verb.as_str())
            {
                let secured = operation
                    .get("security")
                    .and_then(|s| s.as_array())
                    .is_some_and(|s| !s.is_empty());
                routes.push(((verb.to_uppercase(), path.replace("{*", "{")), secured));
            }
        }
    }
    routes
}

/// Every route of the application: the OpenAPI document, plus what is mounted
/// outside it.
fn served_main() -> BTreeSet<Route> {
    let mut routes: BTreeSet<Route> = openapi_routes().into_iter().map(|(r, _)| r).collect();
    routes.extend(
        api::issuer::ROUTES
            .iter()
            .map(|(m, p)| (m.to_string(), p.to_string())),
    );
    routes.insert(("GET".to_string(), api::server::DOCS_PATH.to_string()));
    routes
}

fn served_listener() -> BTreeSet<Route> {
    api::agent::listener::ROUTES
        .iter()
        .map(|(m, p)| (m.to_string(), p.to_string()))
        .collect()
}

/// The problems the inventory check reports.
fn inventory_problems(
    served: &BTreeSet<Route>,
    rows: &[(&str, &str, Class)],
    table: &str,
) -> Vec<String> {
    let declared: BTreeSet<Route> = rows
        .iter()
        .map(|(m, p, _)| (m.to_string(), p.to_string()))
        .collect();
    let mut problems = Vec::new();
    for (m, p) in served.difference(&declared) {
        problems.push(format!(
            "authz_matrix: {m} {p} is served but has no row in {table} \
             (add one, class: Public|Authenticated|ClusterScoped|AgentEnrolled)"
        ));
    }
    for (m, p) in declared.difference(served) {
        problems.push(format!(
            "authz_matrix: {m} {p} has a row in {table} but is not served; drop the row"
        ));
    }
    for (m, p, class) in rows {
        if *class == Public && !PUBLIC_PREFIXES.iter().any(|prefix| p.starts_with(prefix)) {
            problems.push(format!(
                "authz_matrix: {m} {p} is declared Public but is not under an allowed public prefix"
            ));
        }
    }
    problems
}

#[test]
fn every_served_route_has_a_row_and_every_row_is_served() {
    let listener: Vec<(&str, &str, Class)> = LISTENER
        .iter()
        .map(|(m, p, class, _, _)| (*m, *p, *class))
        .collect();
    let mut problems = inventory_problems(&served_main(), MAIN, "MAIN");
    let secured = secured_main();
    for (m, p, class) in MAIN {
        if *class == Public && secured.contains(&(m.to_string(), p.to_string())) {
            problems.push(format!(
                "authz_matrix: {m} {p} declares security in its #[utoipa::path] but is classed Public"
            ));
        }
    }
    problems.extend(inventory_problems(
        &served_listener(),
        &listener,
        "LISTENER",
    ));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The listener's rows name the `agent_tunnel` tests that prove them: those
/// tests must exist.
#[test]
fn the_listener_rows_name_tests_that_exist() {
    let source = include_str!("agent_tunnel.rs");
    for (m, p, class, refusal, positive) in LISTENER {
        if *class == AgentEnrolled {
            for test in [refusal, positive] {
                assert!(
                    source.contains(&format!("async fn {test}(")),
                    "authz_matrix: {m} {p} names {test}, which agent_tunnel.rs does not declare"
                );
            }
        }
    }
}

/// The guard can fail: a route without a row, a row without a route, and a
/// `Public` row outside the allowed prefixes are each reported.
#[test]
fn the_guard_reports_each_kind_of_gap() {
    let served: BTreeSet<Route> = [
        ("GET".to_string(), "/api/v1/clusters".to_string()),
        ("GET".to_string(), "/api/v1/new".to_string()),
    ]
    .into();
    let rows = [
        ("GET", "/api/v1/clusters", Public),
        ("GET", "/api/v1/gone", Authenticated),
    ];
    let problems = inventory_problems(&served, &rows, "MAIN");
    assert_eq!(problems.len(), 3, "{problems:#?}");
    assert!(problems[0].contains("GET /api/v1/new is served but has no row"));
    assert!(problems[1].contains("GET /api/v1/gone has a row in MAIN but is not served"));
    assert!(problems[2].contains("GET /api/v1/clusters is declared Public"));
}

/// Two clusters on one store: `allowed` admits `alice` (group `operators`),
/// `refused` requires a group `bob` lacks. Each upstream is its own IdP and
/// answers 200 to any other request.
async fn fixtures() -> (axum::Router, (String, String), (String, String), MockServer) {
    let store = memory_store();
    let alice = MockServer::start().await;
    mount_oidc_provider(&alice, "alice", &["operators"]).await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&alice)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&alice)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&alice)
        .await;
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&alice)
        .await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&alice)
        .await;
    let bob = MockServer::start().await;
    mount_oidc_provider(&bob, "bob", &["viewers"]).await;

    let allowed = unique_cluster();
    let mut proxy = proxy_fixture(&allowed.0, &allowed.1, &alice.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&alice.uri()));
    proxy.spec.proxy_group = Some("operators".to_string());
    seed_proxy(&store, &proxy).await;

    let refused = unique_cluster();
    let mut proxy = proxy_fixture(&refused.0, &refused.1, &bob.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&bob.uri()));
    proxy.spec.proxy_group = Some("operators".to_string());
    seed_proxy(&store, &proxy).await;

    let app = api::server::app(&api::server::AppState {
        // The service-wide provider is alice's: `Authenticated` routes resolve her.
        state: memory_state(&store, alice.uri()),
        prometheus_registry: api::metrics::PrometheusRegistry(None),
        http_metrics: api::metrics::HttpMetrics::new(&opentelemetry::global::meter("test")),
        cors_allowed_origins: common::config::CorsOrigins::SameOriginOnly,
        tls: None,
        shutdown_timeout_secs: 1,
    });
    (app, allowed, refused, alice)
}

async fn call(app: &axum::Router, verb: &str, uri: &str, token: Option<&str>) -> StatusCode {
    let mut request = harness::test::TestRequest::default()
        .method(Method::from_bytes(verb.as_bytes()).expect("a method"))
        .uri(uri);
    if let Some(token) = token {
        request = request.insert_header(("authorization", format!("Bearer {token}")));
    }
    harness::test::call_service(app, request.to_request())
        .await
        .status()
}

fn concrete(path: &str, (ns, cluster): &(String, String)) -> String {
    path.replace("{ns}", ns)
        .replace("{cluster}", cluster)
        .replace("{path}", "api/v1/namespaces/dev/pods")
}

/// The token the mocked providers accept: its claims are not verified here,
/// the userinfo endpoint answers for it.
/// Its `aud` is the proxy's client id, which the cluster routes check.
const TOKEN: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJwcm94eWF1dGhrOHMiLCJzdWIiOiJhbGljZS1zdWIifQ.c2lnbmF0dXJlLW5vdC12ZXJpZmllZC1pbi10aGVzZS10ZXN0cw";

#[tokio::test]
async fn every_row_refuses_what_its_class_refuses_and_serves_what_it_serves() {
    harness::install_crypto_provider();
    let (app, allowed, refused, _idp) = fixtures().await;
    let mut problems = Vec::new();
    for (verb, path, class) in MAIN {
        let uri = concrete(path, &allowed);
        let anonymous = call(&app, verb, &uri, None).await;
        match class {
            Public => {
                if matches!(anonymous, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                    problems.push(format!("{verb} {path}: Public, yet {anonymous} anonymous"));
                }
            }
            Authenticated | ClusterScoped => {
                if anonymous != StatusCode::UNAUTHORIZED {
                    problems.push(format!("{verb} {path}: anonymous got {anonymous}, not 401"));
                }
                let with_token = call(&app, verb, &uri, Some(TOKEN)).await;
                if !with_token.is_success() {
                    problems.push(format!(
                        "{verb} {path}: positive control got {with_token}, not a 2xx"
                    ));
                }
            }
            AgentEnrolled => panic!("{verb} {path}: AgentEnrolled belongs to LISTENER"),
        }
        if *class == ClusterScoped {
            let outside = call(&app, verb, &concrete(path, &refused), Some(TOKEN)).await;
            if outside != StatusCode::FORBIDDEN {
                problems.push(format!(
                    "{verb} {path}: a user outside the cluster's rules got {outside}, not 403"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
