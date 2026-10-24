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
    let state = test_state("https://idp.example.com".to_string());
    let ca = std::sync::Arc::new(agent_pki::AgentCa::generate().unwrap());
    let registry = api::agent::registry::TunnelRegistry::new();
    let mut settings = common::config::Config::from_lookup(|_| None).agent_gateway;
    settings.handshake_timeout = std::time::Duration::from_secs(2);
    // Every test enrols from the same address, in one shared Redis.
    settings.enroll_per_minute = 10_000;
    settings.enroll_per_minute_total = 10_000;
    let gateway =
        api::agent::listener::Gateway::new(state, registry, ca, settings, "gw-0".to_string());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = api::agent::listener::router(std::sync::Arc::clone(&gateway));
    tokio::spawn(async move {
        drop(axum::serve(listener, router).await);
    });
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
    agent_proto::control::write(
        &mut control,
        agent_proto::control::Kind::AgentInfo(agent_proto::control::AgentInfo {
            version: "test".to_string(),
            ca_fingerprint: String::new(),
        }),
    )
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
            let AgentBootstrap::ServiceAccountToken(bootstrap) = &mut agent.bootstrap;
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

/// T9, §5.2: a token for another subject or audience, expired, signed by
/// another key, or for a cluster that is not `Agent`, gets a bare 401.
#[tokio::test]
async fn a_bad_proof_is_refused_without_detail() {
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
            reason: "revoked".to_string()
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
