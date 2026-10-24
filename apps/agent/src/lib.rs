//! The agent of RFC 0012 (§6.5), phase 1: `ServiceAccountToken` bootstrap,
//! `tlsMode: inner`, direct egress.
//!
//! The agent enrols with a projected ServiceAccount token, dials the proxy's
//! agent gateway over a WebSocket, runs a mutual TLS under the agent CA inside
//! it, and yamux over that TLS. It opens the control stream, then relays
//! every stream the gateway opens to its one `upstream`, the local apiserver,
//! without reading a byte: the TLS to the apiserver is the proxy's.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_pki::{AgentKey, TUNNEL_SERVER_NAME, identity_of};
use agent_proto::control::{self, AgentInfo, Kind, Ping, RenewRequest};
use agent_proto::mux::{self, Mode};
use agent_proto::ws_io::WsIo;
use futures_util::io::AsyncReadExt as _;
use reqwest::Url;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};
use tracing::{debug, info, warn};

/// Bound on a TCP connect, a TLS handshake, the `Hello`.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// The gateway closes a control channel silent for 90 s.
const PING_EVERY: Duration = Duration::from_secs(30);
/// The yamux ceiling of this end; the gateway's own bound is lower.
const MAX_STREAMS: usize = 512;
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// What the agent waits for a `RenewResponse` before asking again.
const RENEW_RETRY: Duration = Duration::from_secs(5 * 60);

/// Why a tunnel ended, for the logs and `proxyauth_agent_reconnects_total`.
pub type Reason = &'static str;
/// The reasons the metric counts, in a fixed order.
pub const REASONS: [Reason; 5] = ["enroll", "connect", "tls", "closed", "revoked"];

/// What the agent is told by its chart (§4.1).
#[derive(Clone, Debug)]
pub struct Config {
    /// `wss://<host>/agent/v1/connect` (`ws://` on a loopback address only).
    pub proxy_url: Url,
    /// `<namespace>/<name>` of the `ProxyKubeApi`.
    pub cluster: String,
    /// The CA of the proxy's URL, PEM; `None`: the system roots.
    pub ca_bundle: Option<String>,
    /// The one destination: `host:port` of the local apiserver.
    pub upstream: String,
    /// The projected ServiceAccount token (`aud` = the CR's audience).
    pub token_path: PathBuf,
    /// The local cluster's CA, whose SHA-256 the agent reports.
    pub kube_ca_path: Option<PathBuf>,
}

impl Config {
    /// From `AGENT_*` variables, read through `lookup`.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let var = |name: &str| lookup(name).filter(|value| !value.trim().is_empty());
        let proxy_url: Url = var("AGENT_PROXY_URL")
            .ok_or("AGENT_PROXY_URL is required")?
            .parse()
            .map_err(|err| format!("AGENT_PROXY_URL: {err}"))?;
        let loopback = proxy_url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        match proxy_url.scheme() {
            "wss" => {}
            // The enrolment proof would cross the network in clear.
            "ws" if loopback => {}
            _ => return Err("AGENT_PROXY_URL must be a wss:// URL".to_string()),
        }
        let cluster = var("AGENT_CLUSTER").ok_or("AGENT_CLUSTER is required (namespace/name)")?;
        if cluster.split('/').filter(|part| !part.is_empty()).count() != 2 {
            return Err("AGENT_CLUSTER must be namespace/name".to_string());
        }
        let ca_bundle = match var("AGENT_PROXY_CA_BUNDLE") {
            Some(path) => Some(
                std::fs::read_to_string(&path)
                    .map_err(|err| format!("AGENT_PROXY_CA_BUNDLE {path}: {err}"))?,
            ),
            None => None,
        };
        let upstream = var("AGENT_UPSTREAM")
            .unwrap_or_else(|| "https://kubernetes.default.svc:443".to_string());
        let upstream: Url = upstream
            .parse()
            .map_err(|err| format!("AGENT_UPSTREAM: {err}"))?;
        let upstream = format!(
            "{}:{}",
            upstream.host_str().ok_or("AGENT_UPSTREAM has no host")?,
            upstream
                .port_or_known_default()
                .ok_or("AGENT_UPSTREAM has no port")?
        );
        Ok(Self {
            proxy_url,
            cluster,
            ca_bundle,
            upstream,
            token_path: var("AGENT_TOKEN_PATH")
                .unwrap_or_else(|| "/var/run/secrets/proxyauth/token".to_string())
                .into(),
            kube_ca_path: var("AGENT_KUBE_CA_PATH").map(PathBuf::from),
        })
    }

    /// `POST /agent/v1/enroll`, on the host of `proxy_url`.
    fn enroll_url(&self) -> Result<Url, Reason> {
        let mut url = self.proxy_url.clone();
        let scheme = if url.scheme() == "wss" {
            "https"
        } else {
            "http"
        };
        url.set_scheme(scheme).map_err(|()| "enroll")?;
        url.set_path("/agent/v1/enroll");
        url.set_query(None);
        Ok(url)
    }
}

