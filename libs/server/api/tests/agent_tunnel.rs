//! RFC 0012: clusters reached through an agent tunnel.
//!
//! An `Agent` cluster has no address the proxy may dial: without a tunnel it
//! is refused with `503 agent unavailable`, never reached directly.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a panic is a failed test, with its message"
)]

mod harness;

use axum::http::StatusCode;
use crd::certificate::CertSource;
use crd::service::{
    AgentBootstrap, AgentService, DEFAULT_AUDIENCE, DEFAULT_SERVER_NAME, Service,
    ServiceAccountTokenBootstrap,
};
use harness::test;
use harness::{
    delete_proxy, proxy_fixture, seed_proxy, test_state, try_redis_pool, unique_cluster,
};

macro_rules! redis_or_skip {
    () => {
        match try_redis_pool().await {
            Some(pool) => pool,
            None => return,
        }
    };
}

fn agent_service() -> Service {
    Service::Agent(AgentService {
        server_name: DEFAULT_SERVER_NAME.to_string(),
        epoch: 0,
        bootstrap: AgentBootstrap::ServiceAccountToken(ServiceAccountTokenBootstrap {
            subject: "system:serviceaccount:proxyauth-agent:proxyauth-agent".to_string(),
            audience: DEFAULT_AUDIENCE.to_string(),
            jwks: None,
            jwks_url: Some("https://issuer.example.com/keys".to_string()),
            jwks_from: None,
        }),
    })
}

/// §6.P point 4: no tunnel, `503 agent unavailable`, and no fallback to the
/// address `url_to_call` names (`https://kubernetes.default.svc`, which from
/// the proxy's pod would be its own cluster).
#[tokio::test]
async fn an_agent_cluster_without_a_tunnel_is_refused_and_never_dialled() {
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://unused.example.com");
    proxy.spec.service = agent_service();
    proxy.spec.cert = CertSource::ConfigMap {
        name: "prod-ca".to_string(),
        key: "ca.crt".to_string(),
        namespace: None,
    };
    seed_proxy(&pool, &proxy).await;

    let app = test::init_service(harness::app(test_state(
        "https://idp.example.com".to_string(),
    )))
    .await;
    for request in [
        test::TestRequest::get().uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods")),
        // An upgrade takes another path after the gates; the refusal comes first.
        test::TestRequest::get()
            .uri(&format!(
                "/clusters/{ns}/{cluster}/api/v1/namespaces/default/pods/web/exec?command=sh&stdout=true"
            ))
            .insert_header(("connection", "Upgrade"))
            .insert_header(("upgrade", "websocket")),
    ] {
        let resp = test::call_service(&app, request.to_request()).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["kind"], "Status");
        assert_eq!(body["message"], "agent unavailable");
    }

    delete_proxy(&pool, &ns, &cluster).await;
}

/// A stand-in apiserver: TLS for `kubernetes.default.svc` under its own CA,
/// answering every request with what it received. Returns its address and the
/// CA, PEM.
async fn fake_apiserver() -> (std::net::SocketAddr, String) {
    use http_body_util::Full;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;

    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(ca_params, ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec![DEFAULT_SERVER_NAME.to_string()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            rustls::pki_types::PrivateKeyDer::try_from(leaf_key.serialize_der()).unwrap(),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(tls));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let echo = service_fn(
                    |mut req: hyper::Request<hyper::body::Incoming>| async move {
                        if req.headers().contains_key("upgrade") {
                            // An exec: switch protocols, then echo every byte.
                            let on_upgrade = hyper::upgrade::on(&mut req);
                            tokio::spawn(async move {
                                if let Ok(upgraded) = on_upgrade.await {
                                    let (mut read, mut write) =
                                        tokio::io::split(TokioIo::new(upgraded));
                                    drop(tokio::io::copy(&mut read, &mut write).await);
                                }
                            });
                            let response = hyper::Response::builder()
                                .status(101)
                                .header("upgrade", "websocket")
                                .header("connection", "Upgrade")
                                .body(Full::new(bytes::Bytes::new()))
                                .unwrap();
                            return Ok::<_, std::convert::Infallible>(response);
                        }
                        let auth = req
                            .headers()
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("none")
                            .to_string();
                        let body = format!("apiserver saw {} auth={auth}", req.uri().path());
                        Ok(hyper::Response::new(Full::new(bytes::Bytes::from(body))))
                    },
                );
                drop(
                    hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(tls), echo)
                        .with_upgrades()
                        .await,
                );
            });
        }
    });
    (addr, ca_cert.pem())
}

