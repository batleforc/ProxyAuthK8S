//! RFC 0012: clusters reached through an agent tunnel.
//!
//! An `Agent` cluster has no address the proxy may dial: without a tunnel it
//! is refused with `503 agent unavailable`, never reached directly.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a panic is a failed test, with its message"
)]

#[cfg(feature = "envtest")]
mod envtest_support;
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

/// The `ServiceAccountToken` of `bootstrap`, the one `agent_service` sets.
fn service_account_token(
    bootstrap: &mut AgentBootstrap,
) -> Option<&mut ServiceAccountTokenBootstrap> {
    match bootstrap {
        AgentBootstrap::ServiceAccountToken(sa) => Some(sa),
        AgentBootstrap::OneShotToken(_) | AgentBootstrap::ExternalCA(_) => None,
    }
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
        limits: None,
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
                        if req.uri().path().ends_with("/selfsubjectreviews") {
                            // `validate_against: Kubernetes`: one known token.
                            let (status, body) = if auth == "Bearer good-token" {
                                (
                                    201,
                                    r#"{"apiVersion":"authentication.k8s.io/v1","kind":"SelfSubjectReview","status":{"userInfo":{"username":"alice","groups":["dev"]}}}"#,
                                )
                            } else {
                                (401, r#"{"kind":"Status","code":401}"#)
                            };
                            let response = hyper::Response::builder()
                                .status(status)
                                .header("content-type", "application/json")
                                .body(Full::new(bytes::Bytes::from(body)))
                                .unwrap();
                            return Ok(response);
                        }
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
    harness::install_crypto_provider();
    let (apiserver, ca_pem) = fake_apiserver().await;
    agent_cluster_to(pool, apiserver, &ca_pem).await
}

/// [`agent_cluster`], its agent relaying to `apiserver`, whose CA is `ca_pem`.
async fn agent_cluster_to(
    pool: &deadpool_redis::Pool,
    apiserver: std::net::SocketAddr,
    ca_pem: &str,
) -> (
    common::State,
    String,
    String,
    api::agent::registry::TunnelGuard,
) {
    use base64::Engine as _;
    use common::agent_gateway::{GatewayClient, presence_key};

    harness::install_crypto_provider();

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
    proxy.spec.cert = CertSource::Cert(base64::prelude::BASE64_STANDARD.encode(ca_pem));
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

/// A gateway's agent listener on a loopback port, under a new agent CA.
async fn agent_listener() -> (
    std::net::SocketAddr,
    std::sync::Arc<api::agent::registry::TunnelRegistry>,
    std::sync::Arc<agent_pki::AgentCa>,
) {
    let (addr, gateway) = agent_gateway().await;
    (
        addr,
        std::sync::Arc::clone(&gateway.registry),
        std::sync::Arc::clone(&gateway.ca),
    )
}

/// [`agent_listener`], the gateway itself returned.
async fn agent_gateway() -> (
    std::net::SocketAddr,
    std::sync::Arc<api::agent::listener::Gateway>,
) {
    agent_gateway_with(|_| ()).await
}

/// [`agent_gateway`], its settings changed by `tweak`; the peer address is
/// recorded, as `api::agent::run` does.
async fn agent_gateway_with(
    tweak: impl FnOnce(&mut common::config::AgentGatewaySettings),
) -> (
    std::net::SocketAddr,
    std::sync::Arc<api::agent::listener::Gateway>,
) {
    let state = test_state("https://idp.example.com".to_string());
    let ca = std::sync::Arc::new(agent_pki::AgentCa::generate().unwrap());
    let registry = api::agent::registry::TunnelRegistry::new();
    let mut settings = common::config::Config::from_lookup(|_| None).agent_gateway;
    settings.handshake_timeout = std::time::Duration::from_secs(2);
    // Every test enrols from the same address, in one shared Redis.
    settings.enroll_per_minute = 10_000;
    settings.enroll_per_minute_total = 10_000;
    tweak(&mut settings);
    let gateway =
        api::agent::listener::Gateway::new(state, registry, ca, settings, "gw-0".to_string());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(api::agent::listener::serve_agents(
        listener,
        std::sync::Arc::clone(&gateway),
    ));
    (addr, gateway)
}

/// An agent's side of the tunnel: WebSocket, mutual TLS with `cert_pem` and
/// `key`, yamux, then the control stream: `AgentInfo` sent, `Hello` read.
async fn dial_agent(
    addr: std::net::SocketAddr,
    trusted_ca_pem: &str,
    cert_pem: &str,
    key: &rcgen::KeyPair,
) -> Result<
    (
        agent_proto::mux::Stream,
        tokio::sync::mpsc::Receiver<agent_proto::mux::Stream>,
        agent_proto::control::Hello,
    ),
    String,
> {
    let info = agent_proto::control::AgentInfo {
        version: "0.4.0".to_string(),
        ca_fingerprint: String::new(),
        protocol_min: agent_proto::PROTOCOL_MIN,
        protocol_max: agent_proto::PROTOCOL_VERSION,
    };
    dial_agent_announcing(addr, trusted_ca_pem, cert_pem, key, info).await
}

/// [`dial_agent`], announcing `info`.
async fn dial_agent_announcing(
    addr: std::net::SocketAddr,
    trusted_ca_pem: &str,
    cert_pem: &str,
    key: &rcgen::KeyPair,
    info: agent_proto::control::AgentInfo,
) -> Result<
    (
        agent_proto::mux::Stream,
        tokio::sync::mpsc::Receiver<agent_proto::mux::Stream>,
        agent_proto::control::Hello,
    ),
    String,
> {
    use rustls::pki_types::pem::PemObject as _;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
    use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (ws, _) = tokio_tungstenite::client_async(format!("ws://{addr}/agent/v1/connect"), tcp)
        .await
        .map_err(|err| err.to_string())?;
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(trusted_ca_pem.as_bytes()).unwrap())
        .unwrap();
    let tls = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(
            vec![CertificateDer::from_pem_slice(cert_pem.as_bytes()).unwrap()],
            PrivateKeyDer::from_pem_slice(key.serialize_pem().as_bytes()).unwrap(),
        )
        .unwrap();
    let tls = tokio_rustls::TlsConnector::from(std::sync::Arc::new(tls))
        .connect(
            ServerName::try_from(agent_pki::TUNNEL_SERVER_NAME).unwrap(),
            agent_proto::ws_io::WsIo::new(ws).compat(),
        )
        .await
        .map_err(|err| err.to_string())?;
    let (handle, inbound, driver) =
        agent_proto::mux::new(tls.compat(), agent_proto::mux::Mode::Server, 8);
    tokio::spawn(driver);
    let mut control = handle.open().await.map_err(|err| err.to_string())?;
    agent_proto::control::write(&mut control, agent_proto::control::Kind::AgentInfo(info))
        .await
        .map_err(|err| err.to_string())?;
    let read = agent_proto::control::read(&mut control);
    let hello = match tokio::time::timeout(std::time::Duration::from_secs(5), read).await {
        Ok(Ok(agent_proto::control::Kind::Hello(hello))) => hello,
        other => return Err(format!("{other:?}")),
    };
    Ok((control, inbound, hello))
}

/// An agent certificate for `identity`, its key generated here.
fn agent_cert(
    ca: &agent_pki::AgentCa,
    identity: &agent_pki::AgentIdentity,
) -> (String, rcgen::KeyPair) {
    let key = rcgen::KeyPair::generate().unwrap();
    let csr = rcgen::CertificateParams::default()
        .serialize_request(&key)
        .unwrap()
        .pem()
        .unwrap();
    (ca.issue_agent(&csr, identity).unwrap(), key)
}

/// An `Agent` cluster seeded in Redis at `epoch`, and the identity naming it.
async fn seeded_agent_cluster(pool: &deadpool_redis::Pool, epoch: u64) -> agent_pki::AgentIdentity {
    seeded_cluster_with_jwks(pool, epoch, None).await
}

/// [`seeded_agent_cluster`], its JWKS inline when `jwks` is given.
async fn seeded_cluster_with_jwks(
    pool: &deadpool_redis::Pool,
    epoch: u64,
    jwks: Option<String>,
) -> agent_pki::AgentIdentity {
    let (ns, cluster) = unique_cluster();
    let uid = format!(
        "{:08x}-0000-4000-8000-{:012x}",
        std::process::id(),
        epoch.saturating_add(1)
    );
    let mut proxy = proxy_fixture(&ns, &cluster, "https://unused.example.com");
    proxy.spec.service = agent_service();
    if let Service::Agent(agent) = &mut proxy.spec.service {
        agent.epoch = epoch;
        if jwks.is_some() {
            let bootstrap = service_account_token(&mut agent.bootstrap).unwrap();
            bootstrap.jwks = jwks;
            bootstrap.jwks_url = None;
        }
    }
    proxy.metadata.uid = Some(uid.clone());
    seed_proxy(pool, &proxy).await;
    agent_pki::AgentIdentity {
        namespace: ns,
        name: cluster,
        uid,
        epoch,
    }
}

/// §5.3 `tlsMode: inner`: an enrolled agent's tunnel is registered, carries
/// the streams the gateway opens, and answers pings and renewals.
#[tokio::test]
async fn an_agent_with_a_valid_certificate_opens_a_tunnel() {
    use agent_proto::control::{self, Kind, Ping, RenewRequest};
    use rustls::pki_types::pem::PemObject as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_util::compat::FuturesAsyncReadCompatExt;

    let pool = redis_or_skip!();
    let (addr, registry, ca) = agent_listener().await;
    let identity = seeded_agent_cluster(&pool, 0).await;
    let (cert, key) = agent_cert(&ca, &identity);
    let (mut control, mut inbound, hello) =
        dial_agent(addr, ca.cert_pem(), &cert, &key).await.unwrap();
    assert_eq!(hello.replica_id, "gw-0");
    assert_eq!(hello.protocol, agent_proto::PROTOCOL_VERSION);

    // The tunnel is registered once the gateway has read `AgentInfo`.
    let stream = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(stream) = registry.open_stream(&identity.uid).await {
                return stream;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the tunnel should register");
    let mut gateway_side = stream.compat();
    // yamux announces the stream with its first bytes.
    gateway_side.write_all(b"hello").await.unwrap();
    let mut agent_side = inbound.recv().await.unwrap().compat();
    let mut seen = [0u8; 5];
    agent_side.read_exact(&mut seen).await.unwrap();
    assert_eq!(&seen, b"hello");

    control::write(&mut control, Kind::Ping(Ping { nonce: 9 }))
        .await
        .unwrap();
    assert_eq!(
        control::read(&mut control).await.unwrap(),
        Kind::Pong(agent_proto::control::Pong { nonce: 9 })
    );
    let csr = rcgen::CertificateParams::default()
        .serialize_request(&key)
        .unwrap()
        .pem()
        .unwrap();
    control::write(
        &mut control,
        Kind::RenewRequest(RenewRequest {
            csr: csr.into_bytes(),
        }),
    )
    .await
    .unwrap();
    let Kind::RenewResponse(renewed) = control::read(&mut control).await.unwrap() else {
        panic!("a renewal should answer RenewResponse");
    };
    let renewed = rustls::pki_types::CertificateDer::from_pem_slice(&renewed.cert).unwrap();
    assert_eq!(agent_pki::identity_of(&renewed).unwrap().identity, identity);

    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// RFC 0013 §6.3: an agent from before the protocol range (`0-0`) gets
/// protocol 1; a range without protocol 1 is refused before `register`, with
/// `Close "protocol"` naming the gateway's range, and a refusal record under the certificate's UID only.
#[tokio::test]
async fn a_protocol_range_without_overlap_is_refused_before_register() {
    let pool = redis_or_skip!();
    let (addr, gateway) = agent_gateway().await;
    // Epochs no other test seeds: the UID follows the epoch, and the refusal
    // key lives in the shared Redis.
    let identity = seeded_agent_cluster(&pool, 13).await;
    let other = seeded_agent_cluster(&pool, 14).await;
    let (cert, key) = agent_cert(&gateway.ca, &identity);
    let announcing = |protocol_min, protocol_max| agent_proto::control::AgentInfo {
        version: "0.7.0".to_string(),
        ca_fingerprint: String::new(),
        protocol_min,
        protocol_max,
    };

    let refused = dial_agent_announcing(addr, gateway.ca.cert_pem(), &cert, &key, announcing(2, 3))
        .await
        .unwrap_err();
    assert!(refused.contains("protocol"), "{refused}");
    // The gateway's own range, for the agent's log (RFC 0013 §17).
    assert!(
        refused.contains("protocol_min: 1, protocol_max: 1"),
        "{refused}"
    );
    assert!(gateway.registry.identities().is_empty());
    let record: common::agent_gateway::AgentRefusal = serde_json::from_str(
        &gateway
            .state
            .redis_get(&common::agent_gateway::agent_refused_key(&identity.uid))
            .await
            .unwrap()
            .expect("the refusal should be recorded"),
    )
    .unwrap();
    assert_eq!(
        record,
        common::agent_gateway::AgentRefusal {
            reason: "protocol".to_string(),
            version: "0.7.0".to_string(),
            protocols: "2-3".to_string(),
        }
    );
    assert_eq!(
        gateway
            .state
            .redis_get(&common::agent_gateway::agent_refused_key(&other.uid))
            .await
            .unwrap(),
        None
    );

    let (_control, _inbound, hello) =
        dial_agent_announcing(addr, gateway.ca.cert_pem(), &cert, &key, announcing(0, 0))
            .await
            .unwrap();
    assert_eq!(hello.protocol, 1);
    registered(&gateway, 1).await;

    delete_proxy(&pool, &identity.namespace, &identity.name).await;
    delete_proxy(&pool, &other.namespace, &other.name).await;
}

/// RFC 0013, T1: what an agent announces reaches the presence record in a
/// bounded shape only; the tunnel is served all the same.
#[tokio::test]
async fn an_invalid_announce_is_served_but_never_recorded_as_sent() {
    let pool = redis_or_skip!();
    let (addr, gateway) = agent_gateway().await;
    let identity = seeded_agent_cluster(&pool, 0).await;
    let (cert, key) = agent_cert(&gateway.ca, &identity);
    let info = agent_proto::control::AgentInfo {
        version: "0.5.0\n{\"forged\":1}".to_string(),
        ca_fingerprint: "x".repeat(4096),
        protocol_min: 1,
        protocol_max: 1,
    };
    let (_control, _inbound, _hello) =
        dial_agent_announcing(addr, gateway.ca.cert_pem(), &cert, &key, info)
            .await
            .unwrap();
    registered(&gateway, 1).await;
    let [(_, info)] = gateway.registry.identities().try_into().unwrap();
    assert_eq!(info.versions, vec!["invalid"]);
    assert_eq!(info.ca_fingerprints, vec![String::new()]);

    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// T8 and §5.3: a certificate for a past `epoch`, or from another CA, opens
/// no tunnel.
#[tokio::test]
async fn a_revoked_or_foreign_certificate_opens_no_tunnel() {
    let pool = redis_or_skip!();
    let (addr, registry, ca) = agent_listener().await;
    let identity = seeded_agent_cluster(&pool, 1).await;

    let stale = agent_pki::AgentIdentity {
        epoch: 0,
        ..identity.clone()
    };
    let (cert, key) = agent_cert(&ca, &stale);
    assert!(dial_agent(addr, ca.cert_pem(), &cert, &key).await.is_err());

    let foreign = agent_pki::AgentCa::generate().unwrap();
    let (cert, key) = agent_cert(&foreign, &identity);
    assert!(dial_agent(addr, ca.cert_pem(), &cert, &key).await.is_err());

    assert!(registry.identities().is_empty());
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// The remote cluster's ServiceAccount issuer: an ES256 key, `kid` `k1`.
struct Issuer {
    key: jsonwebtoken::EncodingKey,
    jwks: String,
}

impl Issuer {
    fn new() -> Self {
        let pem = rcgen::KeyPair::generate().unwrap().serialize_pem();
        let key = jsonwebtoken::EncodingKey::from_ec_pem(pem.as_bytes()).unwrap();
        let mut jwk =
            jsonwebtoken::jwk::Jwk::from_encoding_key(&key, jsonwebtoken::Algorithm::ES256)
                .unwrap();
        jwk.common.key_id = Some("k1".to_string());
        let jwks = serde_json::to_string(&jsonwebtoken::jwk::JwkSet { keys: vec![jwk] }).unwrap();
        Self { key, jwks }
    }

    fn token(&self, sub: &str, aud: &str, exp_in: i64) -> String {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some("k1".to_string());
        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap();
        let claims =
            serde_json::json!({ "sub": sub, "aud": aud, "exp": now.saturating_add(exp_in) });
        jsonwebtoken::encode(&header, &claims, &self.key).unwrap()
    }
}

async fn enroll(
    addr: std::net::SocketAddr,
    cluster: &str,
    csr: &str,
    token: &str,
) -> (u16, serde_json::Value) {
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/agent/v1/enroll"))
        .json(&serde_json::json!({
            "cluster": cluster,
            "csr": csr,
            "proof": { "service_account_token": token },
        }))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or_default())
}

const SUBJECT: &str = "system:serviceaccount:proxyauth-agent:proxyauth-agent";

/// §5.2: a valid projected token gets a certificate for the CR's identity,
/// and the CA that anchors the tunnel TLS.
#[tokio::test]
async fn a_valid_service_account_token_enrols_the_agent() {
    use rustls::pki_types::pem::PemObject as _;

    let pool = redis_or_skip!();
    let (addr, _registry, ca) = agent_listener().await;
    let issuer = Issuer::new();
    let identity = seeded_cluster_with_jwks(&pool, 3, Some(issuer.jwks.clone())).await;
    let csr = rcgen::CertificateParams::default()
        .serialize_request(&rcgen::KeyPair::generate().unwrap())
        .unwrap()
        .pem()
        .unwrap();

    let token = issuer.token(SUBJECT, DEFAULT_AUDIENCE, 600);
    let (status, body) = enroll(addr, &identity.cluster(), &csr, &token).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ca"], ca.cert_pem());
    let cert = rustls::pki_types::CertificateDer::from_pem_slice(
        body["cert"].as_str().unwrap().as_bytes(),
    )
    .unwrap();
    assert_eq!(agent_pki::identity_of(&cert).unwrap().identity, identity);

    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// T7, T9, §5.2: a token for another subject or audience, expired, signed
/// by another key, or for a cluster that is not `Agent`, gets a bare 401.
#[tokio::test]
async fn a_token_for_another_subject_or_audience_is_refused() {
    let pool = redis_or_skip!();
    let (addr, _registry, _ca) = agent_listener().await;
    let issuer = Issuer::new();
    let identity = seeded_cluster_with_jwks(&pool, 0, Some(issuer.jwks.clone())).await;
    let csr = rcgen::CertificateParams::default()
        .serialize_request(&rcgen::KeyPair::generate().unwrap())
        .unwrap()
        .pem()
        .unwrap();
    let (other_ns, other) = unique_cluster();
    seed_proxy(
        &pool,
        &proxy_fixture(&other_ns, &other, "https://other.example.com"),
    )
    .await;

    for (cluster, token) in [
        (
            identity.cluster(),
            issuer.token("system:serviceaccount:x:y", DEFAULT_AUDIENCE, 600),
        ),
        (identity.cluster(), issuer.token(SUBJECT, "kubernetes", 600)),
        (
            identity.cluster(),
            issuer.token(SUBJECT, DEFAULT_AUDIENCE, -600),
        ),
        (
            identity.cluster(),
            Issuer::new().token(SUBJECT, DEFAULT_AUDIENCE, 600),
        ),
        (identity.cluster(), "not-a-jwt".to_string()),
        (
            format!("{other_ns}/{other}"),
            issuer.token(SUBJECT, DEFAULT_AUDIENCE, 600),
        ),
        (
            "default/missing".to_string(),
            issuer.token(SUBJECT, DEFAULT_AUDIENCE, 600),
        ),
    ] {
        let (status, body) = enroll(addr, &cluster, &csr, &token).await;
        assert_eq!(status, 401, "{cluster}: {body}");
        assert_eq!(body, serde_json::Value::Null);
    }

    delete_proxy(&pool, &identity.namespace, &identity.name).await;
    delete_proxy(&pool, &other_ns, &other).await;
}

/// T8: a raised `epoch` closes the tunnels of the old certificates at the
/// next presence round, and a deleted CR withdraws this pod's presence.
#[tokio::test]
async fn presences_follow_the_cr_and_revoke_its_tunnels() {
    use agent_proto::control::{self, Kind};
    use api::agent::presence;
    use common::agent_gateway::presence_key;

    let pool = redis_or_skip!();
    let (addr, gateway) = agent_gateway().await;
    let identity = seeded_cluster_with_jwks(&pool, 4, None).await;
    let key = presence_key(&identity.uid);
    let (cert, agent_key) = agent_cert(&gateway.ca, &identity);
    let (mut control, _inbound, _) = dial_agent(addr, gateway.ca.cert_pem(), &cert, &agent_key)
        .await
        .unwrap();
    // Registered once `Hello` is sent; give the gateway a moment.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while gateway.registry.identities().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();

    presence::refresh(&gateway).await;
    assert_eq!(
        gateway.state.presence_members(&key).await.unwrap(),
        vec!["gw-0"]
    );
    // The record `status.agent` reads: one tunnel, what its agent announced.
    let presences = gateway.state.agent_presences(&identity.uid).await.unwrap();
    let [(pod, info)] = presences.as_slice() else {
        panic!("one gateway pod: {presences:?}");
    };
    assert_eq!(pod, "gw-0");
    assert_eq!(info.tunnels, 1);
    assert_eq!(info.versions, vec!["0.4.0"]);

    // `epoch` 4 -> 5: the agent hears why, the tunnel leaves the registry.
    let mut proxy = proxy_fixture(
        &identity.namespace,
        &identity.name,
        "https://unused.example.com",
    );
    proxy.spec.service = agent_service();
    if let Service::Agent(agent) = &mut proxy.spec.service {
        agent.epoch = 5;
    }
    proxy.metadata.uid = Some(identity.uid.clone());
    seed_proxy(&pool, &proxy).await;
    presence::refresh(&gateway).await;
    let closed = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        control::read(&mut control),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        closed,
        Kind::Close(control::Close {
            reason: "revoked".to_string(),
            protocol_min: agent_proto::PROTOCOL_MIN,
            protocol_max: agent_proto::PROTOCOL_VERSION,
        })
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !gateway.registry.identities().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();

    // A tunnel at the new epoch, then the CR deleted: presence withdrawn.
    let renewed = agent_pki::AgentIdentity {
        epoch: 5,
        ..identity.clone()
    };
    let (cert, agent_key) = agent_cert(&gateway.ca, &renewed);
    let (_control, _inbound, _) = dial_agent(addr, gateway.ca.cert_pem(), &cert, &agent_key)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while gateway.registry.identities().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    presence::refresh(&gateway).await;
    assert_eq!(
        gateway.state.presence_members(&key).await.unwrap(),
        vec!["gw-0"]
    );
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
    presence::refresh(&gateway).await;
    assert!(
        gateway
            .state
            .presence_members(&key)
            .await
            .unwrap()
            .is_empty()
    );
}

/// T18 at start-up: without `AGENT_GATEWAY_TOKEN` the gateway refuses to run.
#[tokio::test]
async fn the_gateway_role_needs_its_connect_token() {
    let settings = common::config::Config::from_lookup(|_| None).agent_gateway;
    let ended = api::agent::run(
        test_state("https://idp.example.com".to_string()),
        settings,
        "gw-0".to_string(),
        std::future::pending(),
    )
    .await;
    // No `{ended:?}`: an error of this module never reaches a log line here.
    assert!(matches!(ended, Err(api::agent::GatewayError::NoToken)));
}

/// The gateway's probes: `/healthz` always, `/readyz` while Redis answers.
#[tokio::test]
async fn the_gateway_answers_its_probes() {
    let _pool = redis_or_skip!();
    let (addr, _gateway) = agent_gateway().await;
    for probe in ["healthz", "readyz"] {
        let status = reqwest::get(format!("http://{addr}/{probe}"))
            .await
            .unwrap()
            .status();
        assert_eq!(status, 200, "{probe}");
    }
}

/// Step 8 of RFC 0012 §17: `validate_against: Kubernetes` reaches an `Agent`
/// cluster's apiserver through the gateway; a refused token is the caller's
/// fault (fail2login), a missing tunnel is not.
#[tokio::test]
async fn a_self_subject_review_crosses_the_gateway_and_the_tunnel() {
    use api::model::user::User;

    let pool = redis_or_skip!();
    let (state, ns, cluster, _tunnel) = agent_cluster(&pool).await;
    let proxy: crd::ProxyKubeApi = state
        .get_object_from_redis(crd::REDIS_PREFIX, &format!("{ns}/{cluster}"))
        .await
        .unwrap()
        .unwrap();

    let user =
        User::auth_against_kubernetes(state.clone(), proxy.clone(), "good-token".to_string())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(user.username, "alice");
    assert_eq!(user.groups, vec!["dev"]);

    let refused = User::auth_against_kubernetes(state.clone(), proxy.clone(), "bad".to_string())
        .await
        .unwrap_err();
    assert!(refused.is_caller_fault(), "{refused:?}");

    let mut orphan = proxy;
    orphan.metadata.uid = Some("ffffffff-0000-4000-8000-000000000000".to_string());
    let unreachable = User::auth_against_kubernetes(state, orphan, "good-token".to_string())
        .await
        .unwrap_err();
    assert!(!unreachable.is_caller_fault(), "{unreachable:?}");

    delete_proxy(&pool, &ns, &cluster).await;
}

/// T14: every enrolment and every tunnel leaves an audit line, refusals with
/// their reason, and a relayed request says it went through the agent.
#[tokio::test]
async fn enrollment_and_tunnels_write_audit_events() {
    use api::cluster::redirect::audit::capture;

    let pool = redis_or_skip!();
    capture::start();
    let (addr, gateway) = agent_gateway().await;
    let issuer = Issuer::new();
    let identity = seeded_cluster_with_jwks(&pool, 0, Some(issuer.jwks.clone())).await;
    let key = rcgen::KeyPair::generate().unwrap();
    let csr = rcgen::CertificateParams::default()
        .serialize_request(&key)
        .unwrap()
        .pem()
        .unwrap();
    let token = issuer.token(SUBJECT, DEFAULT_AUDIENCE, 600);
    let (status, _) = enroll(addr, &identity.cluster(), &csr, &token).await;
    assert_eq!(status, 200);
    let bad = issuer.token("system:serviceaccount:x:y", DEFAULT_AUDIENCE, 600);
    let (status, _) = enroll(addr, &identity.cluster(), &csr, &bad).await;
    assert_eq!(status, 401);

    let (cert, agent_key) = agent_cert(&gateway.ca, &identity);
    let (_control, _inbound, _) = dial_agent(addr, gateway.ca.cert_pem(), &cert, &agent_key)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while gateway.registry.identities().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
    api::agent::presence::refresh(&gateway).await;

    let lines = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let lines = capture::lines();
            if lines
                .iter()
                .any(|line| line["event"] == "agent_disconnected")
            {
                return lines;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the tunnel should close with an audit line");
    let of = |event: &str| -> Vec<&serde_json::Value> {
        lines.iter().filter(|line| line["event"] == event).collect()
    };
    let enrolled = of("agent_enrolled");
    assert_eq!(enrolled.len(), 1, "{lines:?}");
    assert_eq!(enrolled[0]["cluster"], identity.name.as_str());
    assert_eq!(enrolled[0]["agent_epoch"], 0);
    let refused = of("agent_refused");
    assert_eq!(refused.len(), 1, "{lines:?}");
    assert_eq!(refused[0]["reason"], "subject");
    assert_eq!(refused[0]["outcome"], "denied");
    let connected = of("agent_connected");
    assert_eq!(connected.len(), 1, "{lines:?}");
    assert_eq!(connected[0]["agent_version"], "0.4.0");
    assert_eq!(of("agent_disconnected")[0]["reason"], "revoked");
    // No line carries the proof.
    assert!(lines.iter().all(|line| !line.to_string().contains(&token)));
}

/// T14: a request relayed through the gateway says so in its audit line.
#[tokio::test]
async fn a_relayed_request_is_audited_as_upstream_via_agent() {
    use api::cluster::redirect::audit::capture;

    let pool = redis_or_skip!();
    capture::start();
    let (state, ns, cluster, _tunnel) = agent_cluster(&pool).await;
    let app = test::init_service(harness::app(state)).await;
    let request = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", "Bearer user-token"))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::OK
    );
    let lines = capture::lines();
    let request = lines
        .iter()
        .find(|line| line["event"] == "request" && line["cluster"] == cluster.as_str())
        .unwrap_or_else(|| panic!("a request line: {lines:?}"));
    assert_eq!(request["upstream_via"], "agent");

    delete_proxy(&pool, &ns, &cluster).await;
}

/// §6.P point 1 with the real agent: it enrols with its projected token,
/// opens its tunnel, and a request relayed by the proxy reaches the
/// apiserver through the gateway's `CONNECT`, the agent and its upstream.
#[tokio::test]
async fn the_real_agent_enrols_and_carries_a_relayed_request() {
    use base64::Engine as _;
    use common::agent_gateway::GatewayClient;
    use std::sync::atomic::Ordering;

    let pool = redis_or_skip!();
    harness::install_crypto_provider();
    let (apiserver, ca_pem) = fake_apiserver().await;
    let (addr, gateway) = agent_gateway().await;
    let issuer = Issuer::new();
    let identity = seeded_cluster_with_jwks(&pool, 0, Some(issuer.jwks.clone())).await;
    let mut proxy: crd::ProxyKubeApi = gateway
        .state
        .get_object_from_redis(crd::REDIS_PREFIX, &identity.cluster())
        .await
        .unwrap()
        .unwrap();
    proxy.spec.cert = CertSource::Cert(base64::prelude::BASE64_STANDARD.encode(&ca_pem));
    seed_proxy(&pool, &proxy).await;

    let connect = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let connect_addr = connect.local_addr().unwrap();
    tokio::spawn(api::agent::connect::serve(
        connect,
        api::agent::connect::ConnectService::new(
            std::sync::Arc::clone(&gateway.registry),
            Some("t0ken".to_string()),
        ),
    ));

    let token_path = std::env::temp_dir().join(format!("agent-token-{}", identity.uid));
    std::fs::write(&token_path, issuer.token(SUBJECT, DEFAULT_AUDIENCE, 600)).unwrap();
    let vars = std::collections::HashMap::from([
        ("AGENT_PROXY_URL", format!("ws://{addr}/agent/v1/connect")),
        ("AGENT_CLUSTER", identity.cluster()),
        ("AGENT_UPSTREAM", format!("https://{apiserver}")),
        ("AGENT_TOKEN_PATH", token_path.display().to_string()),
    ]);
    let config = agent::Config::from_lookup(|name| vars.get(name).cloned()).unwrap();
    let health = std::sync::Arc::new(agent::Health::default());
    tokio::spawn(agent::run(config, std::sync::Arc::clone(&health)));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !health.tunnel_up.load(Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the agent should open its tunnel");
    assert!(health.cert_not_after.load(Ordering::Relaxed) > 0);
    api::agent::presence::refresh(&gateway).await;

    let mut state = test_state("https://idp.example.com".to_string());
    let client = GatewayClient::with(Some("gw.test".to_string()), 0, Some("t0ken".to_string()));
    *client.fixed_address.lock().unwrap() = Some(connect_addr.to_string());
    state.agent_gateway = std::sync::Arc::new(client);
    let app = test::init_service(harness::app(state)).await;
    let request = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{}/{}/api/v1/pods",
            identity.namespace, identity.name
        ))
        .insert_header(("authorization", "Bearer user-token"))
        .to_request();
    let resp = test::call_service(&app, request).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        String::from_utf8_lossy(&test::read_body(resp).await),
        "apiserver saw /api/v1/pods auth=Bearer user-token"
    );

    drop(std::fs::remove_file(&token_path));
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// A TLS client of `addr` trusting `ca_pem`, presenting `client` if given.
async fn tls_to(
    addr: std::net::SocketAddr,
    ca_pem: &str,
    client: Option<(&str, &rcgen::KeyPair)>,
) -> tokio_rustls::client::TlsStream<tokio::net::TcpStream> {
    use rustls::pki_types::pem::PemObject as _;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};

    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(ca_pem.as_bytes()).unwrap())
        .unwrap();
    let builder = rustls::ClientConfig::builder().with_root_certificates(roots);
    let config = match client {
        Some((cert, key)) => builder
            .with_client_auth_cert(
                vec![CertificateDer::from_pem_slice(cert.as_bytes()).unwrap()],
                PrivateKeyDer::from_pem_slice(key.serialize_pem().as_bytes()).unwrap(),
            )
            .unwrap(),
        None => builder.with_no_client_auth(),
    };
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await
        .unwrap()
}

/// §6.P point 11, `tlsMode: passthrough`: the gateway ends the TLS itself;
/// the real agent enrols without a certificate and opens its tunnel with
/// one; a WebSocket without one, or with one from another CA, opens nothing.
#[tokio::test]
async fn the_real_agent_opens_a_passthrough_tunnel() {
    use std::sync::atomic::Ordering;

    let pool = redis_or_skip!();
    harness::install_crypto_provider();
    let dir = std::env::temp_dir().join(format!("agent-passthrough-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let server = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let (cert_path, key_path) = (dir.join("tls.crt"), dir.join("tls.key"));
    std::fs::write(&cert_path, server.cert.pem()).unwrap();
    std::fs::write(&key_path, server.signing_key.serialize_pem()).unwrap();
    let (addr, gateway) = agent_gateway_with(|settings| {
        settings.tls_mode = common::config::AgentTlsMode::Passthrough {
            cert_path: cert_path.display().to_string(),
            key_path: key_path.display().to_string(),
        };
    })
    .await;
    let issuer = Issuer::new();
    let identity = seeded_cluster_with_jwks(&pool, 0, Some(issuer.jwks.clone())).await;
    let url = format!("wss://localhost:{}/agent/v1/connect", addr.port());

    let refused =
        tokio_tungstenite::client_async(&url, tls_to(addr, &server.cert.pem(), None).await)
            .await
            .map(|_| ())
            .unwrap_err();
    assert!(
        matches!(&refused, tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == 401),
        "{refused:?}"
    );
    let foreign = agent_pki::AgentCa::generate().unwrap();
    let (cert, key) = agent_cert(&foreign, &identity);
    let tls = tls_to(addr, &server.cert.pem(), Some((&cert, &key))).await;
    assert!(tokio_tungstenite::client_async(&url, tls).await.is_err());

    let token_path = dir.join("token");
    std::fs::write(&token_path, issuer.token(SUBJECT, DEFAULT_AUDIENCE, 600)).unwrap();
    let vars = std::collections::HashMap::from([
        ("AGENT_PROXY_URL", url.clone()),
        ("AGENT_PROXY_CA_BUNDLE", cert_path.display().to_string()),
        ("AGENT_TLS_MODE", "passthrough".to_string()),
        ("AGENT_CLUSTER", identity.cluster()),
        ("AGENT_TOKEN_PATH", token_path.display().to_string()),
    ]);
    let config = agent::Config::from_lookup(|name| vars.get(name).cloned()).unwrap();
    let health = std::sync::Arc::new(agent::Health::default());
    tokio::spawn(agent::run(config, std::sync::Arc::clone(&health)));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !health.tunnel_up.load(Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the agent should open its passthrough tunnel");
    let tunnels = gateway.registry.identities();
    assert_eq!(tunnels.len(), 1);
    assert_eq!(tunnels[0].0, identity);

    drop(std::fs::remove_dir_all(&dir));
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

// ---- §14, one test per threat (T6 and T16 come with phase 2; T14 above;
// T18 is `connect::tests::a_connect_without_the_proxy_token_is_refused`).

/// A CSR for a new key.
fn new_csr() -> String {
    rcgen::CertificateParams::default()
        .serialize_request(&rcgen::KeyPair::generate().unwrap())
        .unwrap()
        .pem()
        .unwrap()
}

/// Wait until the registry holds `count` identities.
async fn registered(gateway: &api::agent::listener::Gateway, count: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while gateway.registry.identities().len() != count {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

/// T1: a certificate the agent CA did not issue opens nothing, whatever
/// identity it claims.
#[tokio::test]
async fn a_cert_from_another_ca_is_refused() {
    let pool = redis_or_skip!();
    let (addr, gateway) = agent_gateway().await;
    let identity = seeded_agent_cluster(&pool, 0).await;
    let foreign = agent_pki::AgentCa::generate().unwrap();
    let (cert, key) = agent_cert(&foreign, &identity);
    assert!(
        dial_agent(addr, gateway.ca.cert_pem(), &cert, &key)
            .await
            .is_err()
    );
    // Trusting the foreign CA on the agent side changes nothing.
    assert!(
        dial_agent(addr, foreign.cert_pem(), &cert, &key)
            .await
            .is_err()
    );
    assert!(gateway.registry.identities().is_empty());
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// T2: tenant A's proof never enrols for B, A's certificate never serves B,
/// and an identity mixing B's name with A's UID is refused.
#[tokio::test]
async fn an_agent_cannot_serve_another_proxykubeapi() {
    let pool = redis_or_skip!();
    let (addr, gateway) = agent_gateway().await;
    let issuer = Issuer::new();
    let a = seeded_cluster_with_jwks(&pool, 0, Some(issuer.jwks.clone())).await;
    let b = seeded_cluster_with_jwks(&pool, 1, Some(issuer.jwks.clone())).await;
    // B's CR names another subject: A's token does not prove B.
    let mut proxy_b: crd::ProxyKubeApi = gateway
        .state
        .get_object_from_redis(crd::REDIS_PREFIX, &b.cluster())
        .await
        .unwrap()
        .unwrap();
    if let Service::Agent(agent) = &mut proxy_b.spec.service {
        let bootstrap = service_account_token(&mut agent.bootstrap).unwrap();
        bootstrap.subject = "system:serviceaccount:tenant-b:agent".to_string();
    }
    seed_proxy(&pool, &proxy_b).await;
    let token_a = issuer.token(SUBJECT, DEFAULT_AUDIENCE, 600);
    let (status, _) = enroll(addr, &b.cluster(), &new_csr(), &token_a).await;
    assert_eq!(status, 401);

    let (cert, key) = agent_cert(&gateway.ca, &a);
    let _tunnel = dial_agent(addr, gateway.ca.cert_pem(), &cert, &key)
        .await
        .unwrap();
    registered(&gateway, 1).await;
    assert!(gateway.registry.open_stream(&b.uid).await.is_err());

    let mixed = agent_pki::AgentIdentity {
        uid: a.uid.clone(),
        ..b.clone()
    };
    let (cert, key) = agent_cert(&gateway.ca, &mixed);
    assert!(
        dial_agent(addr, gateway.ca.cert_pem(), &cert, &key)
            .await
            .is_err()
    );

    delete_proxy(&pool, &a.namespace, &a.name).await;
    delete_proxy(&pool, &b.namespace, &b.name).await;
}

/// T3: what the agent relays is TLS: the user's token never crosses it in
/// clear.
#[tokio::test]
async fn the_agent_only_sees_tls_records() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let pool = redis_or_skip!();
    harness::install_crypto_provider();
    let (apiserver, ca_pem) = fake_apiserver().await;
    // A tap between the agent and the apiserver, keeping what goes up.
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let tap = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tap_addr = tap.local_addr().unwrap();
    let recorded = std::sync::Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((mut agent_side, _)) = tap.accept().await {
            let recorded = std::sync::Arc::clone(&recorded);
            tokio::spawn(async move {
                let mut upstream = tokio::net::TcpStream::connect(apiserver).await.unwrap();
                let (mut agent_read, mut agent_write) = agent_side.split();
                let (mut up_read, mut up_write) = upstream.split();
                let up = async {
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = agent_read.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        recorded.lock().unwrap().extend_from_slice(&buf[..n]);
                        if up_write.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                };
                let down = tokio::io::copy(&mut up_read, &mut agent_write);
                drop(tokio::join!(up, down));
            });
        }
    });
    let (state, ns, cluster, _tunnel) = agent_cluster_to(&pool, tap_addr, &ca_pem).await;
    let app = test::init_service(harness::app(state)).await;
    let request = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", "Bearer very-secret-user-token"))
        .to_request();
    assert_eq!(
        test::call_service(&app, request).await.status(),
        StatusCode::OK
    );
    let seen = seen.lock().unwrap().clone();
    assert!(!seen.is_empty(), "the tap saw the relayed bytes");
    assert!(
        !seen
            .windows(b"very-secret".len())
            .any(|window| window == b"very-secret"),
        "the token crossed the agent in clear"
    );
    assert!(
        !seen
            .windows(b"/api/v1/pods".len())
            .any(|window| window == b"/api/v1/pods")
    );
    delete_proxy(&pool, &ns, &cluster).await;
}

/// T4: an `Agent` cluster needs its apiserver's CA; `SystemRoots` would take
/// any public certificate the agent presents.
#[tokio::test]
async fn spec_cert_is_required_and_system_roots_refused() {
    let agent = match agent_service() {
        Service::Agent(agent) => Some(agent),
        _ => None,
    }
    .unwrap();
    for cert in [CertSource::SystemRoots(true), CertSource::Insecure(true)] {
        let err = agent.validate(&cert).unwrap_err();
        assert!(err.contains("SystemRoots"), "{err}");
    }
    let private = CertSource::ConfigMap {
        name: "prod-ca".to_string(),
        key: "ca.crt".to_string(),
        namespace: None,
    };
    agent.validate(&private).unwrap();
}

/// T5: the destination of a `CONNECT` goes nowhere: the stream the agent gets
/// starts with the proxy's own bytes, no address before them.
#[tokio::test]
async fn a_stream_open_carries_no_destination() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

    let (gateway_io, agent_io) = tokio::io::duplex(1 << 16);
    let (handle, _inbound, driver) =
        agent_proto::mux::new(gateway_io.compat(), agent_proto::mux::Mode::Client, 8);
    let (_agent, mut agent_in, agent_driver) =
        agent_proto::mux::new(agent_io.compat(), agent_proto::mux::Mode::Server, 8);
    tokio::spawn(driver);
    tokio::spawn(agent_driver);
    let registry = api::agent::registry::TunnelRegistry::new();
    let _guard = registry
        .register(
            agent_pki::AgentIdentity {
                namespace: "a".into(),
                name: "b".into(),
                uid: "cc".into(),
                epoch: 0,
            },
            handle,
            1,
            8,
        )
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(api::agent::connect::serve(
        listener,
        api::agent::connect::ConnectService::new(registry, Some("t0ken".to_string())),
    ));
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    tcp.write_all(
        b"CONNECT 10.0.0.1:22 HTTP/1.1\r\nHost: 10.0.0.1:22\r\n\
          Proxy-Authorization: Bearer t0ken\r\nX-Proxyauth-Agent: cc\r\n\r\n",
    )
    .await
    .unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        tcp.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 200"));
    tcp.write_all(b"client-hello").await.unwrap();
    let mut stream = agent_in.recv().await.unwrap().compat();
    let mut first = [0u8; 12];
    stream.read_exact(&mut first).await.unwrap();
    assert_eq!(&first, b"client-hello");
}

/// T8: once `epoch` is raised, a renewal is refused and the tunnel closed.
#[tokio::test]
async fn an_epoch_bump_closes_tunnels_and_refuses_renewal() {
    use agent_proto::control::{self, Close, Kind, RenewRequest};

    let pool = redis_or_skip!();
    let (addr, gateway) = agent_gateway().await;
    let identity = seeded_agent_cluster(&pool, 2).await;
    let (cert, key) = agent_cert(&gateway.ca, &identity);
    let (mut control, _inbound, _) = dial_agent(addr, gateway.ca.cert_pem(), &cert, &key)
        .await
        .unwrap();
    registered(&gateway, 1).await;
    let mut proxy: crd::ProxyKubeApi = gateway
        .state
        .get_object_from_redis(crd::REDIS_PREFIX, &identity.cluster())
        .await
        .unwrap()
        .unwrap();
    if let Service::Agent(agent) = &mut proxy.spec.service {
        agent.epoch = 3;
    }
    seed_proxy(&pool, &proxy).await;
    let csr = rcgen::CertificateParams::default()
        .serialize_request(&key)
        .unwrap()
        .pem()
        .unwrap();
    control::write(
        &mut control,
        Kind::RenewRequest(RenewRequest {
            csr: csr.into_bytes(),
        }),
    )
    .await
    .unwrap();
    let answer = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        control::read(&mut control),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        answer,
        Kind::Close(Close {
            reason: "epoch".to_string(),
            protocol_min: agent_proto::PROTOCOL_MIN,
            protocol_max: agent_proto::PROTOCOL_VERSION,
        })
    );
    registered(&gateway, 0).await;
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// T9: the enrolment answers a certificate and the CA, never a key.
#[tokio::test]
async fn enrollment_never_returns_the_private_key() {
    let pool = redis_or_skip!();
    let (addr, _gateway) = agent_gateway().await;
    let issuer = Issuer::new();
    let identity = seeded_cluster_with_jwks(&pool, 0, Some(issuer.jwks.clone())).await;
    let token = issuer.token(SUBJECT, DEFAULT_AUDIENCE, 600);
    let (status, body) = enroll(addr, &identity.cluster(), &new_csr(), &token).await;
    assert_eq!(status, 200);
    let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["ca", "cert"]);
    assert!(!body.to_string().contains("PRIVATE KEY"));
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// T6: a `OneShotToken` enrols once, whatever the race; a wrong token, the
/// other proof, or a refused CSR burns nothing.
#[tokio::test]
async fn a_one_shot_token_is_accepted_once() {
    let pool = redis_or_skip!();
    let (addr, gateway) = agent_gateway().await;
    let identity = seeded_agent_cluster(&pool, 0).await;
    let mut proxy: crd::ProxyKubeApi = gateway
        .state
        .get_object_from_redis(crd::REDIS_PREFIX, &identity.cluster())
        .await
        .unwrap()
        .unwrap();
    if let Service::Agent(agent) = &mut proxy.spec.service {
        agent.bootstrap =
            AgentBootstrap::OneShotToken(crd::service::OneShotTokenBootstrap::default());
    }
    seed_proxy(&pool, &proxy).await;
    let token = "one-shot-token-of-the-test";
    gateway
        .state
        .set_flag(
            &common::agent_gateway::bootstrap_key(&identity.uid, token),
            common::agent_gateway::BOOTSTRAP_TTL.as_secs(),
        )
        .await
        .unwrap();
    let one_shot = |csr: String, token: &str| {
        let body = serde_json::json!({
            "cluster": identity.cluster(),
            "csr": csr,
            "proof": { "one_shot_token": token },
        });
        async move {
            reqwest::Client::new()
                .post(format!("http://{addr}/agent/v1/enroll"))
                .json(&body)
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }
    };

    assert_eq!(one_shot(new_csr(), "not-the-token").await, 401);
    assert_eq!(one_shot("not a CSR".to_string(), token).await, 401);
    assert_eq!(
        enroll(addr, &identity.cluster(), &new_csr(), token).await.0,
        401
    );
    let race = futures_util::future::join_all((0..8).map(|_| one_shot(new_csr(), token))).await;
    assert_eq!(
        race.iter().filter(|status| **status == 200).count(),
        1,
        "{race:?}"
    );
    assert!(
        race.iter().all(|status| [200, 401].contains(status)),
        "{race:?}"
    );
    assert_eq!(one_shot(new_csr(), token).await, 401);
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// An apiserver holding the agent's identity Secret, refusing a write whose
/// `resourceVersion` is not the current one (decision 21).
async fn identity_secret_apiserver() -> String {
    use axum::extract::State;
    use axum::http::StatusCode;
    use std::sync::{Arc, Mutex};

    let stored = Arc::new(Mutex::new(serde_json::json!({
        "apiVersion": "v1", "kind": "Secret", "type": "Opaque",
        "metadata": { "name": "identity", "namespace": "agent", "resourceVersion": "1" },
    })));
    let app = axum::Router::new()
        .route(
            "/api/v1/namespaces/agent/secrets/identity",
            axum::routing::get(
                |State(stored): State<Arc<Mutex<serde_json::Value>>>| async move {
                    axum::Json(stored.lock().unwrap().clone())
                },
            )
            .put(
                |State(stored): State<Arc<Mutex<serde_json::Value>>>,
                 axum::Json(mut body): axum::Json<serde_json::Value>| async move {
                    const VERSION: &str = "/metadata/resourceVersion";
                    let mut current = stored.lock().unwrap();
                    let version = current.pointer(VERSION).cloned().unwrap();
                    if body.pointer(VERSION) != Some(&version) {
                        return StatusCode::CONFLICT;
                    }
                    let next = version
                        .as_str()
                        .unwrap()
                        .parse::<u64>()
                        .unwrap()
                        .saturating_add(1);
                    *body.pointer_mut(VERSION).unwrap() = serde_json::json!(next.to_string());
                    *current = body;
                    StatusCode::OK
                },
            ),
        )
        .with_state(stored);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

/// Decision 21, §6.P point 8: two agent replicas start on one `OneShotToken`
/// and one identity Secret; the token is spent once, both hold a tunnel.
#[tokio::test]
async fn one_shot_replicas_spend_one_token_and_share_the_identity() {
    use std::sync::atomic::Ordering;

    let pool = redis_or_skip!();
    harness::install_crypto_provider();
    let (addr, gateway) = agent_gateway().await;
    let identity = seeded_agent_cluster(&pool, 0).await;
    let mut proxy: crd::ProxyKubeApi = gateway
        .state
        .get_object_from_redis(crd::REDIS_PREFIX, &identity.cluster())
        .await
        .unwrap()
        .unwrap();
    if let Service::Agent(agent) = &mut proxy.spec.service {
        agent.bootstrap =
            AgentBootstrap::OneShotToken(crd::service::OneShotTokenBootstrap::default());
    }
    seed_proxy(&pool, &proxy).await;
    let token = "the-one-shot-token-of-the-replicas";
    gateway
        .state
        .set_flag(
            &common::agent_gateway::bootstrap_key(&identity.uid, token),
            3600,
        )
        .await
        .unwrap();
    let dir = std::env::temp_dir().join(format!("agent-one-shot-{}", identity.uid));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("token"), token).unwrap();
    std::fs::write(dir.join("api-token"), "api-token").unwrap();
    let api = identity_secret_apiserver().await;
    let vars = std::collections::HashMap::from([
        ("AGENT_PROXY_URL", format!("ws://{addr}/agent/v1/connect")),
        ("AGENT_CLUSTER", identity.cluster()),
        ("AGENT_TOKEN_PATH", dir.join("token").display().to_string()),
        ("AGENT_BOOTSTRAP_METHOD", "OneShotToken".to_string()),
        ("AGENT_IDENTITY_SECRET", "identity".to_string()),
        ("AGENT_NAMESPACE", "agent".to_string()),
        ("AGENT_API_URL", api),
        (
            "AGENT_API_TOKEN_PATH",
            dir.join("api-token").display().to_string(),
        ),
    ]);
    let config = agent::Config::from_lookup(|name| vars.get(name).cloned()).unwrap();
    let replicas: Vec<_> = (0..2)
        .map(|_| {
            let health = std::sync::Arc::new(agent::Health::default());
            tokio::spawn(agent::run(config.clone(), std::sync::Arc::clone(&health)));
            health
        })
        .collect();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while !replicas
            .iter()
            .all(|health| health.tunnel_up.load(Ordering::Relaxed))
        {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("both replicas should open a tunnel");
    let tunnels = gateway.registry.identities();
    assert_eq!(tunnels.len(), 1);
    assert_eq!(tunnels[0].1.tunnels, 2, "one identity, two tunnels");
    assert!(
        !gateway
            .state
            .key_exists(&common::agent_gateway::bootstrap_key(&identity.uid, token))
            .await
            .unwrap()
    );

    drop(std::fs::remove_dir_all(&dir));
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// An issuer of the operator's (cert-manager, Vault PKI) for `ExternalCA`.
struct ExternalIssuer {
    issuer: rcgen::Issuer<'static, rcgen::KeyPair>,
    pem: String,
}

impl ExternalIssuer {
    fn new() -> Self {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::default();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::DigitalSignature,
        ];
        let pem = params.self_signed(&key).unwrap().pem();
        Self {
            issuer: rcgen::Issuer::new(params, key),
            pem,
        }
    }

    /// A client certificate with the URI SAN `uri`, valid from a minute ago
    /// for `lifetime`.
    fn leaf(&self, uri: &str, lifetime: std::time::Duration) -> (String, rcgen::KeyPair) {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::default();
        params.subject_alt_names = vec![rcgen::SanType::URI(uri.try_into().unwrap())];
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now.checked_sub(time::Duration::minutes(1)).unwrap();
        params.not_after = now.checked_add(lifetime.try_into().unwrap()).unwrap();
        (params.signed_by(&key, &self.issuer).unwrap().pem(), key)
    }
}

/// An `Agent` cluster whose bootstrap is `ExternalCA` with `ca` inline.
async fn seeded_external_cluster(
    pool: &deadpool_redis::Pool,
    gateway: &api::agent::listener::Gateway,
    epoch: u64,
    ca: &str,
) -> agent_pki::AgentIdentity {
    let identity = seeded_agent_cluster(pool, epoch).await;
    let mut proxy: crd::ProxyKubeApi = gateway
        .state
        .get_object_from_redis(crd::REDIS_PREFIX, &identity.cluster())
        .await
        .unwrap()
        .unwrap();
    if let Service::Agent(agent) = &mut proxy.spec.service {
        agent.bootstrap = AgentBootstrap::ExternalCA(crd::service::ExternalCaBootstrap {
            ca: Some(ca.to_string()),
            ca_ref: None,
            max_cert_lifetime_hours: crd::service::DEFAULT_MAX_CERT_LIFETIME_HOURS,
        });
    }
    seed_proxy(pool, &proxy).await;
    identity
}

/// Decisions 28 and 29: a certificate of the CR's own issuer, naming the CR
/// by `proxyauth://<ns>/<name>`, opens a tunnel for that CR alone, at its
/// UID and `epoch`; another issuer, a longer lifetime, another bootstrap,
/// an enrolment or a renewal are refused.
#[tokio::test]
async fn an_external_ca_certificate_serves_its_cluster_only() {
    let pool = redis_or_skip!();
    harness::install_crypto_provider();
    let (addr, gateway) = agent_gateway().await;
    let issuer = ExternalIssuer::new();
    let day = std::time::Duration::from_secs(24 * 3600);
    let a = seeded_external_cluster(&pool, &gateway, 0, &issuer.pem).await;
    // Tenant B shares the issuer (one Vault PKI for both).
    let b = seeded_external_cluster(&pool, &gateway, 1, &issuer.pem).await;
    let uri_of =
        |id: &agent_pki::AgentIdentity| format!("proxyauth://{}/{}", id.namespace, id.name);
    let trusted = gateway.ca.cert_pem().to_string();

    let (cert, key) = issuer.leaf(&uri_of(&a), day);
    let (mut control, _inbound, _) = dial_agent(addr, &trusted, &cert, &key).await.unwrap();
    registered(&gateway, 1).await;
    assert_eq!(
        gateway.registry.identities()[0].0,
        a,
        "A's UID and epoch, never B's"
    );

    // Its issuer renews it, never the agent CA.
    agent_proto::control::write(
        &mut control,
        agent_proto::control::Kind::RenewRequest(agent_proto::control::RenewRequest {
            csr: new_csr().into_bytes(),
        }),
    )
    .await
    .unwrap();
    let answer = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        agent_proto::control::read(&mut control),
    )
    .await
    .unwrap();
    assert!(
        matches!(&answer, Ok(agent_proto::control::Kind::Close(close)) if close.reason == "method"),
        "{answer:?}"
    );
    registered(&gateway, 0).await;

    let foreign = ExternalIssuer::new();
    let (cert, key) = foreign.leaf(&uri_of(&a), day);
    assert!(
        dial_agent(addr, &trusted, &cert, &key).await.is_err(),
        "another issuer"
    );
    let (cert, key) = issuer.leaf(&uri_of(&a), 8 * day);
    assert!(
        dial_agent(addr, &trusted, &cert, &key).await.is_err(),
        "past 7 days"
    );
    let sa = seeded_agent_cluster(&pool, 2).await;
    let (cert, key) = issuer.leaf(&uri_of(&sa), day);
    assert!(
        dial_agent(addr, &trusted, &cert, &key).await.is_err(),
        "not ExternalCA"
    );
    assert_eq!(enroll(addr, &a.cluster(), &new_csr(), "any").await.0, 401);
    assert!(gateway.registry.identities().is_empty());

    for id in [a, b, sa] {
        delete_proxy(&pool, &id.namespace, &id.name).await;
    }
}

/// Decision 29: an external certificate's tunnel closes when it expires.
#[tokio::test]
async fn an_external_tunnel_closes_when_its_certificate_expires() {
    let pool = redis_or_skip!();
    harness::install_crypto_provider();
    let (addr, gateway) = agent_gateway().await;
    let issuer = ExternalIssuer::new();
    let id = seeded_external_cluster(&pool, &gateway, 3, &issuer.pem).await;
    let (cert, key) = issuer.leaf(
        &format!("proxyauth://{}/{}", id.namespace, id.name),
        std::time::Duration::from_secs(3),
    );
    let (_control, _inbound, _) = dial_agent(addr, gateway.ca.cert_pem(), &cert, &key)
        .await
        .unwrap();
    registered(&gateway, 1).await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !gateway.registry.identities().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the tunnel should close at the certificate's expiry");
    delete_proxy(&pool, &id.namespace, &id.name).await;
}

/// Decisions 28 and 29, the real agent: no enrolment, its certificate read
/// from files; at expiry the gateway closes the tunnel, and the agent comes
/// back with the certificate its issuer renewed in those files.
#[tokio::test]
async fn the_real_agent_uses_an_external_certificate_and_its_renewal() {
    use std::sync::atomic::Ordering;

    let pool = redis_or_skip!();
    harness::install_crypto_provider();
    let (addr, gateway) = agent_gateway().await;
    let issuer = ExternalIssuer::new();
    let id = seeded_external_cluster(&pool, &gateway, 4, &issuer.pem).await;
    let uri = format!("proxyauth://{}/{}", id.namespace, id.name);
    let dir = std::env::temp_dir().join(format!("agent-external-{}", id.uid));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |lifetime: std::time::Duration| {
        let (cert, key) = issuer.leaf(&uri, lifetime);
        std::fs::write(dir.join("tls.crt"), cert).unwrap();
        std::fs::write(dir.join("tls.key"), key.serialize_pem()).unwrap();
    };
    write(std::time::Duration::from_secs(4));
    std::fs::write(dir.join("tunnel-ca.crt"), gateway.ca.cert_pem()).unwrap();
    let vars = std::collections::HashMap::from([
        ("AGENT_PROXY_URL", format!("ws://{addr}/agent/v1/connect")),
        ("AGENT_CLUSTER", id.cluster()),
        ("AGENT_BOOTSTRAP_METHOD", "ExternalCA".to_string()),
        (
            "AGENT_TLS_CERT_PATH",
            dir.join("tls.crt").display().to_string(),
        ),
        (
            "AGENT_TLS_KEY_PATH",
            dir.join("tls.key").display().to_string(),
        ),
        (
            "AGENT_TUNNEL_CA_PATH",
            dir.join("tunnel-ca.crt").display().to_string(),
        ),
    ]);
    let config = agent::Config::from_lookup(|name| vars.get(name).cloned()).unwrap();
    let health = std::sync::Arc::new(agent::Health::default());
    tokio::spawn(agent::run(config, std::sync::Arc::clone(&health)));
    let up = |health: &agent::Health| health.tunnel_up.load(Ordering::Relaxed);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !up(&health) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the agent should open its tunnel with the external certificate");
    assert_eq!(gateway.registry.identities()[0].0, id);

    // The issuer renews: the next tunnel carries the later certificate.
    write(std::time::Duration::from_secs(24 * 3600));
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while !(health
            .reconnects
            .iter()
            .any(|count| count.load(Ordering::Relaxed) > 0)
            && up(&health))
        {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the agent should come back after its certificate expired");
    registered(&gateway, 1).await;

    drop(std::fs::remove_dir_all(&dir));
    delete_proxy(&pool, &id.namespace, &id.name).await;
}

/// T10: past the per-address limit, an enrolment is refused before its body
/// is read: no certificate is issued.
#[tokio::test]
async fn the_enroll_route_is_rate_limited_before_any_write() {
    use api::cluster::redirect::audit::capture;

    let pool = redis_or_skip!();
    capture::start();
    let (addr, _gateway) = agent_gateway_with(|settings| settings.enroll_per_minute = 2).await;
    let issuer = Issuer::new();
    let identity = seeded_cluster_with_jwks(&pool, 0, Some(issuer.jwks.clone())).await;
    let token = issuer.token(SUBJECT, DEFAULT_AUDIENCE, 600);
    // An address of its own in 127/8: the counter is per client address.
    let client_ip: std::net::IpAddr = format!(
        "127.{}.{}.{}",
        std::process::id() % 200 + 1,
        std::process::id() / 200 % 250,
        7
    )
    .parse()
    .unwrap();
    let client = reqwest::Client::builder()
        .local_address(client_ip)
        .build()
        .unwrap();
    let mut statuses = Vec::new();
    for _ in 0..3 {
        let response = client
            .post(format!("http://{addr}/agent/v1/enroll"))
            .json(&serde_json::json!({
                "cluster": identity.cluster(),
                "csr": new_csr(),
                "proof": { "service_account_token": token },
            }))
            .send()
            .await
            .unwrap();
        statuses.push(response.status().as_u16());
    }
    assert_eq!(statuses, [200, 200, 429]);
    let lines = capture::lines();
    let enrolled = lines
        .iter()
        .filter(|line| line["event"] == "agent_enrolled")
        .count();
    assert_eq!(enrolled, 2);
    assert!(
        lines
            .iter()
            .any(|line| line["event"] == "agent_refused" && line["reason"] == "rate limit")
    );
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// T11, decision 27: the tunnels of one identity on a pod, and the streams of
/// a tunnel (the CR's override included), are bounded.
#[tokio::test]
async fn stream_and_tunnel_limits_hold() {
    let pool = redis_or_skip!();
    let (addr, gateway) = agent_gateway_with(|settings| settings.tunnels_per_identity = 1).await;
    let identity = seeded_agent_cluster(&pool, 0).await;
    let mut proxy: crd::ProxyKubeApi = gateway
        .state
        .get_object_from_redis(crd::REDIS_PREFIX, &identity.cluster())
        .await
        .unwrap()
        .unwrap();
    if let Service::Agent(agent) = &mut proxy.spec.service {
        agent.limits = Some(crd::service::AgentLimits {
            streams_per_tunnel: Some(1),
        });
    }
    seed_proxy(&pool, &proxy).await;
    let (cert, key) = agent_cert(&gateway.ca, &identity);
    let _first = dial_agent(addr, gateway.ca.cert_pem(), &cert, &key)
        .await
        .unwrap();
    registered(&gateway, 1).await;
    // The second replica's tunnel is refused on this pod.
    assert!(
        dial_agent(addr, gateway.ca.cert_pem(), &cert, &key)
            .await
            .is_err()
    );
    assert_eq!(gateway.registry.identities()[0].1.tunnels, 1);

    let _stream = gateway.registry.open_stream(&identity.uid).await.unwrap();
    let second = gateway.registry.open_stream(&identity.uid).await;
    assert!(
        matches!(second, Err(api::agent::registry::OpenError::Limited)),
        "a second stream past streams_per_tunnel: 1"
    );
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// T12: the CA key appears in no audit line, no enrolment answer, no debug
/// output.
#[tokio::test]
async fn the_ca_key_never_reaches_a_log() {
    use api::cluster::redirect::audit::capture;

    let pool = redis_or_skip!();
    capture::start();
    let (addr, gateway) = agent_gateway().await;
    let issuer = Issuer::new();
    let identity = seeded_cluster_with_jwks(&pool, 0, Some(issuer.jwks.clone())).await;
    let token = issuer.token(SUBJECT, DEFAULT_AUDIENCE, 600);
    let (_, body) = enroll(addr, &identity.cluster(), &new_csr(), &token).await;
    let key_body: String = gateway
        .ca
        .key_pem()
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    assert!(!key_body.is_empty());
    assert!(!body.to_string().contains(&key_body));
    assert!(!format!("{:?}", gateway.ca).contains(&key_body));
    assert!(
        capture::lines()
            .iter()
            .all(|line| !line.to_string().contains(&key_body))
    );
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// T13: a `jwks_url` that is not `https://` (a CR that skipped admission)
/// is never fetched.
#[tokio::test]
async fn jwks_url_refuses_http_and_redirects() {
    let pool = redis_or_skip!();
    let issuer = Issuer::new();
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(issuer.jwks.clone()))
        .expect(0)
        .mount(&server)
        .await;
    let (addr, gateway) = agent_gateway().await;
    let identity = seeded_agent_cluster(&pool, 0).await;
    let mut proxy: crd::ProxyKubeApi = gateway
        .state
        .get_object_from_redis(crd::REDIS_PREFIX, &identity.cluster())
        .await
        .unwrap()
        .unwrap();
    if let Service::Agent(agent) = &mut proxy.spec.service {
        let bootstrap = service_account_token(&mut agent.bootstrap).unwrap();
        bootstrap.jwks_url = Some(format!("{}/keys", server.uri()));
    }
    seed_proxy(&pool, &proxy).await;
    let token = issuer.token(SUBJECT, DEFAULT_AUDIENCE, 600);
    let (status, _) = enroll(addr, &identity.cluster(), &new_csr(), &token).await;
    assert_eq!(status, 401);
    // `expect(0)`: wiremock checks on drop that nothing was fetched.
    drop(server);
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// T15: the agents' listener serves the agent routes and the probes, none of
/// the user API.
#[tokio::test]
async fn user_routes_are_not_served_on_the_agent_port() {
    let (addr, _gateway) = agent_gateway().await;
    for path in [
        "/clusters/default/prod/api/v1/pods",
        "/api/clusters",
        "/management/ready",
        "/oauth/authorize",
        "/",
    ] {
        let status = reqwest::get(format!("http://{addr}{path}"))
            .await
            .unwrap()
            .status();
        assert_eq!(status, 404, "{path}");
    }
}

/// T19: a presence member is a pod name, never an address, and it only
/// routes to a ready pod of the gateway's Service as the Kubernetes API lists
/// it: here the API cannot be read, so nothing routes, forged or not.
#[tokio::test]
async fn a_presence_names_a_gateway_pod_never_an_address() {
    use common::agent_gateway::{GatewayClient, presence_key};

    let _pool = redis_or_skip!();
    let state = test_state("https://idp.example.com".to_string());
    let uid = format!("{:08x}-0000-4000-8000-0000000000f1", std::process::id());
    let key = presence_key(&uid);
    let members = ["attacker.example.com:443", "10.0.0.1", "gw-0.evil", "gw-1"];
    for member in members {
        state.presence_refresh(&key, member, 30).await.unwrap();
    }
    let client = GatewayClient::with(
        Some("proxyauth-gateway-pods.proxyauth.svc".to_string()),
        8445,
        Some("t0ken".to_string()),
    );
    let route = client.route(&state, &uid).await;
    assert!(route.is_err(), "{route:?}");
    for member in members {
        drop(state.presence_remove(&key, member).await);
    }
}

/// T17: `jwks_from` is read at every enrolment: a rotated key is accepted at
/// once, and an emptied ConfigMap refuses instead of letting anyone in.
#[cfg(feature = "envtest")]
#[tokio::test]
async fn a_jwks_from_change_is_picked_up_and_an_empty_one_refuses() {
    use k8s_openapi::api::core::v1::ConfigMap;
    use kube::api::{Api, ObjectMeta, Patch, PatchParams, PostParams};

    let Some(env_test) = envtest_support::EnvTest::try_start().await else {
        return;
    };
    let pool = redis_or_skip!();
    let client = env_test.client().unwrap();
    let mut state = test_state("https://idp.example.com".to_string());
    state.client = client.clone();
    let mut settings = common::config::Config::from_lookup(|_| None).agent_gateway;
    settings.enroll_per_minute = 10_000;
    settings.enroll_per_minute_total = 10_000;
    let gateway = api::agent::listener::Gateway::new(
        state,
        api::agent::registry::TunnelRegistry::new(),
        std::sync::Arc::new(agent_pki::AgentCa::generate().unwrap()),
        settings,
        "gw-0".to_string(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        drop(axum::serve(listener, api::agent::listener::router(gateway)).await);
    });

    let first = Issuer::new();
    let configmaps: Api<ConfigMap> = Api::namespaced(client, "default");
    let name = format!("agent-jwks-{}", std::process::id());
    configmaps
        .create(
            &PostParams::default(),
            &ConfigMap {
                metadata: ObjectMeta {
                    name: Some(name.clone()),
                    ..ObjectMeta::default()
                },
                data: Some([("jwks.json".to_string(), first.jwks.clone())].into()),
                ..ConfigMap::default()
            },
        )
        .await
        .unwrap();
    let identity = seeded_agent_cluster(&pool, 0).await;
    let mut proxy: crd::ProxyKubeApi = test_state("https://idp.example.com".to_string())
        .get_object_from_redis(crd::REDIS_PREFIX, &identity.cluster())
        .await
        .unwrap()
        .unwrap();
    if let Service::Agent(agent) = &mut proxy.spec.service {
        let bootstrap = service_account_token(&mut agent.bootstrap).unwrap();
        bootstrap.jwks_url = None;
        bootstrap.jwks_from = Some(crd::service::JwksSource::ConfigMap {
            name: name.clone(),
            key: "jwks.json".to_string(),
            namespace: None,
        });
    }
    seed_proxy(&pool, &proxy).await;
    let enroll_with = |issuer: &Issuer| {
        let token = issuer.token(SUBJECT, DEFAULT_AUDIENCE, 600);
        let cluster = identity.cluster();
        async move { enroll(addr, &cluster, &new_csr(), &token).await.0 }
    };
    assert_eq!(enroll_with(&first).await, 200);

    // The key rotates: the next enrolment reads the new set.
    let second = Issuer::new();
    let patch = |jwks: String| Patch::Merge(serde_json::json!({ "data": { "jwks.json": jwks } }));
    configmaps
        .patch(&name, &PatchParams::default(), &patch(second.jwks.clone()))
        .await
        .unwrap();
    assert_eq!(enroll_with(&second).await, 200);
    assert_eq!(enroll_with(&first).await, 401);

    // Emptied: refused, never open.
    configmaps
        .patch(
            &name,
            &PatchParams::default(),
            &patch(r#"{"keys":[]}"#.to_string()),
        )
        .await
        .unwrap();
    assert_eq!(enroll_with(&second).await, 401);

    drop(
        configmaps
            .delete(&name, &kube::api::DeleteParams::default())
            .await,
    );
    delete_proxy(&pool, &identity.namespace, &identity.name).await;
}

/// §4.2: with `validate_against: Kubernetes`, a cluster without a tunnel is
/// `503 agent unavailable`, not a `401`: the token could not be reviewed,
/// it was not refused.
#[tokio::test]
async fn token_review_without_a_tunnel_is_agent_unavailable() {
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://unused.example.com");
    proxy.spec.service = agent_service();
    proxy.spec.auth_config = Some(harness::kubernetes_auth_config());
    proxy.metadata.uid = Some(format!(
        "{:08x}-0000-4000-8000-0000000000e1",
        std::process::id()
    ));
    seed_proxy(&pool, &proxy).await;

    let mut state = test_state("https://idp.example.com".to_string());
    state.agent_gateway = std::sync::Arc::new(common::agent_gateway::GatewayClient::with(
        Some("gw.test".to_string()),
        0,
        Some("t0ken".to_string()),
    ));
    let app = test::init_service(harness::app(state)).await;
    let request = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", "Bearer some-token"))
        .to_request();
    let resp = test::call_service(&app, request).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["message"], "agent unavailable");
    delete_proxy(&pool, &ns, &cluster).await;
}