/// What the probes and `/metrics` read.
#[derive(Debug, Default)]
pub struct Health {
    pub tunnel_up: AtomicBool,
    /// Unix seconds of the certificate's `notAfter`; `0` without one.
    pub cert_not_after: AtomicI64,
    pub reconnects: [AtomicU64; REASONS.len()],
}

impl Health {
    fn reconnected(&self, reason: Reason) {
        if let Some(index) = REASONS.iter().position(|known| *known == reason)
            && let Some(counter) = self.reconnects.get(index)
        {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The Prometheus text exposition of the agent's series (§10.1).
    #[must_use]
    pub fn render(&self) -> String {
        let now = unix_now();
        let not_after = self.cert_not_after.load(Ordering::Relaxed);
        let expiry = if not_after == 0 {
            0
        } else {
            not_after.saturating_sub(now)
        };
        let mut text = format!(
            "# TYPE proxyauth_agent_tunnel_up gauge\nproxyauth_agent_tunnel_up {}\n\
             # TYPE proxyauth_agent_cert_expiry_seconds gauge\nproxyauth_agent_cert_expiry_seconds {expiry}\n\
             # TYPE proxyauth_agent_reconnects_total counter\n",
            u8::from(self.tunnel_up.load(Ordering::Relaxed)),
        );
        for (reason, counter) in REASONS.iter().zip(&self.reconnects) {
            text.push_str(&format!(
                "proxyauth_agent_reconnects_total{{reason=\"{reason}\"}} {}\n",
                counter.load(Ordering::Relaxed)
            ));
        }
        text
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

/// The agent's identity: its key, the certificate the gateway issued for it,
/// and the agent CA that anchors the tunnel TLS. Memory only.
struct Identity {
    key: AgentKey,
    cert_pem: String,
    ca_pem: String,
    /// Unix seconds.
    not_after: i64,
}

impl Identity {
    /// Renewed at two thirds of its lifetime (eight hours before the end of
    /// a 24 h certificate).
    fn renew_in(&self) -> Duration {
        let renew_at = self.not_after.saturating_sub(
            i64::try_from(agent_pki::AGENT_CERT_LIFETIME.as_secs() / 3).unwrap_or(0),
        );
        Duration::from_secs(u64::try_from(renew_at.saturating_sub(unix_now())).unwrap_or(0))
    }

    fn not_after_of(cert_pem: &str) -> Result<i64, Reason> {
        let der = CertificateDer::from_pem_slice(cert_pem.as_bytes()).map_err(|_| "enroll")?;
        let peer = identity_of(&der).map_err(|_| "enroll")?;
        Ok(peer.not_after.unix_timestamp())
    }
}

#[derive(serde::Deserialize)]
struct Enrolled {
    cert: String,
    ca: String,
}

/// The TLS of the proxy's URL: `ca_bundle`, or the system roots.
fn outer_tls(config: &Config) -> Result<ClientConfig, String> {
    let builder = ClientConfig::builder();
    match &config.ca_bundle {
        Some(pem) => {
            let mut roots = RootCertStore::empty();
            for cert in CertificateDer::pem_slice_iter(pem.as_bytes()) {
                roots
                    .add(cert.map_err(|err| err.to_string())?)
                    .map_err(|err| err.to_string())?;
            }
            Ok(builder.with_root_certificates(roots).with_no_client_auth())
        }
        None => {
            use rustls_platform_verifier::BuilderVerifierExt as _;
            Ok(builder
                .with_platform_verifier()
                .map_err(|err| err.to_string())?
                .with_no_client_auth())
        }
    }
}

/// The agent: enrols, holds one tunnel, reconnects with a backoff from 1 to
/// 60 s. Never returns.
pub async fn run(config: Config, health: Arc<Health>) {
    let outer = match outer_tls(&config) {
        Ok(outer) => Arc::new(outer),
        Err(err) => {
            warn!(error = %err, "the proxy's CA bundle is unusable");
            return std::future::pending().await;
        }
    };
    let http = reqwest::Client::builder()
        .use_preconfigured_tls((*outer).clone())
        .timeout(STEP_TIMEOUT)
        .build();
    let http = match http {
        Ok(http) => http,
        Err(err) => {
            warn!(error = %err, "no HTTP client for the enrolment");
            return std::future::pending().await;
        }
    };
    let identity: Mutex<Option<Identity>> = Mutex::new(None);
    let mut backoff = BACKOFF_MIN;
    loop {
        let expired = identity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_none_or(|current| current.renew_in().is_zero());
        let ended = if expired {
            match enroll(&config, &http).await {
                Ok(enrolled) => {
                    health
                        .cert_not_after
                        .store(enrolled.not_after, Ordering::Relaxed);
                    *identity.lock().unwrap_or_else(PoisonError::into_inner) = Some(enrolled);
                    tunnel(&config, &outer, &identity, &health).await
                }
                Err(reason) => Err(reason),
            }
        } else {
            tunnel(&config, &outer, &identity, &health).await
        };
        let was_up = health.tunnel_up.swap(false, Ordering::Relaxed);
        let reason = match ended {
            Ok(reason) | Err(reason) => reason,
        };
        if reason == "revoked" {
            *identity.lock().unwrap_or_else(PoisonError::into_inner) = None;
            health.cert_not_after.store(0, Ordering::Relaxed);
        }
        health.reconnected(reason);
        if was_up {
            backoff = BACKOFF_MIN;
        }
        warn!(reason, retry_in = ?backoff, "agent tunnel down");
        tokio::time::sleep(backoff).await;
        backoff = backoff.saturating_mul(2).min(BACKOFF_MAX);
    }
}

async fn enroll(config: &Config, http: &reqwest::Client) -> Result<Identity, Reason> {
    let token = tokio::fs::read_to_string(&config.token_path)
        .await
        .map_err(|err| {
            warn!(error = %err, path = %config.token_path.display(), "no ServiceAccount token");
            "enroll"
        })?;
    let key = AgentKey::generate().map_err(|_| "enroll")?;
    let csr = key.csr_pem().map_err(|_| "enroll")?;
    let response = http
        .post(config.enroll_url()?)
        .json(&serde_json::json!({
            "cluster": config.cluster,
            "csr": csr,
            "proof": { "service_account_token": token.trim() },
        }))
        .send()
        .await
        .map_err(|err| {
            warn!(error = %err, "enrolment request failed");
            "enroll"
        })?;
    if !response.status().is_success() {
        warn!(status = %response.status(), "enrolment refused");
        return Err("enroll");
    }
    let enrolled: Enrolled = response.json().await.map_err(|_| "enroll")?;
    info!(cluster = %config.cluster, "agent enrolled");
    Ok(Identity {
        not_after: Identity::not_after_of(&enrolled.cert)?,
        key,
        cert_pem: enrolled.cert,
        ca_pem: enrolled.ca,
    })
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// The inner TLS (`tlsMode: inner`): this agent's certificate, the gateway's
/// under the agent CA alone.
fn inner_tls(identity: &Identity) -> Result<ClientConfig, Reason> {
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(identity.ca_pem.as_bytes()).map_err(|_| "tls")?)
        .map_err(|_| "tls")?;
    ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(
            vec![CertificateDer::from_pem_slice(identity.cert_pem.as_bytes()).map_err(|_| "tls")?],
            PrivateKeyDer::from_pem_slice(identity.key.key_pem().as_bytes()).map_err(|_| "tls")?,
        )
        .map_err(|_| "tls")
}

/// One tunnel, until it ends. `Ok` when the gateway closed it, with why.
async fn tunnel(
    config: &Config,
    outer: &Arc<ClientConfig>,
    identity: &Mutex<Option<Identity>>,
    health: &Health,
) -> Result<Reason, Reason> {
    let url = &config.proxy_url;
    let host = url.host_str().ok_or("connect")?;
    let port = url.port_or_known_default().ok_or("connect")?;
    let tcp = tokio::time::timeout(STEP_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| "connect")?
        .map_err(|_| "connect")?;
    let stream: Box<dyn Io> = if url.scheme() == "wss" {
        let name = ServerName::try_from(host.to_string()).map_err(|_| "connect")?;
        Box::new(
            tokio::time::timeout(
                STEP_TIMEOUT,
                TlsConnector::from(Arc::clone(outer)).connect(name, tcp),
            )
            .await
            .map_err(|_| "connect")?
            .map_err(|_| "connect")?,
        )
    } else {
        Box::new(tcp)
    };
    let (ws, _) = tokio::time::timeout(
        STEP_TIMEOUT,
        tokio_tungstenite::client_async(url.as_str(), stream),
    )
    .await
    .map_err(|_| "connect")?
    .map_err(|err| {
        warn!(error = %err, "WebSocket handshake failed");
        "connect"
    })?;
    let inner = {
        let identity = identity.lock().unwrap_or_else(PoisonError::into_inner);
        inner_tls(identity.as_ref().ok_or("enroll")?)?
    };
    let server = ServerName::try_from(TUNNEL_SERVER_NAME).map_err(|_| "tls")?;
    let tls = tokio::time::timeout(
        STEP_TIMEOUT,
        TlsConnector::from(Arc::new(inner)).connect(server, WsIo::new(ws).compat()),
    )
    .await
    .map_err(|_| "tls")?
    .map_err(|err| {
        warn!(error = %err, "tunnel TLS failed");
        "tls"
    })?;

    let (handle, mut inbound, driver) = mux::new(tls.compat(), Mode::Server, MAX_STREAMS);
    let session = async {
        let mut control = handle.open().await.map_err(|_| "closed")?;
        // yamux announces the stream with its first frame: this one.
        let info = Kind::AgentInfo(AgentInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            ca_fingerprint: ca_fingerprint(config).await,
        });
        control::write(&mut control, info)
            .await
            .map_err(|_| "closed")?;
        match tokio::time::timeout(STEP_TIMEOUT, control::read(&mut control)).await {
            Ok(Ok(Kind::Hello(hello))) if hello.protocol == agent_proto::PROTOCOL_VERSION => {
                info!(gateway = %hello.replica_id, "agent tunnel open");
            }
            Ok(Ok(Kind::Close(close))) => return Ok(close_reason(&close.reason)),
            _ => return Err("closed"),
        }
        health.tunnel_up.store(true, Ordering::Relaxed);
        let (mut reader, mut writer) = control.split();
        let relay = async {
            while let Some(stream) = inbound.recv().await {
                let upstream = config.upstream.clone();
                tokio::spawn(async move {
                    let Ok(Ok(mut tcp)) =
                        tokio::time::timeout(STEP_TIMEOUT, TcpStream::connect(upstream.as_str()))
                            .await
                    else {
                        return warn!(%upstream, "upstream unreachable");
                    };
                    let mut stream = stream.compat();
                    if let Err(err) = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await {
                        debug!(error = %err, "stream relay ended");
                    }
                });
            }
        };
        let read = async {
            loop {
                match control::read(&mut reader).await.map_err(|_| "closed")? {
                    Kind::Pong(_) => {}
                    Kind::RenewResponse(renewed) => {
                        let cert = String::from_utf8(renewed.cert).map_err(|_| "closed")?;
                        let not_after = Identity::not_after_of(&cert)?;
                        if let Some(current) = identity
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .as_mut()
                        {
                            current.cert_pem = cert;
                            current.not_after = not_after;
                        }
                        health.cert_not_after.store(not_after, Ordering::Relaxed);
                        info!("agent certificate renewed");
                    }
                    Kind::Close(close) => return Ok::<_, Reason>(close_reason(&close.reason)),
                    _ => return Err("closed"),
                }
            }
        };
        let write = async {
            let mut pings = tokio::time::interval(PING_EVERY);
            let mut nonce = 0u64;
            let mut asked: Option<tokio::time::Instant> = None;
            loop {
                let renew_in = identity
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_ref()
                    .map_or(RENEW_RETRY, Identity::renew_in);
                // Due, and asked lately: wait out the retry, pings going on.
                let wait = match asked {
                    Some(at) if renew_in.is_zero() => RENEW_RETRY.saturating_sub(at.elapsed()),
                    _ => renew_in,
                };
                tokio::select! {
                    _ = pings.tick() => {
                        nonce = nonce.wrapping_add(1);
                        control::write(&mut writer, Kind::Ping(Ping { nonce }))
                            .await
                            .map_err(|_| "closed")?;
                    }
                    () = tokio::time::sleep(wait) => {
                        let csr = identity
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .as_ref()
                            .filter(|current| current.renew_in().is_zero())
                            .and_then(|current| current.key.csr_pem().ok());
                        if let Some(csr) = csr {
                            asked = Some(tokio::time::Instant::now());
                            control::write(
                                &mut writer,
                                Kind::RenewRequest(RenewRequest { csr: csr.into_bytes() }),
                            )
                            .await
                            .map_err(|_| "closed")?;
                        }
                    }
                }
            }
        };
        tokio::select! {
            ended = read => ended,
            ended = write => ended,
            () = relay => Err("closed"),
        }
    };
    tokio::select! {
        ended = session => ended,
        _ = driver => Err("closed"),
    }
}

/// A `Close` that revokes this identity makes the agent enrol again.
fn close_reason(reason: &str) -> Reason {
    match reason {
        "revoked" | "epoch" | "uid" | "unknown cluster" | "not an agent cluster" => "revoked",
        _ => "closed",
    }
}

/// SHA-256 of the local cluster's CA, hex; empty without one.
async fn ca_fingerprint(config: &Config) -> String {
    let Some(path) = &config.kube_ca_path else {
        return String::new();
    };
    let Ok(pem) = tokio::fs::read(path).await else {
        return String::new();
    };
    Sha256::digest(&pem)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(vars: &[(&str, &str)]) -> Result<Config, String> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        Config::from_lookup(|name| vars.get(name).cloned())
    }

    #[test]
    fn a_plain_ws_url_is_refused_unless_it_is_loopback() {
        let base = [("AGENT_CLUSTER", "team-a/prod")];
        for url in [
            "ws://proxy.example.com/agent/v1/connect",
            "https://proxy.example.com/",
        ] {
            let err = config(&[base[0], ("AGENT_PROXY_URL", url)]).unwrap_err();
            assert!(err.contains("wss://"), "{url}: {err}");
        }
        for url in [
            "wss://proxy.example.com/agent/v1/connect",
            "ws://127.0.0.1:8444/agent/v1/connect",
        ] {
            config(&[base[0], ("AGENT_PROXY_URL", url)]).unwrap();
        }
    }

    #[test]
    fn the_enrolment_url_and_the_upstream_follow_the_config() {
        let config = config(&[
            ("AGENT_CLUSTER", "team-a/prod"),
            (
                "AGENT_PROXY_URL",
                "wss://proxy.example.com/agent/v1/connect?x=1",
            ),
        ])
        .unwrap();
        assert_eq!(
            config.enroll_url().unwrap().as_str(),
            "https://proxy.example.com/agent/v1/enroll"
        );
        assert_eq!(config.upstream, "kubernetes.default.svc:443");
        let bad = super::Config::from_lookup(|name| match name {
            "AGENT_PROXY_URL" => Some("wss://p/agent/v1/connect".to_string()),
            "AGENT_CLUSTER" => Some("prod".to_string()),
            _ => None,
        });
        assert!(bad.is_err());
    }

    #[test]
    fn a_revoking_close_drops_the_identity() {
        assert_eq!(close_reason("epoch"), "revoked");
        assert_eq!(close_reason("idle"), "closed");
    }

    #[test]
    fn the_metrics_render_every_reason() {
        let health = Health::default();
        health.reconnected("tls");
        let text = health.render();
        assert!(text.contains("proxyauth_agent_tunnel_up 0"), "{text}");
        assert!(
            text.contains("proxyauth_agent_reconnects_total{reason=\"tls\"} 1"),
            "{text}"
        );
        assert!(
            text.contains("proxyauth_agent_cert_expiry_seconds 0"),
            "{text}"
        );
    }
}
