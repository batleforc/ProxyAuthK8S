//! The gateway's agent listener (RFC 0012 §5.3, §6.3).
//!
//! `GET /agent/v1/connect` upgrades to a WebSocket. With `tlsMode: inner`,
//! the agent and this pod run a mutual TLS under the agent CA inside it, and
//! yamux over that TLS; with `passthrough`, this pod ends the TLS of the
//! connection itself, the agent's certificate is that TLS's client
//! certificate, and yamux runs on the WebSocket. Either way, the TLS that
//! names the agent ends in this pod. The agent's identity is its
//! certificate's, checked against the `ProxyKubeApi`
//! it names (UID, `epoch`, `Agent` service): no header set by an ingress is
//! read as identity. The first stream the agent opens is the control channel
//! (`AgentInfo` from the agent, then `Hello`); the agent opens no other.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use agent_pki::{AgentCa, AgentIdentity, external_identity_of, identity_of};
use agent_proto::control::{self, AgentInfo, Close, Hello, Kind, Pong, RenewResponse};
use agent_proto::mux::{self, Mode};
use agent_proto::ws_io::{MAX_FRAME, Message, WsIo};
use axum::extract::ws::{self, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Extension};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use axum::{Router, http::StatusCode};
use common::State;
use common::agent_gateway::{AgentRefusal, REFUSAL_TTL, agent_refused_key};
use common::config::{AgentGatewaySettings, AgentTlsMode};
use crd::ProxyKubeApi;
use crd::service::{AgentBootstrap, AgentService, Service};
use futures_util::{AsyncRead, AsyncWrite, SinkExt, StreamExt, future};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::service::TowerToHyperService;
use rustls::client::danger::HandshakeSignatureValid;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{ServerConfig, WebPkiClientVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, RootCertStore, SignatureScheme};
use tokio_rustls::TlsAcceptor;
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};
use tracing::{debug, info, warn};

use crate::cluster::redirect::audit::{AuditEvent, Event, Outcome};
use crate::cluster::redirect::forwarded::throttle_client_ip;
use crate::helper::{forwarded_for, peer_ip};

use super::registry::TunnelRegistry;

/// A control channel silent this long is a dead tunnel: the agent pings
/// every 30 s.
const CONTROL_IDLE: Duration = Duration::from_secs(90);
/// The tunnel server certificate lives 7 days; a new one is issued daily.
/// In `passthrough`, the chart's certificate is read again as often.
const SERVER_CERT_REISSUE: Duration = Duration::from_secs(24 * 60 * 60);

/// Why a tunnel was refused or closed, for the logs and the audit (T14).
pub type Reason = &'static str;

