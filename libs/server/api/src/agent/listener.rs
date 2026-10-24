//! The gateway's agent listener (RFC 0012 §5.3, §6.3, `tlsMode: inner`).
//!
//! `GET /agent/v1/connect` upgrades to a WebSocket; inside it, the agent and
//! this pod run a mutual TLS under the agent CA, and yamux over that TLS. The
//! agent's identity is its certificate's, checked against the `ProxyKubeApi`
//! it names (UID, `epoch`, `Agent` service): no header set by an ingress is
//! read as identity. The first stream the agent opens is the control channel
//! (`AgentInfo` from the agent, then `Hello`); the agent opens no other.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use agent_pki::{AgentCa, AgentIdentity, identity_of};
use agent_proto::control::{self, AgentInfo, Close, Hello, Kind, Pong, RenewResponse};
use agent_proto::mux::{self, Mode};
use agent_proto::ws_io::{MAX_FRAME, Message, WsIo};
use axum::Router;
use axum::extract::ws::{self, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::get;
use common::State;
use common::config::AgentGatewaySettings;
use crd::ProxyKubeApi;
use crd::service::{AgentService, Service};
use futures_util::{SinkExt, StreamExt, future};
use rustls::RootCertStore;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ServerConfig, WebPkiClientVerifier};
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
const SERVER_CERT_REISSUE: Duration = Duration::from_secs(24 * 60 * 60);

/// Why a tunnel was refused or closed, for the logs and the audit (T14).
pub type Reason = &'static str;

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
        Arc::new(Self {
            state,
            registry,
            ca,
            settings,
            pod,
            tls: Mutex::new(None),
            jwks: jwt_validator::JwksCache::new(),
            // T13: no redirect followed, 5 s per fetch; a client that cannot
            // be built (no TLS backend) refuses every `jwks_url`.
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(5))
                .build()
                .ok(),
        })
    }

    fn tls_config(&self) -> Result<Arc<ServerConfig>, String> {
        let mut tls = self.tls.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((issued, config)) = tls.as_ref()
            && issued.elapsed() < SERVER_CERT_REISSUE
        {
            return Ok(Arc::clone(config));
        }
        let config = Arc::new(server_config(&self.ca)?);
        *tls = Some((Instant::now(), Arc::clone(&config)));
        Ok(config)
    }

    async fn serve(self: Arc<Self>, socket: WebSocket, client: String) {
        let refused = |identity: Option<&AgentIdentity>, reason: Reason| {
            warn!(%client, cluster = identity.map(AgentIdentity::cluster), reason, "agent tunnel refused");
            let line = AuditEvent::new(Event::AgentRefused, Outcome::Denied)
                .client_ip(&client)
                .reason(reason);
            match identity {
                Some(identity) => line
                    .cluster(&identity.namespace, &identity.name)
                    .agent(identity.epoch, None),
                None => line,
            }
            .emit();
        };
        let socket = socket
            .map(|message| message.map(to_tungstenite))
            .with(|message| future::ready(Ok::<_, axum::Error>(from_tungstenite(message))));
        let tls = match self.tls_config() {
            Ok(tls) => tls,
            Err(err) => return warn!(error = %err, "agent tunnel: no server certificate"),
        };
        let handshake = TlsAcceptor::from(tls).accept(WsIo::new(socket).compat());
        let tls = match tokio::time::timeout(self.settings.handshake_timeout, handshake).await {
            Ok(Ok(tls)) => tls,
            Ok(Err(err)) => {
                debug!(error = %err, "agent tunnel TLS failed");
                return refused(None, "tls");
            }
            Err(_) => return refused(None, "tls timeout"),
        };
        let identity = match tls
            .get_ref()
            .1
            .peer_certificates()
            .and_then(<[_]>::first)
            .map(|cert| identity_of(cert))
        {
            Some(Ok(peer)) => peer.identity,
            _ => return refused(None, "identity"),
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
            mux::new(TokioAsyncReadCompatExt::compat(tls), Mode::Client, streams.saturating_add(1));
        let connected = AtomicBool::new(false);
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
                    (&client, &connected),
                )
                .await;
            if let Err(reason) = ended {
                // Best effort: the agent learns why before the tunnel drops.
                let close = Kind::Close(Close {
                    reason: reason.to_string(),
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
            return refused(Some(&identity), reason);
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
        (client, connected): (&str, &AtomicBool),
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
        } = &info;
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
            protocol: agent_proto::PROTOCOL_VERSION,
        });
        control::write(control, hello)
            .await
            .map_err(|_| "control")?;
        guard.describe(version, ca_fingerprint);
        info!(cluster = %identity.cluster(), %version, streams, "agent tunnel open");
        connected.store(true, Ordering::Release);
        AuditEvent::new(Event::AgentConnected, Outcome::Allowed)
            .client_ip(client)
            .cluster(&identity.namespace, &identity.name)
            .agent(identity.epoch, Some(version))
            .emit();
        let mut closed = guard.closed.clone();
        tokio::select! {
            ended = self.control_loop(identity, control) => ended,
            _ = closed.wait_for(|closed| *closed) => Err("revoked"),
            // An agent opens no data stream: each is reset on arrival.
            () = async { while inbound.recv().await.is_some() {} } => Err("tunnel closed"),
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
                    still_valid(&self.state, identity).await?;
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
        .get_object_from_redis(crd::REDIS_PREFIX, &identity.cluster())
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

/// The tunnel TLS: a fresh server certificate under the agent CA, and a
/// client certificate required from that CA alone.
fn server_config(ca: &AgentCa) -> Result<ServerConfig, String> {
    let (cert_pem, key_pem) = ca.issue_server().map_err(|err| err.to_string())?;
    let mut roots = RootCertStore::empty();
    roots
        .add(
            CertificateDer::from_pem_slice(ca.cert_pem().as_bytes())
                .map_err(|err| err.to_string())?,
        )
        .map_err(|err| err.to_string())?;
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .map_err(|err| err.to_string())?;
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
    // `WsIo` writes frames of `MAX_FRAME` at most: the agent sends no larger.
    upgrade
        .max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |socket| gateway.serve(socket, client))
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