/// An agent: every stream the gateway opens is spliced to `upstream`, the
/// agent's one destination. Returns the gateway end of the tunnel.
fn fake_agent(upstream: std::net::SocketAddr) -> agent_proto::mux::MuxHandle {
    use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

    let (gateway_io, agent_io) = tokio::io::duplex(1 << 20);
    let (handle, _inbound, driver) =
        agent_proto::mux::new(gateway_io.compat(), agent_proto::mux::Mode::Client, 8);
    let (_agent, mut agent_in, agent_driver) =
        agent_proto::mux::new(agent_io.compat(), agent_proto::mux::Mode::Server, 8);
    tokio::spawn(driver);
    tokio::spawn(agent_driver);
    tokio::spawn(async move {
        while let Some(stream) = agent_in.recv().await {
            tokio::spawn(async move {
                let Ok(mut tcp) = tokio::net::TcpStream::connect(upstream).await else {
                    return;
                };
                let mut stream = stream.compat();
                drop(tokio::io::copy_bidirectional(&mut stream, &mut tcp).await);
            });
        }
    });
    handle
}

/// §6.P point 1, in process: the relay reaches an apiserver it has no route
/// to, through a gateway `CONNECT` and an agent tunnel, with the user's
/// `Authorization` carried inside the end-to-end TLS.
#[tokio::test]
async fn a_request_crosses_the_gateway_and_the_tunnel_to_the_apiserver() {
    let pool = redis_or_skip!();
    let (state, ns, cluster, _tunnel) = agent_cluster(&pool).await;

    let app = test::init_service(harness::app(state)).await;
    let request = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", "Bearer user-token"))
        .to_request();
    let resp = test::call_service(&app, request).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = test::read_body(resp).await;
    assert_eq!(
        String::from_utf8_lossy(&body),
        "apiserver saw /api/v1/pods auth=Bearer user-token"
    );

    delete_proxy(&pool, &ns, &cluster).await;
}

/// An `Agent` cluster behind a running gateway and agent, seeded in Redis;
/// the returned state routes through that gateway.
async fn agent_cluster(
    pool: &deadpool_redis::Pool,
) -> (
    common::State,
    String,
    String,
    api::agent::registry::TunnelGuard,
) {
    use base64::Engine as _;
    use common::agent_gateway::{GatewayClient, presence_key};

    harness::install_crypto_provider();
    let (apiserver, ca_pem) = fake_apiserver().await;

    let (ns, cluster) = unique_cluster();
    let uid = format!("{:08x}-0000-4000-8000-000000000000", std::process::id());
    let registry = api::agent::registry::TunnelRegistry::new();
    let tunnel = registry
        .register(
            agent_pki::AgentIdentity {
                namespace: ns.clone(),
                name: cluster.clone(),
                uid: uid.clone(),
                epoch: 0,
            },
            fake_agent(apiserver),
            4,
            8,
        )
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gateway_addr = listener.local_addr().unwrap();
    tokio::spawn(api::agent::connect::serve(
        listener,
        api::agent::connect::ConnectService::new(registry, Some("t0ken".to_string())),
    ));

    let mut state = test_state("https://idp.example.com".to_string());
    let gateway = GatewayClient::with(Some("gw.test".to_string()), 0, Some("t0ken".to_string()));
    *gateway.fixed_address.lock().unwrap() = Some(gateway_addr.to_string());
    state.agent_gateway = std::sync::Arc::new(gateway);
    state
        .presence_refresh(&presence_key(&uid), "gw-0", 30)
        .await
        .unwrap();

    let mut proxy = proxy_fixture(&ns, &cluster, "https://unused.example.com");
    proxy.spec.service = agent_service();
    proxy.spec.cert = CertSource::Cert(base64::prelude::BASE64_STANDARD.encode(&ca_pem));
    proxy.metadata.uid = Some(uid.clone());
    seed_proxy(pool, &proxy).await;
    // The tunnel stays registered while the caller holds the guard.
    (state, ns, cluster, tunnel)
}

/// §6.P point 2, in process: an exec handshake and its bytes cross the
/// gateway `CONNECT`, then the TLS to the apiserver, on the upgrade path.
#[tokio::test]
async fn an_exec_upgrade_crosses_the_gateway_and_the_tunnel() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let pool = redis_or_skip!();
    let (state, ns, cluster, _tunnel) = agent_cluster(&pool).await;
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = api::server::AppState {
            state,
            prometheus_registry: api::metrics::PrometheusRegistry(None),
            http_metrics: api::metrics::HttpMetrics::new(&opentelemetry::global::meter("test")),
            cors_allowed_origins: common::config::CorsOrigins::SameOriginOnly,
            tls: None,
            shutdown_timeout_secs: 1,
        };
        drop(api::server::serve(listener, app).await);
    });

    let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
    client
        .write_all(
            format!(
                "GET /clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods/web/exec?command=sh HTTP/1.1\r\n\
                 Host: proxy\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.read_exact(&mut byte),
        )
        .await
        .expect("the 101 should arrive")
        .unwrap();
        head.push(byte[0]);
    }
    assert!(
        head.starts_with(b"HTTP/1.1 101"),
        "{}",
        String::from_utf8_lossy(&head)
    );
    client.write_all(b"ping").await.unwrap();
    let mut echoed = [0u8; 4];
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.read_exact(&mut echoed),
    )
    .await
    .expect("the exec bytes should come back")
    .unwrap();
    assert_eq!(&echoed, b"ping");

    server.abort();
    delete_proxy(&pool, &ns, &cluster).await;
}