/// The client certificate chain of a `passthrough` connection, set by
/// [`serve_agents`] on each of its requests; never present with `inner`.
#[derive(Clone)]
pub struct ClientCertificate(pub Option<Vec<CertificateDer<'static>>>);

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

fn refused(client: &str, identity: Option<&AgentIdentity>, reason: Reason) {
    refused_announcing(client, identity, None, reason);
}

/// [`refused`], with the version the agent announced once it has (RFC 0013).
fn refused_announcing(
    client: &str,
    identity: Option<&AgentIdentity>,
    version: Option<&str>,
    reason: Reason,
) {
    warn!(%client, cluster = identity.map(AgentIdentity::cluster), version, reason, "agent tunnel refused");
    crate::metrics::record_agent_refusal(reason);
    let line = AuditEvent::new(Event::AgentRefused, Outcome::Denied)
        .client_ip(client)
        .reason(reason);
    match identity {
        Some(identity) => line
            .cluster(&identity.namespace, &identity.name)
            .agent(identity.epoch, version),
        None => line,
    }
    .emit();
}

/// Accepts a client certificate the agent CA issued, as before; and,
/// provisionally, one naming a cluster by `proxyauth://<ns>/<name>`
/// (`ExternalCA`, decision 28), whose chain [`Gateway::identify`] checks
/// after the handshake against that cluster's CA. The handshake proves the
/// key either way.
#[derive(Debug)]
struct AgentClientVerifier(Arc<dyn ClientCertVerifier>);

impl ClientCertVerifier for AgentClientVerifier {
    fn offer_client_auth(&self) -> bool {
        self.0.offer_client_auth()
    }

    fn client_auth_mandatory(&self) -> bool {
        self.0.client_auth_mandatory()
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        // The agent CA alone would steer an external agent away.
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        self.0
            .verify_client_cert(end_entity, intermediates, now)
            .or_else(|err| match external_identity_of(end_entity) {
                Ok(_) => Ok(ClientCertVerified::assertion()),
                Err(_) => Err(err),
            })
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

pub struct Gateway {
    pub state: State,
    pub registry: Arc<TunnelRegistry>,
    pub ca: Arc<AgentCa>,
    pub settings: AgentGatewaySettings,
    /// This pod's name: the `replica_id` of `Hello`.
    pub pod: String,
    tls: Mutex<Option<(Instant, Arc<ServerConfig>)>>,
    /// The `jwks_url` keys of enrolment proofs.
    pub(super) jwks: jwt_validator::JwksCache,
    pub(super) http: Option<reqwest::Client>,
}

impl Gateway {
    #[must_use]
    pub fn new(
        state: State,
        registry: Arc<TunnelRegistry>,
        ca: Arc<AgentCa>,
        settings: AgentGatewaySettings,
        pod: String,
    ) -> Arc<Self> {
        // The bound `PROXY_JWKS_MAX_DOCUMENT_BYTES` set on the state's validator.
        let jwks = jwt_validator::JwksCache::with_max_document_bytes(
            state.jwt_validator.max_document_bytes(),
        );
        Arc::new(Self {
            state,
            registry,
            ca,
            settings,
            pod,
            tls: Mutex::new(None),
            jwks,
            // T13: no redirect followed, 5 s per fetch; a client that cannot
            // be built (no TLS backend) refuses every `jwks_url`.
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(5))
                .build()
                .ok(),
        })
    }

    pub(super) fn tls_config(&self) -> Result<Arc<ServerConfig>, String> {
        let mut tls = self.tls.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((issued, config)) = tls.as_ref()
            && issued.elapsed() < SERVER_CERT_REISSUE
        {
            return Ok(Arc::clone(config));
        }
        let config = Arc::new(match &self.settings.tls_mode {
            AgentTlsMode::Inner => server_config(&self.ca)?,
            AgentTlsMode::Passthrough {
                cert_path,
                key_path,
            } => passthrough_config(&self.ca, cert_path, key_path)?,
        });
        *tls = Some((Instant::now(), Arc::clone(&config)));
        Ok(config)
    }

    /// Who a verified client chain speaks for: a certificate of the agent
    /// CA names its identity; an external one (decision 28) names a cluster
    /// whose bootstrap must be `ExternalCA`, chains to that cluster's CA, and
    /// lives no longer than its `max_cert_lifetime_hours` (decision 29). With
    /// it, when an external certificate expires: its tunnel closes then.
    async fn identify(
        &self,
        chain: &[CertificateDer<'static>],
    ) -> Result<(AgentIdentity, Option<i64>), Reason> {
        let end_entity = chain.first().ok_or("identity")?;
        if let Ok(peer) = identity_of(end_entity) {
            // Only the agent CA lets a four-segment identity through the handshake.
            return Ok((peer.identity, None));
        }
        let peer = external_identity_of(end_entity).map_err(|_| "identity")?;
        let proxy: ProxyKubeApi = self
            .state
            .get_proxy_from_redis(&peer.cluster())
            .await
            .map_err(|_| "redis")?
            .ok_or("unknown cluster")?;
        let Service::Agent(agent) = &proxy.spec.service else {
            return Err("not an agent cluster");
        };
        let AgentBootstrap::ExternalCA(external) = &agent.bootstrap else {
            return Err("method");
        };
        let pem = match (&external.ca, &external.ca_ref) {
            (Some(inline), _) => inline.clone(),
            (None, Some(source)) => source
                .as_cert_source()
                .get_cert(self.state.client.clone(), &peer.namespace)
                .await
                .map_err(|_| "ca")?
                .ok_or("ca")?,
            (None, None) => return Err("ca"),
        };
        let mut roots = RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(pem.as_bytes()) {
            roots.add(cert.map_err(|_| "ca")?).map_err(|_| "ca")?;
        }
        WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|_| "ca")?
            .verify_client_cert(
                end_entity,
                chain.get(1..).unwrap_or_default(),
                UnixTime::now(),
            )
            .map_err(|_| "chain")?;
        let (not_before, not_after) = (
            peer.not_before.unix_timestamp(),
            peer.not_after.unix_timestamp(),
        );
        if not_after.saturating_sub(not_before)
            > i64::from(external.max_cert_lifetime_hours).saturating_mul(3600)
        {
            return Err("lifetime");
        }
        Ok((
            AgentIdentity {
                namespace: peer.namespace,
                name: peer.name,
                uid: proxy.metadata.uid.clone().ok_or("uid")?,
                epoch: agent.epoch,
            },
            Some(not_after),
        ))
    }

    async fn serve(
        self: Arc<Self>,
        socket: WebSocket,
        client: String,
        client_chain: Option<Vec<CertificateDer<'static>>>,
    ) {
        let refused = |identity: Option<&AgentIdentity>, reason: Reason| {
            refused(&client, identity, reason);
        };
        let socket = WsIo::new(
            socket
                .map(|message| message.map(to_tungstenite))
                .with(|message| future::ready(Ok::<_, axum::Error>(from_tungstenite(message)))),
        );
        let (io, chain): (Box<dyn Io>, _) = match &self.settings.tls_mode {
            // `connect` let no request without a certificate through.
            AgentTlsMode::Passthrough { .. } => {
                (Box::new(socket), client_chain.unwrap_or_default())
            }
            AgentTlsMode::Inner => {
                let tls = match self.tls_config() {
                    Ok(tls) => tls,
                    Err(err) => {
                        return warn!(error = %err, "agent tunnel: no server certificate");
                    }
                };
                let handshake = TlsAcceptor::from(tls).accept(socket.compat());
                let tls =
                    match tokio::time::timeout(self.settings.handshake_timeout, handshake).await {
                        Ok(Ok(tls)) => tls,
                        Ok(Err(err)) => {
                            debug!(error = %err, "agent tunnel TLS failed");
                            return refused(None, "tls");
                        }
                        Err(_) => return refused(None, "tls timeout"),
                    };
                let chain = tls
                    .get_ref()
                    .1
                    .peer_certificates()
                    .map(|chain| chain.iter().map(|cert| cert.clone().into_owned()).collect())
                    .unwrap_or_default();
                (Box::new(TokioAsyncReadCompatExt::compat(tls)), chain)
            }
        };
        let (identity, expires) = match self.identify(&chain).await {
            Ok(identified) => identified,
            Err(reason) => return refused(None, reason),
        };
        let agent = match still_valid(&self.state, &identity).await {
            Ok(agent) => agent,
            Err(reason) => return refused(Some(&identity), reason),
        };
        // Decision 27: the CR may raise its bound, never past the gateway's.
        let (streams, capped) = agent.streams_per_tunnel(
            self.settings.streams_per_tunnel,
            self.settings.max_streams_per_tunnel,
        );
        if capped {
            warn!(cluster = %identity.cluster(), streams, "limits.streams_per_tunnel capped at the gateway's maximum");
        }

        let (handle, mut inbound, driver) =
            // One more for the control stream, which yamux counts too.
            mux::new(io, Mode::Client, streams.saturating_add(1));
        let connected = AtomicBool::new(false);
        let announced = std::sync::OnceLock::new();
        let session = async {
            let mut control = tokio::time::timeout(self.settings.handshake_timeout, inbound.recv())
                .await
                .ok()
                .flatten()
                .ok_or("no control stream")?;
            let ended = self
                .session(
                    &identity,
                    &mut control,
                    (handle, streams),
                    &mut inbound,
                    (&client, &connected, &announced),
                    expires,
                )
                .await;
            if let Err(reason) = ended {
                // Best effort: the agent learns why before the tunnel drops.
                let close = Kind::Close(Close {
                    reason: reason.to_string(),
                    protocol_min: agent_proto::PROTOCOL_MIN,
                    protocol_max: agent_proto::PROTOCOL_VERSION,
                });
                drop(
                    tokio::time::timeout(
                        Duration::from_secs(1),
                        control::write(&mut control, close),
                    )
                    .await,
                );
            }
            ended
        };
        let mut driver = std::pin::pin!(driver);
        let ended = tokio::select! {
            ended = session => ended,
            _ = &mut driver => Err("driver"),
        };
        if ended != Err("driver") {
            // The `Close` frame is only queued: drive it out, or let the agent hang up.
            drop(tokio::time::timeout(Duration::from_secs(1), driver).await);
        }
        let reason = ended.err().unwrap_or("agent");
        if !connected.load(Ordering::Acquire) {
            return refused_announcing(
                &client,
                Some(&identity),
                announced.get().map(String::as_str),
                reason,
            );
        }
        info!(cluster = %identity.cluster(), reason, "agent tunnel closed");
        AuditEvent::new(Event::AgentDisconnected, Outcome::Allowed)
            .client_ip(&client)
            .cluster(&identity.namespace, &identity.name)
            .agent(identity.epoch, None)
            .reason(reason)
            .emit();
    }

    async fn session(
        &self,
        identity: &AgentIdentity,
        control: &mut mux::Stream,
        (handle, streams): (agent_proto::mux::MuxHandle, usize),
        inbound: &mut tokio::sync::mpsc::Receiver<mux::Stream>,
        (client, connected, announced): (&str, &AtomicBool, &std::sync::OnceLock<String>),
        expires: Option<i64>,
    ) -> Result<(), Reason> {
        // yamux announces a stream with its first frame: the agent speaks first.
        let info =
            match tokio::time::timeout(self.settings.handshake_timeout, control::read(control))
                .await
            {
                Ok(Ok(Kind::AgentInfo(info))) => info,
                Ok(Ok(_)) => return Err("protocol"),
                Ok(Err(_)) | Err(_) => return Err("control"),
            };
        let AgentInfo {
            version,
            ca_fingerprint,
            protocol_min,
            protocol_max,
        } = &info;
        // What the agent announces reaches Redis and the CR's status: only
        // in a bounded, known shape (RFC 0013, T1).
        let version = agent_proto::valid_version(version).unwrap_or_else(|| {
            warn!(cluster = %identity.cluster(), "the agent announced an invalid version");
            "invalid"
        });
        let ca_fingerprint = if ca_fingerprint.is_empty()
            || agent_proto::valid_fingerprint(ca_fingerprint)
        {
            ca_fingerprint.as_str()
        } else {
            warn!(cluster = %identity.cluster(), "the agent announced an invalid CA fingerprint");
            ""
        };
        announced.get_or_init(|| version.to_string());
        let protocols = (*protocol_min, *protocol_max);
        // Before `register`: a refused tunnel is never routed (RFC 0013 §5.1).
        let Some(protocol) = agent_proto::negotiate(
            protocols,
            (agent_proto::PROTOCOL_MIN, agent_proto::PROTOCOL_VERSION),
        ) else {
            self.record_protocol_refusal(identity, version, protocols)
                .await;
            return Err("protocol");
        };
        let guard = self
            .registry
            .register(
                identity.clone(),
                handle,
                self.settings.tunnels_per_identity,
                streams,
            )
            .map_err(|_| "too many tunnels")?;
        // Registered first: `Hello` tells the agent its tunnel is accepted.
        let hello = Kind::Hello(Hello {
            replica_id: self.pod.clone(),
            replicas_expected: 1,
            protocol,
        });
        control::write(control, hello)
            .await
            .map_err(|_| "control")?;
        guard.describe(version, ca_fingerprint, protocol);
        info!(cluster = %identity.cluster(), %version, protocol, streams, "agent tunnel open");
        connected.store(true, Ordering::Release);
        AuditEvent::new(Event::AgentConnected, Outcome::Allowed)
            .client_ip(client)
            .cluster(&identity.namespace, &identity.name)
            .agent(identity.epoch, Some(version))
            .emit();
        let mut closed = guard.closed.clone();
        let expiry = async {
            match expires {
                Some(not_after) => {
                    let left = u64::try_from(not_after.saturating_sub(unix_now())).unwrap_or(0);
                    tokio::time::sleep(Duration::from_secs(left)).await;
                }
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            ended = self.control_loop(identity, control) => ended,
            _ = closed.wait_for(|closed| *closed) => Err("revoked"),
            // An external certificate cannot be renewed here (decision 29).
            () = expiry => Err("expired"),
            // An agent opens no data stream: each is reset on arrival.
            () = async { while inbound.recv().await.is_some() {} } => Err("tunnel closed"),
        }
    }

    /// The refusal the controller turns into `AgentProtocolUnsupported`,
    /// keyed by the certificate's UID, never by what the agent announced
    /// (RFC 0013, T5). A Redis failure loses the cause, not the refusal.
    async fn record_protocol_refusal(
        &self,
        identity: &AgentIdentity,
        version: &str,
        (min, max): (u32, u32),
    ) {
        let record = AgentRefusal {
            reason: "protocol".to_string(),
            version: version.to_string(),
            protocols: format!("{min}-{max}"),
        };
        let written = self
            .state
            .redis_set(
                &agent_refused_key(&identity.uid),
                &serde_json::to_string(&record).unwrap_or_default(),
                Some(REFUSAL_TTL.as_secs()),
            )
            .await;
        if let Err(err) = written {
            warn!(cluster = %identity.cluster(), error = %err, "agent protocol refusal not recorded");
        }
    }

    async fn control_loop(
        &self,
        identity: &AgentIdentity,
        control: &mut mux::Stream,
    ) -> Result<(), Reason> {
        loop {
            let message = match tokio::time::timeout(CONTROL_IDLE, control::read(control)).await {
                Ok(Ok(message)) => message,
                Ok(Err(_)) => return Err("control"),
                Err(_) => return Err("idle"),
            };
            let reply = match message {
                Kind::Ping(ping) => Kind::Pong(Pong { nonce: ping.nonce }),
                Kind::RenewRequest(renew) => {
                    let agent = still_valid(&self.state, identity).await?;
                    // Its issuer renews an external certificate, never this CA.
                    if matches!(agent.bootstrap, AgentBootstrap::ExternalCA(_)) {
                        return Err("method");
                    }
                    let csr = String::from_utf8(renew.csr).map_err(|_| "csr")?;
                    let cert = self.ca.issue_agent(&csr, identity).map_err(|_| "csr")?;
                    Kind::RenewResponse(RenewResponse {
                        cert: cert.into_bytes(),
                    })
                }
                Kind::Close(_) => return Ok(()),
                _ => return Err("protocol"),
            };
            control::write(control, reply)
                .await
                .map_err(|_| "control")?;
        }
    }
}

/// Whether `identity` still names its `ProxyKubeApi`: same UID, same `epoch`,
/// still an `Agent` service (T8). Returns that service.
pub async fn still_valid(state: &State, identity: &AgentIdentity) -> Result<AgentService, Reason> {
    let agent = current_agent(state, identity).await?;
    if agent.epoch == identity.epoch {
        Ok(agent)
    } else {
        Err("epoch")
    }
}

/// The `Agent` service of the `ProxyKubeApi` `identity` names, or why that CR
/// no longer accepts the identity's UID (`"redis"` when it cannot be read).
pub async fn current_agent(
    state: &State,
    identity: &AgentIdentity,
) -> Result<AgentService, Reason> {
    let proxy: ProxyKubeApi = state
        .get_proxy_from_redis(&identity.cluster())
        .await
        .map_err(|_| "redis")?
        .ok_or("unknown cluster")?;
    if proxy.metadata.uid.as_deref() != Some(identity.uid.as_str()) {
        return Err("uid");
    }
    match proxy.spec.service {
        Service::Agent(agent) => Ok(agent),
        _ => Err("not an agent cluster"),
    }
}

/// The agent CA, the one root of agent certificates.
fn agent_roots(ca: &AgentCa) -> Result<Arc<RootCertStore>, String> {
    let mut roots = RootCertStore::empty();
    roots
        .add(
            CertificateDer::from_pem_slice(ca.cert_pem().as_bytes())
                .map_err(|err| err.to_string())?,
        )
        .map_err(|err| err.to_string())?;
    Ok(Arc::new(roots))
}

/// The `passthrough` TLS: the chart's certificate, and a client certificate
/// from the agent CA, optional at the handshake (enrolment and probes have
/// none) and required by `connect`. A certificate from another CA still
/// fails the handshake.
fn passthrough_config(
    ca: &AgentCa,
    cert_path: &str,
    key_path: &str,
) -> Result<ServerConfig, String> {
    let certs = CertificateDer::pem_file_iter(cert_path)
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|err| format!("{cert_path}: {err}"))?;
    let key = PrivateKeyDer::from_pem_file(key_path).map_err(|err| format!("{key_path}: {err}"))?;
    let verifier = Arc::new(AgentClientVerifier(
        WebPkiClientVerifier::builder(agent_roots(ca)?)
            .allow_unauthenticated()
            .build()
            .map_err(|err| err.to_string())?,
    ));
    let mut config = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .map_err(|err| err.to_string())?;
    // A WebSocket upgrade is HTTP/1.1.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

/// The tunnel TLS: a fresh server certificate under the agent CA, and a
/// client certificate required from that CA alone.
fn server_config(ca: &AgentCa) -> Result<ServerConfig, String> {
    let (cert_pem, key_pem) = ca.issue_server().map_err(|err| err.to_string())?;
    let verifier = Arc::new(AgentClientVerifier(
        WebPkiClientVerifier::builder(agent_roots(ca)?)
            .build()
            .map_err(|err| err.to_string())?,
    ));
    ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![
                CertificateDer::from_pem_slice(cert_pem.as_bytes())
                    .map_err(|err| err.to_string())?,
            ],
            PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).map_err(|err| err.to_string())?,
        )
        .map_err(|err| err.to_string())
}

fn to_tungstenite(message: ws::Message) -> Message {
    match message {
        ws::Message::Binary(data) => Message::Binary(data),
        ws::Message::Ping(data) => Message::Ping(data),
        ws::Message::Pong(data) => Message::Pong(data),
        ws::Message::Close(_) => Message::Close(None),
        // `WsIo` fails the read on any text frame.
        ws::Message::Text(_) => Message::Text(String::new().into()),
    }
}

fn from_tungstenite(message: Message) -> ws::Message {
    match message {
        Message::Binary(data) => ws::Message::Binary(data),
        // `WsIo` writes binary frames only.
        _ => ws::Message::Close(None),
    }
}

async fn connect(
    axum::extract::State(gateway): axum::extract::State<Arc<Gateway>>,
    parts: axum::http::request::Parts,
    upgrade: WebSocketUpgrade,
) -> Response {
    let client = throttle_client_ip(forwarded_for(&parts.headers), peer_ip(&parts));
    let client_chain = parts
        .extensions
        .get::<ClientCertificate>()
        .and_then(|cert| cert.0.clone());
    // Before the upgrade: no WebSocket for a client without a certificate;
    // whom it names is `identify`'s to say.
    if matches!(gateway.settings.tls_mode, AgentTlsMode::Passthrough { .. })
        && client_chain.as_ref().is_none_or(Vec::is_empty)
    {
        refused(&client, None, "identity");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // `WsIo` writes frames of `MAX_FRAME` at most: the agent sends no larger.
    upgrade
        .max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |socket| gateway.serve(socket, client, client_chain))
}

/// Ready while Redis answers: no tunnel can be checked, nor enrolment counted,
/// without it.
async fn ready(
    axum::extract::State(gateway): axum::extract::State<Arc<Gateway>>,
) -> axum::http::StatusCode {
    match tokio::time::timeout(Duration::from_secs(2), gateway.state.redis_ping()).await {
        Ok(Ok(())) => axum::http::StatusCode::OK,
        _ => axum::http::StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Serve [`router`] on `listener` until it fails: plain HTTP with `inner`
/// (the ingress ends the TLS), this pod's TLS with `passthrough`, each
/// request carrying the connection's [`ClientCertificate`].
pub async fn serve_agents(
    listener: tokio::net::TcpListener,
    gateway: Arc<Gateway>,
) -> std::io::Result<()> {
    let app = router(Arc::clone(&gateway));
    if gateway.settings.tls_mode == AgentTlsMode::Inner {
        // The peer address names the enrolling client (`TRUSTED_PROXY_COUNT`).
        return axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    }
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(err) => {
                // Out of descriptors, say: wait, as `axum::serve` does.
                warn!(error = %err, "agent listener accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let (gateway, app) = (Arc::clone(&gateway), app.clone());
        tokio::spawn(async move {
            let client = throttle_client_ip(None, Some(peer.ip()));
            let tls = match gateway.tls_config() {
                Ok(tls) => tls,
                Err(err) => return warn!(error = %err, "agent listener: no server certificate"),
            };
            let handshake = TlsAcceptor::from(tls).accept(tcp);
            let tls =
                match tokio::time::timeout(gateway.settings.handshake_timeout, handshake).await {
                    Ok(Ok(tls)) => tls,
                    Ok(Err(err)) => {
                        debug!(error = %err, "agent listener TLS failed");
                        return refused(&client, None, "tls");
                    }
                    Err(_) => return refused(&client, None, "tls timeout"),
                };
            let cert = ClientCertificate(
                tls.get_ref()
                    .1
                    .peer_certificates()
                    .map(|chain| chain.iter().map(|cert| cert.clone().into_owned()).collect()),
            );
            let app = app
                .layer(Extension(cert))
                .layer(Extension(ConnectInfo(peer)));
            let served = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                .http1_only()
                .serve_connection_with_upgrades(TokioIo::new(tls), TowerToHyperService::new(app))
                .await;
            if let Err(err) = served {
                debug!(error = %err, "agent listener connection ended");
            }
        });
    }
}

/// The routes of the agents' listener (`agentGateway.port`). Behind an
/// ingress, `TRUSTED_PROXY_COUNT` names the enrolling client's address.
pub fn router(gateway: Arc<Gateway>) -> Router {
    Router::new()
        .route("/agent/v1/connect", get(connect))
        .route(
            "/agent/v1/enroll",
            axum::routing::post(super::enroll::enroll),
        )
        .route("/healthz", get(|| async { axum::http::StatusCode::OK }))
        .route("/readyz", get(ready))
        .with_state(gateway)
}
