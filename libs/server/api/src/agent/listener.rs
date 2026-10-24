//! The gateway's agent listener (RFC 0012 §5.3, §6.3, `tlsMode: inner`).
//!
//! `GET /agent/v1/connect` upgrades to a WebSocket; inside it, the agent and
//! this pod run a mutual TLS under the agent CA, and yamux over that TLS. The
//! agent's identity is its certificate's, checked against the `ProxyKubeApi`
//! it names (UID, `epoch`, `Agent` service): no header set by an ingress is
//! read as identity. The first stream the agent opens is the control channel
//! (`AgentInfo` from the agent, then `Hello`); the agent opens no other.

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
use crd::service::Service;
use futures_util::{SinkExt, StreamExt, future};
use rustls::RootCertStore;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ServerConfig, WebPkiClientVerifier};
use tokio_rustls::TlsAcceptor;
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};
use tracing::{info, warn};

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
    pub(super) http: reqwest::Client,
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
            http: reqwest::Client::new(),
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

    async fn serve(self: Arc<Self>, socket: WebSocket) {
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
            Ok(Err(err)) => return warn!(error = %err, "agent tunnel refused: TLS"),
            Err(_) => return warn!("agent tunnel refused: TLS handshake timed out"),
        };
        let identity = match tls
            .get_ref()
            .1
            .peer_certificates()
            .and_then(<[_]>::first)
            .map(|cert| identity_of(cert))
        {
            Some(Ok(peer)) => peer.identity,
            _ => return warn!("agent tunnel refused: no agent identity"),
        };
        if let Err(reason) = still_valid(&self.state, &identity).await {
            return warn!(cluster = %identity.cluster(), reason, "agent tunnel refused");
        }

        let (handle, mut inbound, driver) = mux::new(
            TokioAsyncReadCompatExt::compat(tls),
            Mode::Client,
            self.settings.streams_per_tunnel,
        );
        let session = async {
            let mut control = tokio::time::timeout(self.settings.handshake_timeout, inbound.recv())
                .await
                .ok()
                .flatten()
                .ok_or("no control stream")?;
            let ended = self
                .session(&identity, &mut control, handle, &mut inbound)
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
        match ended {
            Ok(()) => info!(cluster = %identity.cluster(), "agent tunnel closed by the agent"),
            Err(reason) => info!(cluster = %identity.cluster(), reason, "agent tunnel closed"),
        }
    }

    async fn session(
        &self,
        identity: &AgentIdentity,
        control: &mut mux::Stream,
        handle: agent_proto::mux::MuxHandle,
        inbound: &mut tokio::sync::mpsc::Receiver<mux::Stream>,
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
        let hello = Kind::Hello(Hello {
            replica_id: self.pod.clone(),
            replicas_expected: 1,
            protocol: agent_proto::PROTOCOL_VERSION,
        });
        control::write(control, hello)
            .await
            .map_err(|_| "control")?;
        let AgentInfo { version, .. } = &info;
        let guard = self
            .registry
            .register(
                identity.clone(),
                handle,
                self.settings.tunnels_per_identity,
                self.settings.streams_per_tunnel,
            )
            .map_err(|_| "too many tunnels")?;
        info!(cluster = %identity.cluster(), %version, "agent tunnel open");
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
/// still an `Agent` service (T8).
pub async fn still_valid(state: &State, identity: &AgentIdentity) -> Result<(), Reason> {
    if current_epoch(state, identity).await? == identity.epoch {
        Ok(())
    } else {
        Err("epoch")
    }
}

/// The `epoch` of the `ProxyKubeApi` `identity` names, or why that CR no
/// longer accepts the identity's UID (`"redis"` when it cannot be read).
pub async fn current_epoch(state: &State, identity: &AgentIdentity) -> Result<u64, Reason> {
    let proxy: ProxyKubeApi = state
        .get_object_from_redis(crd::REDIS_PREFIX, &identity.cluster())
        .await
        .map_err(|_| "redis")?
        .ok_or("unknown cluster")?;
    if proxy.metadata.uid.as_deref() != Some(identity.uid.as_str()) {
        return Err("uid");
    }
    match &proxy.spec.service {
        Service::Agent(agent) => Ok(agent.epoch),
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
    upgrade: WebSocketUpgrade,
) -> Response {
    // `WsIo` writes frames of `MAX_FRAME` at most: the agent sends no larger.
    upgrade
        .max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |socket| gateway.serve(socket))
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
        .with_state(gateway)
}
