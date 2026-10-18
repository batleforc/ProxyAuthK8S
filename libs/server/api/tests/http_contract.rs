//! RFC 0007 §6.1: the HTTP contract of the server, measured on actix before
//! the move to axum, which must pass it unchanged.
//!
//! The server is started in process through [`serve`], on an ephemeral port,
//! and only spoken to over HTTP (reqwest, raw TCP, raw TLS).

mod harness;

use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::Duration;

use api::metrics::{HttpMetrics, PrometheusRegistry};
use api::server::{AppState, serve};
use common::config::CorsOrigins;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The process-wide meter provider, exporting into this registry: the shared
/// policy counter binds to the global meter on first use.
fn registry() -> prometheus::Registry {
    static REGISTRY: OnceLock<prometheus::Registry> = OnceLock::new();
    REGISTRY
        .get_or_init(|| {
            let registry = prometheus::Registry::new();
            let exporter = opentelemetry_prometheus::exporter()
                .with_registry(registry.clone())
                .build()
                .expect("register the Prometheus exporter");
            let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
                .with_reader(exporter)
                .build();
            opentelemetry::global::set_meter_provider(provider);
            registry
        })
        .clone()
}

fn app_state(cors: CorsOrigins, metrics: bool) -> AppState {
    let registry = registry();
    AppState {
        state: harness::test_state("http://127.0.0.1:1".to_string()),
        prometheus_registry: PrometheusRegistry(metrics.then_some(registry)),
        http_metrics: HttpMetrics::new(&opentelemetry::global::meter("proxyauthk8s")),
        cors_allowed_origins: cors,
        tls: None,
        shutdown_timeout_secs: 1,
    }
}

async fn start(app: AppState) -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve(listener, app));
    addr
}

/// Send raw bytes, return the whole answer (the request asks to close).
async fn raw(addr: SocketAddr, request: &[u8]) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(request).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut out)).await;
    String::from_utf8_lossy(&out).into_owned()
}

fn status_of(response: &str) -> u16 {
    response
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

/// Measures the limits the contract freezes; run with `--ignored --nocapture`.
#[tokio::test]
#[ignore = "measurement helper"]
async fn measure_limits() {
    let addr = start(app_state(CorsOrigins::SameOriginOnly, false)).await;
    for (kind, mut lo, mut hi) in [
        ("value", 64 * 1024usize, 128 * 1024usize),
        ("line", 32 * 1024, 64 * 1024),
    ] {
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            let req = if kind == "value" {
                format!(
                    "GET /management/health HTTP/1.1\r\nhost: x\r\nx-big: {}\r\nconnection: close\r\n\r\n",
                    "a".repeat(mid)
                )
            } else {
                format!(
                    "GET /management/health?{} HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n",
                    "a".repeat(mid)
                )
            };
            if status_of(&raw(addr, req.as_bytes()).await) == 200 {
                lo = mid
            } else {
                hi = mid
            }
        }
        println!("bisect {kind}: max ok {lo}");
    }
    for count in [90, 94, 95, 96, 97, 100] {
        let headers: String = (0..count).map(|i| format!("x-h{i}: v\r\n")).collect();
        let r = raw(
            addr,
            format!(
                "GET /management/health HTTP/1.1\r\nhost: x\r\n{headers}connection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await;
        println!("headers +{count}: {}", status_of(&r));
    }
    for size in [
        8 * 1024,
        16 * 1024,
        32 * 1024,
        64 * 1024,
        128 * 1024,
        256 * 1024,
    ] {
        let value = "a".repeat(size);
        let r = raw(addr, format!("GET /management/health HTTP/1.1\r\nhost: x\r\nx-big: {value}\r\nconnection: close\r\n\r\n").as_bytes()).await;
        println!("header value {size}: {}", status_of(&r));
    }
    for size in [
        8 * 1024,
        16 * 1024,
        32 * 1024,
        64 * 1024,
        128 * 1024,
        256 * 1024,
    ] {
        let path = format!("/management/health?{}", "a".repeat(size));
        let r = raw(
            addr,
            format!("GET {path} HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n").as_bytes(),
        )
        .await;
        println!("request line {size}: {}", status_of(&r));
    }
    for size in [
        16usize * 1024 - 100,
        16 * 1024,
        16 * 1024 + 1,
        32 * 1024,
        300 * 1024,
    ] {
        let body = format!("grant_type=x&code={}", "a".repeat(size.saturating_sub(18)));
        let r = raw(addr, format!("POST /clusters/ns/c/oauth/token HTTP/1.1\r\nhost: x\r\ncontent-type: application/x-www-form-urlencoded\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
        println!(
            "form {}: {} {}",
            body.len(),
            status_of(&r),
            r.lines().last().unwrap_or("")
        );
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_gzip()
        .build()
        .expect("client")
}

/// A raw request on `path`, with `extra` header lines, asking to close.
fn get(path: &str, extra: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nhost: proxy\r\n{extra}connection: close\r\n\r\n")
}

/// The limits actix applies (none is configured), measured by
/// `measure_limits`: hyper and axum must be set to the same.
#[tokio::test]
async fn http_limits() {
    let addr = start(app_state(CorsOrigins::SameOriginOnly, false)).await;

    // 96 header lines in all (host and connection included), 97 refused.
    let headers =
        |count: usize| -> String { (0..count).map(|i| format!("x-h{i}: v\r\n")).collect() };
    assert_eq!(
        status_of(&raw(addr, get("/management/health", &headers(94)).as_bytes()).await),
        200
    );
    assert_eq!(
        status_of(&raw(addr, get("/management/health", &headers(95)).as_bytes()).await),
        431
    );

    // The request head: 128 KiB.
    let head = |value: usize| {
        get(
            "/management/health",
            &format!("x-big: {}\r\n", "a".repeat(value)),
        )
    };
    let fits = 131_072 - head(0).len();
    assert_eq!(head(fits).len(), 131_072);
    assert_eq!(status_of(&raw(addr, head(fits).as_bytes()).await), 200);
    assert_eq!(status_of(&raw(addr, head(fits + 1).as_bytes()).await), 431);

    // The URI: 65 534 bytes (the `http` crate's bound). Beyond, actix
    // answered 400 and hyper answers 414 URI Too Long before any service
    // runs: the one gap the move to axum could not close (RFC 0007 §17).
    let uri = |len: usize| {
        format!(
            "/management/health?{}",
            "a".repeat(len - "/management/health?".len())
        )
    };
    assert_eq!(
        status_of(&raw(addr, get(&uri(65_534), "").as_bytes()).await),
        200
    );
    assert_eq!(
        status_of(&raw(addr, get(&uri(65_535), "").as_bytes()).await),
        414
    );

    // A form body (the OAuth token endpoint): 16 KiB, then 413.
    let form = |len: usize| {
        let body = format!("grant_type=x&code={}", "a".repeat(len - 18));
        format!(
            "POST /clusters/ns/c/oauth/token HTTP/1.1\r\nhost: proxy\r\ncontent-type: application/x-www-form-urlencoded\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    assert_eq!(
        status_of(&raw(addr, form(16_384).as_bytes()).await),
        400,
        "parsed, then refused for its fields"
    );
    assert_eq!(status_of(&raw(addr, form(16_385).as_bytes()).await), 413);
}

#[tokio::test]
async fn management_routes_request_id_and_metrics() {
    let addr = start(app_state(CorsOrigins::SameOriginOnly, true)).await;
    let health = client()
        .get(format!("http://{addr}/management/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    let request_id = health.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(request_id.len(), 36, "a UUID: {request_id}");
    assert_eq!(health.text().await.unwrap(), "");

    let metrics = client()
        .get(format!("http://{addr}/management/metrics"))
        .send()
        .await
        .unwrap();
    assert_eq!(metrics.status(), 200);
    assert_eq!(
        metrics.headers()["content-type"],
        api::metrics::PROMETHEUS_CONTENT_TYPE
    );
    let body = metrics.text().await.unwrap();
    let line = body
        .lines()
        .find(|l| {
            l.starts_with("http_server_request_duration_seconds_count")
                && l.contains("/management/health")
        })
        .unwrap_or_else(|| panic!("no duration for the health route:\n{body}"));
    assert!(
        line.contains(r#"http_route="/management/health""#),
        "{line}"
    );

    let disabled = start(app_state(CorsOrigins::SameOriginOnly, false)).await;
    let res = client()
        .get(format!("http://{disabled}/management/metrics"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn compression() {
    let addr = start(app_state(CorsOrigins::SameOriginOnly, false)).await;
    let res = client()
        .get(format!("http://{addr}/api/docs"))
        .header("accept-encoding", "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["content-encoding"], "gzip");
    let plain = client()
        .get(format!("http://{addr}/api/docs"))
        .send()
        .await
        .unwrap();
    assert!(plain.headers().get("content-encoding").is_none());
}

/// (status, access-control-allow-origin) of a preflight and of a simple GET
/// from `origin`.
async fn cors(addr: SocketAddr, origin: &str) -> ((u16, Option<String>), (u16, Option<String>)) {
    let acao = |res: &reqwest::Response| {
        res.headers()
            .get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap().to_string())
    };
    let preflight = client()
        .request(
            reqwest::Method::OPTIONS,
            format!("http://{addr}/api/v1/clusters"),
        )
        .header("origin", origin)
        .header("access-control-request-method", "GET")
        .send()
        .await
        .unwrap();
    let simple = client()
        .get(format!("http://{addr}/management/health"))
        .header("origin", origin)
        .send()
        .await
        .unwrap();
    (
        (preflight.status().as_u16(), acao(&preflight)),
        (simple.status().as_u16(), acao(&simple)),
    )
}

#[tokio::test]
async fn cors_modes() {
    let same = start(app_state(CorsOrigins::SameOriginOnly, false)).await;
    let any = start(app_state(CorsOrigins::Any, false)).await;
    let list = start(app_state(
        CorsOrigins::List(vec!["https://front.example".to_string()]),
        false,
    ))
    .await;
    let refused = ((400, None), (200, None));
    let echoed = |origin: &str| {
        let origin = Some(origin.to_string());
        ((200, origin.clone()), (200, origin))
    };
    // SameOriginOnly: no cross-origin origin at all (no X-Forwarded-Host
    // logic either: actix-cors with an empty allow-list).
    assert_eq!(cors(same, "https://evil.example").await, refused);
    // Any: the origin is echoed, never `*`.
    assert_eq!(
        cors(any, "https://evil.example").await,
        echoed("https://evil.example")
    );
    assert_eq!(
        cors(list, "https://front.example").await,
        echoed("https://front.example")
    );
    assert_eq!(cors(list, "https://evil.example").await, refused);

    let preflight = client()
        .request(
            reqwest::Method::OPTIONS,
            format!("http://{list}/api/v1/clusters"),
        )
        .header("origin", "https://front.example")
        .header("access-control-request-method", "DELETE")
        .header("access-control-request-headers", "authorization")
        .send()
        .await
        .unwrap();
    assert_eq!(preflight.headers()["access-control-max-age"], "3600");
    // An OPTIONS that is not a preflight (no Origin) is not answered by CORS:
    // it reaches the routes, which answer 404 as for any unrouted method.
    let plain = client()
        .request(
            reqwest::Method::OPTIONS,
            format!("http://{list}/api/v1/clusters"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(plain.status(), 404);
    assert!(
        preflight.headers()["access-control-allow-methods"]
            .to_str()
            .unwrap()
            .contains("DELETE")
    );
}

/// A proxy behind `serve`, restricted like RFC 0003 §1 and RFC 0006 §1, in
/// front of a recording wiremock upstream.
async fn restricted_proxy() -> Option<(
    SocketAddr,
    wiremock::MockServer,
    String,
    deadpool_redis::Pool,
)> {
    use crd::security::{AllowedPathConfiguration, AllowedPathConfigurationEnum, PortSpec, Verb};
    let pool = harness::try_redis_pool().await?;
    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("upstream"))
        .mount(&upstream)
        .await;
    let rule = |path: &str, ports: Option<&str>, verbs: Option<Vec<Verb>>| {
        AllowedPathConfigurationEnum::Path(AllowedPathConfiguration {
            path: path.to_string(),
            parametised: true,
            allowed_ports: ports.map(|p| vec![PortSpec(p.to_string())]),
            verbs,
        })
    };
    let (ns, cluster) = harness::unique_cluster();
    let mut proxy = harness::proxy_fixture(&ns, &cluster, &upstream.uri());
    let mut config = harness::security_config(vec![]);
    config.allowed_resources = vec![
        rule(
            "/api/v1/namespaces/dev/pods/*/portforward",
            Some("8080"),
            None,
        ),
        rule("/api/v1/namespaces/dev/**", None, Some(Verb::READ.to_vec())),
        rule("/api/v1/nodes/**", None, None),
    ];
    proxy.spec.security_config = Some(config);
    harness::seed_proxy(&pool, &proxy).await;

    let mut app = app_state(CorsOrigins::SameOriginOnly, true);
    app.state = harness::test_state(upstream.uri());
    let addr = start(app).await;
    Some((addr, upstream, format!("/clusters/{ns}/{cluster}"), pool))
}

fn message(response: &str) -> String {
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("");
    // Chunked or not, the Status JSON is on one line.
    let json = body.lines().find(|l| l.starts_with('{')).unwrap_or("{}");
    serde_json::from_str::<serde_json::Value>(json).unwrap()["message"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

#[tokio::test]
async fn proxy_refusals_raw_paths_and_headers() {
    let Some((addr, upstream, prefix, pool)) = restricted_proxy().await else {
        return;
    };
    let ns = "/api/v1/namespaces/dev";
    let refused = |r: &str| (status_of(r), message(r));

    // The 403 messages of RFC 0003 and 0006.
    let r = raw(
        addr,
        get(&format!("{prefix}{ns}/pods/web:5432/proxy/"), "").as_bytes(),
    )
    .await;
    assert_eq!(
        refused(&r),
        (403, "port 5432 is not allowed on this cluster".into())
    );
    let r = raw(
        addr,
        get(&format!("{prefix}{ns}/pods/web:http/proxy/"), "").as_bytes(),
    )
    .await;
    assert_eq!(
        refused(&r),
        (
            403,
            "named or missing port is not allowed on this cluster".into()
        )
    );
    let r = raw(
        addr,
        format!(
            "DELETE {prefix}{ns}/pods/web HTTP/1.1\r\nhost: proxy\r\nconnection: close\r\n\r\n"
        )
        .as_bytes(),
    )
    .await;
    assert_eq!(
        refused(&r),
        (
            403,
            format!("verb \"delete\" is not allowed on \"{ns}/pods/web\"")
        )
    );
    let r = raw(
        addr,
        get(
            &format!("{prefix}/api/v1/nodes/n1/proxy/exec/dev/web/app"),
            "",
        )
        .as_bytes(),
    )
    .await;
    assert_eq!(
        refused(&r),
        (
            403,
            "nodes/proxy is not allowed while ports or verbs are restricted".into()
        )
    );

    // kubectl's SPDY fallback gets the policy's reason, not the SPDY refusal;
    // a conforming SPDY handshake gets the 400.
    let spdy = |ports: &str| {
        format!(
            "GET {prefix}{ns}/pods/web/portforward?ports={ports} HTTP/1.1\r\nhost: proxy\r\nupgrade: SPDY/3.1\r\nconnection: Upgrade\r\nconnection: close\r\n\r\n"
        )
    };
    assert_eq!(
        refused(&raw(addr, spdy("5432").as_bytes()).await),
        (403, "port 5432 is not allowed on this cluster".into())
    );
    let conforming = raw(addr, spdy("8080").as_bytes()).await;
    assert_eq!(status_of(&conforming), 400);
    assert!(conforming.contains("SPDY is not supported"), "{conforming}");

    // The raw path text reaches the matchers: an encoded slash or a `..` is
    // refused as typed; a doubly encoded dot is relayed as typed.
    let r = raw(
        addr,
        get(&format!("{prefix}{ns}/pods/web%2Fexec"), "").as_bytes(),
    )
    .await;
    assert_eq!(
        refused(&r),
        (
            403,
            format!("path \"{ns}/pods/web%2Fexec\" is not allowed on this cluster")
        )
    );
    let r = raw(
        addr,
        get(&format!("{prefix}{ns}/pods/../../kube-system/secrets"), "").as_bytes(),
    )
    .await;
    assert_eq!(
        refused(&r),
        (
            403,
            format!("path \"{ns}/pods/../../kube-system/secrets\" is not allowed on this cluster")
        )
    );
    let r = raw(
        addr,
        get(&format!("{prefix}{ns}/configmaps/%252e"), "").as_bytes(),
    )
    .await;
    assert_eq!(status_of(&r), 200);
    let received = upstream.received_requests().await.unwrap_or_default();
    assert_eq!(
        received.len(),
        1,
        "only the allowed request reached the cluster"
    );
    assert_eq!(received[0].url.path(), format!("{ns}/configmaps/%252e"));

    // Identity and method-override headers never reach the cluster.
    let r = raw(
        addr,
        get(
            &format!("{prefix}{ns}/pods"),
            "impersonate-user: admin\r\nx-remote-user: admin\r\nx-forwarded-user: admin\r\nx-http-method-override: DELETE\r\n",
        )
        .as_bytes(),
    )
    .await;
    assert_eq!(status_of(&r), 200);
    let received = upstream.received_requests().await.unwrap_or_default();
    let headers = &received.last().unwrap().headers;
    for name in [
        "impersonate-user",
        "x-remote-user",
        "x-forwarded-user",
        "x-http-method-override",
    ] {
        assert!(!headers.contains_key(name), "{name} was relayed");
    }

    // The shared policy counter, on the Prometheus endpoint.
    let metrics = client()
        .get(format!("http://{addr}/management/metrics"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    for (policy, decision) in [
        ("port", "denied"),
        ("verb", "denied"),
        ("port", "node_proxy_denied"),
    ] {
        assert!(
            metrics
                .lines()
                .any(|l| l.starts_with("proxyauth_policy_decisions_total")
                    && l.contains(&format!(r#"policy="{policy}""#))
                    && l.contains(&format!(r#"decision="{decision}""#))),
            "no {policy}/{decision} in:\n{metrics}"
        );
    }

    let (ns_name, cluster) = prefix
        .trim_start_matches("/clusters/")
        .split_once('/')
        .unwrap();
    harness::delete_proxy(&pool, ns_name, cluster).await;
}

/// The ALPN protocol the TLS listener picks when the client offers `offer`.
async fn negotiated(
    addr: SocketAddr,
    cert: &rcgen::CertifiedKey<rcgen::KeyPair>,
    offer: &[&[u8]],
) -> Option<Vec<u8>> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.cert.der().clone()).unwrap();
    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = offer.iter().map(|p| p.to_vec()).collect();
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
    let tcp = TcpStream::connect(addr).await.unwrap();
    let tls = connector
        .connect("localhost".try_into().unwrap(), tcp)
        .await
        .expect("TLS handshake");
    tls.get_ref().1.alpn_protocol().map(<[u8]>::to_vec)
}

/// The TLS listener, built by `ServerConfig::rustls_config` as in production,
/// announces h2 and http/1.1: kubectl's requests go h2, an upgrade (which
/// only offers http/1.1) stays on http/1.1.
#[tokio::test]
async fn tls_alpn() {
    harness::install_crypto_provider();
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let dir = std::env::temp_dir().join(format!("http-contract-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (cert_path, key_path) = (dir.join("cert.pem"), dir.join("key.pem"));
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.signing_key.serialize_pem()).unwrap();
    let tls = common::ServerConfig {
        port: 0,
        https: true,
        cert_path: Some(cert_path.display().to_string()),
        key_path: Some(key_path.display().to_string()),
    }
    .rustls_config()
    .expect("rustls config");

    let mut app = app_state(CorsOrigins::SameOriginOnly, false);
    app.tls = Some(tls);
    let addr = start(app).await;

    assert_eq!(
        negotiated(addr, &cert, &[b"h2", b"http/1.1"])
            .await
            .as_deref(),
        Some(&b"h2"[..])
    );
    assert_eq!(
        negotiated(addr, &cert, &[b"http/1.1"]).await.as_deref(),
        Some(&b"http/1.1"[..])
    );
    std::fs::remove_dir_all(&dir).ok();
}
