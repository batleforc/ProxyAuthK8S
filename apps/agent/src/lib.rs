//! The agent of RFC 0012 (§6.5): `ServiceAccountToken` bootstrap, both TLS
//! modes, egress direct or through a `CONNECT` or SOCKS5 proxy.
//!
//! The agent enrols with a projected ServiceAccount token, dials the proxy's
//! agent gateway over a WebSocket, and runs yamux on a mutual TLS under the
//! agent CA: inside the WebSocket (`tlsMode: inner`), or the WebSocket's own
//! TLS, its certificate presented to the gateway (`passthrough`). It opens the control stream, then relays
//! every stream the gateway opens to its one `upstream`, the local apiserver,
//! without reading a byte: the TLS to the apiserver is the proxy's.

pub mod jwks;
mod store;

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_pki::{AgentKey, TUNNEL_SERVER_NAME, external_identity_of, identity_of};
use agent_proto::control::{self, AgentInfo, Kind, Ping, RenewRequest};
use agent_proto::mux::{self, Mode};
use agent_proto::ws_io::WsIo;
use base64::Engine as _;
use futures_util::io::{AsyncRead as FuturesRead, AsyncReadExt as _, AsyncWrite as FuturesWrite};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::connect::proxy::{SocksV5, Tunnel};
use reqwest::Url;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};
use tower_service::Service as _;
use tracing::{debug, info, warn};

/// The bound `jwks_from` applies to a JWKS (RFC 0012 §4.3).
const MAX_JWKS_BYTES: usize = 64 * 1024;
/// Bound on a TCP connect, a TLS handshake, the `Hello`.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// The gateway closes a control channel silent for 90 s.
const PING_EVERY: Duration = Duration::from_secs(30);
/// The yamux ceiling of this end: a gateway's `maxStreamsPerTunnel` up to
/// 1024, plus the control stream. The gateway's own bound is the real one.
const MAX_STREAMS: usize = 1025;
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
    /// The enrolment proof: the projected ServiceAccount token (`aud` = the
    /// CR's audience), or the `OneShotToken` from `tokenSecret`.
    pub token_path: PathBuf,
    /// `AGENT_BOOTSTRAP_METHOD=OneShotToken`: the identity Secret the
    /// replicas share; `None` (`ServiceAccountToken`): memory only.
    pub store: Option<store::IdentityStore>,
    /// `AGENT_BOOTSTRAP_METHOD=ExternalCA`: no enrolment, the certificate an
    /// issuer of the operator's keeps in files.
    pub external: Option<ExternalCert>,
    /// The local cluster's CA, whose SHA-256 the agent reports.
    pub kube_ca_path: Option<PathBuf>,
    /// How the agent reaches the proxy (§5.4).
    pub egress: Egress,
    /// `AGENT_TLS_MODE`: the gateway's `tlsMode` (§5.3).
    pub tls_mode: TlsMode,
}

/// `ExternalCA` (decision 28): the certificate (URI SAN
/// `proxyauth://<ns>/<name>`, then its chain) and key an issuer such as
/// cert-manager writes, read again at each connection; with `tlsMode:
/// inner`, the proxy's agent CA, which anchors the tunnel TLS.
#[derive(Clone, Debug)]
pub struct ExternalCert {
    cert_path: PathBuf,
    key_path: PathBuf,
    tunnel_ca_path: Option<PathBuf>,
}

impl ExternalCert {
    /// The certificate as the files hold it now.
    fn load(&self) -> Result<Identity, Reason> {
        let read = |path: &PathBuf| {
            std::fs::read_to_string(path).map_err(|err| {
                warn!(error = %err, path = %path.display(), "external certificate unreadable");
                "enroll"
            })
        };
        let cert_pem = read(&self.cert_path)?;
        Ok(Identity {
            not_after: Identity::not_after_of(&cert_pem)?,
            key_pem: read(&self.key_path)?,
            key: None,
            cert_pem,
            ca_pem: self
                .tunnel_ca_path
                .as_ref()
                .map(read)
                .transpose()?
                .unwrap_or_default(),
        })
    }
}

/// Where the agent's certificate is presented (§5.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TlsMode {
    /// In a mutual TLS inside the WebSocket, under the agent CA.
    #[default]
    Inner,
    /// In the TLS of the WebSocket itself, which the gateway ends.
    Passthrough,
}

/// How the agent opens its TCP to the proxy, the enrolment's and the
/// tunnel's alike (§5.4).
#[derive(Clone, Default, PartialEq, Eq)]
pub enum Egress {
    #[default]
    Direct,
    /// `http://` (`CONNECT`), `socks5h://` (DNS by the proxy) or `socks5://`,
    /// its credentials apart from the URL.
    Proxy {
        url: Url,
        credentials: Option<(String, String)>,
    },
}

/// Never prints the credentials.
impl fmt::Debug for Egress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct => f.write_str("Direct"),
            Self::Proxy { url, credentials } => f
                .debug_struct("Proxy")
                .field("url", &url.as_str())
                .field("credentials", &credentials.is_some())
                .finish(),
        }
    }
}

impl Egress {
    /// `AGENT_EGRESS_PROXY_URL` (the chart's `egress.proxy.url`), else
    /// `HTTPS_PROXY` or `ALL_PROXY` unless `NO_PROXY` names `host`; neither:
    /// direct. `AGENT_EGRESS_PROXY_USERNAME` / `_PASSWORD` (the chart's
    /// `credentialsSecret`) win over the URL's own.
    fn from_lookup(var: impl Fn(&str) -> Option<String>, host: &str) -> Result<Self, String> {
        let url = match var("AGENT_EGRESS_PROXY_URL") {
            Some(url) => url,
            None => {
                let no_proxy = var("NO_PROXY").or_else(|| var("no_proxy"));
                if no_proxy.is_some_and(|no_proxy| bypasses(&no_proxy, host)) {
                    return Ok(Self::Direct);
                }
                match ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
                    .into_iter()
                    .find_map(&var)
                {
                    Some(url) => url,
                    None => return Ok(Self::Direct),
                }
            }
        };
        let mut url: Url = url
            .parse()
            .map_err(|err| format!("egress proxy URL: {err}"))?;
        if !matches!(url.scheme(), "http" | "socks5" | "socks5h") {
            return Err("the egress proxy must be http://, socks5h:// or socks5://".to_string());
        }
        if url.host_str().is_none() {
            return Err("the egress proxy URL has no host".to_string());
        }
        let decode = |part: &str| {
            percent_encoding::percent_decode_str(part)
                .decode_utf8_lossy()
                .into_owned()
        };
        let credentials = match var("AGENT_EGRESS_PROXY_USERNAME") {
            Some(user) => Some((user, var("AGENT_EGRESS_PROXY_PASSWORD").unwrap_or_default())),
            None if !url.username().is_empty() => Some((
                decode(url.username()),
                decode(url.password().unwrap_or_default()),
            )),
            None => None,
        };
        // Both only fail on a URL without a host, refused above.
        url.set_username("").ok();
        url.set_password(None).ok();
        Ok(Self::Proxy { url, credentials })
    }

    /// The enrolment client's proxy: the same as the tunnel's.
    fn apply(&self, builder: reqwest::ClientBuilder) -> Result<reqwest::ClientBuilder, String> {
        match self {
            Self::Direct => Ok(builder.no_proxy()),
            Self::Proxy { url, credentials } => {
                let mut proxy = reqwest::Proxy::all(url.as_str()).map_err(|err| err.to_string())?;
                if let Some((user, password)) = credentials {
                    proxy = proxy.basic_auth(user, password);
                }
                Ok(builder.proxy(proxy))
            }
        }
    }
}

/// Whether `NO_PROXY` names `host`: `*`, the host itself or a parent domain
/// (`example.com` and `.example.com` both match `a.example.com`).
// shortcut: no CIDR ranges nor ports, add them when an operator needs one.
fn bypasses(no_proxy: &str, host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    no_proxy
        .split(',')
        .map(|entry| entry.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            entry == "*"
                || host == entry
                || host
                    .strip_suffix(entry.as_str())
                    .is_some_and(|rest| rest.ends_with('.'))
        })
}

/// A TCP connection to `host:port`, direct or through the egress proxy;
/// the TLS that follows goes through it untouched.
async fn dial(egress: &Egress, host: &str, port: u16) -> Result<TcpStream, Reason> {
    let Egress::Proxy { url, credentials } = egress else {
        return tokio::time::timeout(STEP_TIMEOUT, TcpStream::connect((host, port)))
            .await
            .map_err(|_| "connect")?
            .map_err(|_| "connect");
    };
    let proxy: http::Uri = url.as_str().parse().map_err(|_| "connect")?;
    let target: http::Uri = format!("https://{host}:{port}")
        .parse()
        .map_err(|_| "connect")?;
    let mut connector = HttpConnector::new();
    // The proxy's own scheme is `socks5h`, which the connector refuses otherwise.
    connector.enforce_http(false);
    let refused = |err: &dyn std::error::Error| {
        warn!(error = %err, proxy = %url, "egress proxy refused the tunnel");
        "connect"
    };
    let io = if url.scheme() == "http" {
        let mut tunnel = Tunnel::new(proxy, connector);
        if let Some((user, password)) = credentials {
            let basic =
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
            let mut auth =
                http::HeaderValue::from_str(&format!("Basic {basic}")).map_err(|_| "connect")?;
            auth.set_sensitive(true);
            tunnel = tunnel.with_auth(auth);
        }
        tokio::time::timeout(STEP_TIMEOUT, tunnel.call(target))
            .await
            .map_err(|_| "connect")?
            .map_err(|err| refused(&err))?
    } else {
        let mut socks = SocksV5::new(proxy, connector).local_dns(url.scheme() == "socks5");
        if let Some((user, password)) = credentials {
            socks = socks.with_auth(user.clone(), password.clone());
        }
        tokio::time::timeout(STEP_TIMEOUT, socks.call(target))
            .await
            .map_err(|_| "connect")?
            .map_err(|err| refused(&err))?
    };
    Ok(io.into_inner())
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
        let tls_mode = match var("AGENT_TLS_MODE").as_deref() {
            None | Some("inner") => TlsMode::Inner,
            Some("passthrough") if proxy_url.scheme() == "wss" => TlsMode::Passthrough,
            Some("passthrough") => {
                return Err("AGENT_TLS_MODE=passthrough needs a wss:// AGENT_PROXY_URL".to_string());
            }
            Some(other) => return Err(format!("AGENT_TLS_MODE `{other}`: inner or passthrough")),
        };
        let (mut store, mut external) = (None, None);
        match var("AGENT_BOOTSTRAP_METHOD").as_deref() {
            None | Some("ServiceAccountToken") => {}
            Some("OneShotToken") => store = Some(store::IdentityStore::from_lookup(var)?),
            Some("ExternalCA") => {
                let tunnel_ca_path = var("AGENT_TUNNEL_CA_PATH").map(PathBuf::from);
                if tls_mode == TlsMode::Inner && tunnel_ca_path.is_none() {
                    return Err(
                        "ExternalCA with tlsMode inner needs AGENT_TUNNEL_CA_PATH (the proxy's agent CA)"
                            .to_string(),
                    );
                }
                external = Some(ExternalCert {
                    cert_path: var("AGENT_TLS_CERT_PATH")
                        .unwrap_or_else(|| "/var/run/secrets/proxyauth-cert/tls.crt".to_string())
                        .into(),
                    key_path: var("AGENT_TLS_KEY_PATH")
                        .unwrap_or_else(|| "/var/run/secrets/proxyauth-cert/tls.key".to_string())
                        .into(),
                    tunnel_ca_path,
                });
            }
            Some(other) => {
                return Err(format!(
                    "AGENT_BOOTSTRAP_METHOD `{other}`: ServiceAccountToken, OneShotToken or ExternalCA"
                ));
            }
        }
        let egress = Egress::from_lookup(var, proxy_url.host_str().unwrap_or_default())?;
        Ok(Self {
            proxy_url,
            cluster,
            ca_bundle,
            upstream,
            token_path: var("AGENT_TOKEN_PATH")
                .unwrap_or_else(|| "/var/run/secrets/proxyauth/token".to_string())
                .into(),
            store,
            external,
            kube_ca_path: var("AGENT_KUBE_CA_PATH").map(PathBuf::from),
            egress,
            tls_mode,
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

/// The agent's identity: its key, the certificate the gateway (or, with
/// `ExternalCA`, the operator's issuer) issued for it, and the agent CA that
/// anchors the tunnel TLS. In memory, and in the identity Secret with
/// `OneShotToken`.
struct Identity {
    /// The key, PEM: any encoding an external issuer writes.
    key_pem: String,
    /// The same key, for the CSRs of a renewal; `None` with `ExternalCA`.
    key: Option<AgentKey>,
    /// The certificate, then its chain.
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

    /// Still good for a tunnel, a renewal due or not: a `OneShotToken` agent
    /// renews over its tunnel rather than spend a token.
    fn usable(&self) -> bool {
        self.not_after > unix_now().saturating_add(60)
    }

    /// This agent's certificate and key, as a TLS client presents them.
    fn client_auth(
        &self,
    ) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), Reason> {
        Ok((
            CertificateDer::pem_slice_iter(self.cert_pem.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "tls")?,
            PrivateKeyDer::from_pem_slice(self.key_pem.as_bytes()).map_err(|_| "tls")?,
        ))
    }

    /// The `notAfter` of a certificate of the agent CA, or of an external
    /// issuer (`proxyauth://<ns>/<name>`).
    fn not_after_of(cert_pem: &str) -> Result<i64, Reason> {
        let der = CertificateDer::from_pem_slice(cert_pem.as_bytes()).map_err(|_| "enroll")?;
        match identity_of(&der) {
            Ok(peer) => Ok(peer.not_after.unix_timestamp()),
            Err(_) => Ok(external_identity_of(&der)
                .map_err(|_| "enroll")?
                .not_after
                .unix_timestamp()),
        }
    }
}

#[derive(serde::Deserialize)]
struct Enrolled {
    cert: String,
    ca: String,
}

/// The TLS of the proxy's URL: `ca_bundle`, or the system roots; with
/// `identity` (`passthrough`), its certificate presented to the gateway.
fn outer_tls(config: &Config, identity: Option<&Identity>) -> Result<ClientConfig, String> {
    let builder = ClientConfig::builder();
    let builder = match &config.ca_bundle {
        Some(pem) => {
            let mut roots = RootCertStore::empty();
            for cert in CertificateDer::pem_slice_iter(pem.as_bytes()) {
                roots
                    .add(cert.map_err(|err| err.to_string())?)
                    .map_err(|err| err.to_string())?;
            }
            builder.with_root_certificates(roots)
        }
        None => {
            use rustls_platform_verifier::BuilderVerifierExt as _;
            builder
                .with_platform_verifier()
                .map_err(|err| err.to_string())?
        }
    };
    match identity {
        None => Ok(builder.with_no_client_auth()),
        Some(identity) => {
            let (certs, key) = identity.client_auth().map_err(str::to_string)?;
            builder
                .with_client_auth_cert(certs, key)
                .map_err(|err| err.to_string())
        }
    }
}

/// The agent: enrols, holds one tunnel, reconnects with a backoff from 1 to
/// 60 s. Never returns.
pub async fn run(config: Config, health: Arc<Health>) {
    let outer = match outer_tls(&config, None) {
        Ok(outer) => Arc::new(outer),
        Err(err) => {
            warn!(error = %err, "the proxy's CA bundle is unusable");
            return std::future::pending().await;
        }
    };
    let http = config
        .egress
        .apply(reqwest::Client::builder())
        .and_then(|builder| {
            builder
                .use_preconfigured_tls((*outer).clone())
                .timeout(STEP_TIMEOUT)
                .build()
                .map_err(|err| err.to_string())
        });
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
            .is_none_or(|current| match config.store {
                Some(_) => !current.usable(),
                None => current.renew_in().is_zero(),
            });
        // An external certificate is read again at each connection.
        let ended = if expired || config.external.is_some() {
            let obtained = match (&config.store, &config.external) {
                (_, Some(external)) => external.load(),
                (Some(store), None) => obtain(&config, &http, store).await,
                (None, None) => enroll(&config, &http).await,
            };
            match obtained {
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
            // No replica may reuse it: the next one needs a new token.
            if let Some(store) = &config.store {
                let cleared = match store.load().await {
                    Ok((secret, _)) => store.save(secret, serde_json::json!({})).await,
                    Err(reason) => Err(reason),
                };
                if !matches!(cleared, Ok(true)) {
                    warn!("the revoked identity is still in the identity Secret");
                }
            }
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

/// `OneShotToken`: the identity the Secret holds, else a new one, written
/// there unless another replica was first; then the winner's.
async fn obtain(
    config: &Config,
    http: &reqwest::Client,
    store: &store::IdentityStore,
) -> Result<Identity, Reason> {
    let (secret, stored) = store.load().await?;
    if let Some(stored) = stored.filter(Identity::usable) {
        info!("agent identity read from its Secret");
        return Ok(stored);
    }
    let enrolled = match enroll(config, http).await {
        Ok(enrolled) => enrolled,
        // Spent by a replica that may have written the Secret since.
        Err(reason) => return store.load().await?.1.filter(Identity::usable).ok_or(reason),
    };
    if store.save(secret, enrolled.to_data()).await? {
        return Ok(enrolled);
    }
    store
        .load()
        .await?
        .1
        .filter(Identity::usable)
        .ok_or("enroll")
}

async fn enroll(config: &Config, http: &reqwest::Client) -> Result<Identity, Reason> {
    let token = tokio::fs::read_to_string(&config.token_path)
        .await
        .map_err(|err| {
            warn!(error = %err, path = %config.token_path.display(), "no enrolment token");
            "enroll"
        })?;
    let proof = if config.store.is_some() {
        "one_shot_token"
    } else {
        "service_account_token"
    };
    let key = AgentKey::generate().map_err(|_| "enroll")?;
    let csr = key.csr_pem().map_err(|_| "enroll")?;
    let response = http
        .post(config.enroll_url()?)
        .json(&serde_json::json!({
            "cluster": config.cluster,
            "csr": csr,
            "proof": { proof: token.trim() },
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
        key_pem: key.key_pem(),
        key: Some(key),
        cert_pem: enrolled.cert,
        ca_pem: enrolled.ca,
    })
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// What yamux runs on: the inner TLS, or the WebSocket itself.
trait MuxIo: FuturesRead + FuturesWrite + Unpin + Send {}
impl<T: FuturesRead + FuturesWrite + Unpin + Send> MuxIo for T {}

/// The inner TLS (`tlsMode: inner`): this agent's certificate, the gateway's
/// under the agent CA alone.
fn inner_tls(identity: &Identity) -> Result<ClientConfig, Reason> {
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(identity.ca_pem.as_bytes()).map_err(|_| "tls")?)
        .map_err(|_| "tls")?;
    let (certs, key) = identity.client_auth()?;
    ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(certs, key)
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
    let tcp = dial(&config.egress, host, port).await?;
    let stream: Box<dyn Io> = if url.scheme() == "wss" {
        let name = ServerName::try_from(host.to_string()).map_err(|_| "connect")?;
        let outer = match config.tls_mode {
            TlsMode::Inner => Arc::clone(outer),
            TlsMode::Passthrough => {
                let identity = identity.lock().unwrap_or_else(PoisonError::into_inner);
                Arc::new(
                    outer_tls(config, Some(identity.as_ref().ok_or("enroll")?)).map_err(|err| {
                        warn!(error = %err, "no client TLS for the gateway");
                        "tls"
                    })?,
                )
            }
        };
        Box::new(
            tokio::time::timeout(STEP_TIMEOUT, TlsConnector::from(outer).connect(name, tcp))
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
    let io: Box<dyn MuxIo> = match config.tls_mode {
        // The gateway checked this agent's certificate in the WebSocket's TLS.
        TlsMode::Passthrough => Box::new(WsIo::new(ws)),
        TlsMode::Inner => {
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
            Box::new(tls.compat())
        }
    };

    let (handle, mut inbound, driver) = mux::new(io, Mode::Server, MAX_STREAMS);
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
                        let data = identity
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .as_mut()
                            .map(|current| {
                                current.cert_pem = cert;
                                current.not_after = not_after;
                                current.to_data()
                            });
                        health.cert_not_after.store(not_after, Ordering::Relaxed);
                        info!("agent certificate renewed");
                        if let (Some(store), Some(data)) = (&config.store, data) {
                            share_renewal(store, data, not_after).await;
                        }
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
                        // Its issuer renews an external certificate: reconnect
                        // with the later one once the files hold it.
                        if let Some(external) = &config.external {
                            asked = Some(tokio::time::Instant::now());
                            let current = identity
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .as_ref()
                                .map_or(0, |current| current.not_after);
                            if external.load().is_ok_and(|fresh| fresh.not_after > current) {
                                info!("external certificate renewed: reconnecting");
                                return Ok("closed");
                            }
                            continue;
                        }
                        let csr = identity
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .as_ref()
                            .filter(|current| current.renew_in().is_zero())
                            .and_then(|current| current.key.as_ref()?.csr_pem().ok());
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

/// A renewed certificate goes to the identity Secret unless a replica wrote
/// a later one; losing that race is fine, the Secret holds a valid one.
async fn share_renewal(store: &store::IdentityStore, data: serde_json::Value, not_after: i64) {
    match store.load().await {
        Ok((secret, stored))
            if stored
                .as_ref()
                .is_none_or(|stored| stored.not_after < not_after) =>
        {
            if !matches!(store.save(secret, data).await, Ok(true)) {
                debug!("renewed certificate not shared: the Secret moved on");
            }
        }
        Ok(_) => {}
        Err(reason) => debug!(reason, "renewed certificate not shared"),
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
    fn passthrough_needs_a_wss_url() {
        let base = ("AGENT_CLUSTER", "team-a/prod");
        let wss = ("AGENT_PROXY_URL", "wss://p/agent/v1/connect");
        assert_eq!(config(&[base, wss]).unwrap().tls_mode, TlsMode::Inner);
        let mode = ("AGENT_TLS_MODE", "passthrough");
        assert_eq!(
            config(&[base, wss, mode]).unwrap().tls_mode,
            TlsMode::Passthrough
        );
        let ws = ("AGENT_PROXY_URL", "ws://127.0.0.1:8444/agent/v1/connect");
        assert!(config(&[base, ws, mode]).is_err());
        assert!(config(&[base, wss, ("AGENT_TLS_MODE", "outer")]).is_err());
    }

    #[test]
    fn an_external_ca_in_inner_mode_needs_the_tunnel_ca() {
        let base = [
            ("AGENT_CLUSTER", "team-a/prod"),
            ("AGENT_PROXY_URL", "wss://p/agent/v1/connect"),
            ("AGENT_BOOTSTRAP_METHOD", "ExternalCA"),
        ];
        assert!(config(&base).unwrap_err().contains("AGENT_TUNNEL_CA_PATH"));
        let mut passthrough = base.to_vec();
        passthrough.push(("AGENT_TLS_MODE", "passthrough"));
        assert!(config(&passthrough).unwrap().external.is_some());
        let mut inner = base.to_vec();
        inner.push(("AGENT_TUNNEL_CA_PATH", "/ca.crt"));
        assert!(config(&inner).unwrap().external.is_some());
    }

    #[test]
    fn a_revoking_close_drops_the_identity() {
        assert_eq!(close_reason("epoch"), "revoked");
        assert_eq!(close_reason("idle"), "closed");
    }

    fn egress(vars: &[(&str, &str)], host: &str) -> Result<Egress, String> {
        let vars: HashMap<&str, &str> = vars.iter().copied().collect();
        Egress::from_lookup(|name| vars.get(name).map(ToString::to_string), host)
    }

    #[test]
    fn the_egress_follows_the_chart_then_the_environment() {
        assert_eq!(egress(&[], "proxy.example.com").unwrap(), Egress::Direct);
        let Egress::Proxy { url, credentials } = egress(
            &[
                ("HTTPS_PROXY", "http://u%40x:p%3Ass@squid:3128"),
                ("ALL_PROXY", "socks5h://other:1080"),
            ],
            "proxy.example.com",
        )
        .unwrap() else {
            panic!("HTTPS_PROXY ignored")
        };
        assert_eq!(url.as_str(), "http://squid:3128/");
        assert_eq!(credentials, Some(("u@x".to_string(), "p:ss".to_string())));
        for no_proxy in ["*", "example.com", ".example.com", "a,proxy.example.com"] {
            assert_eq!(
                egress(
                    &[("HTTPS_PROXY", "http://squid:3128"), ("NO_PROXY", no_proxy)],
                    "proxy.example.com"
                )
                .unwrap(),
                Egress::Direct,
                "{no_proxy}"
            );
        }
        assert_ne!(
            egress(
                &[
                    ("HTTPS_PROXY", "http://squid:3128"),
                    ("NO_PROXY", "ample.com")
                ],
                "proxy.example.com"
            )
            .unwrap(),
            Egress::Direct
        );
        // The chart's proxy wins, NO_PROXY or not, and so do its credentials.
        let chart = egress(
            &[
                (
                    "AGENT_EGRESS_PROXY_URL",
                    "socks5h://user:url@microsocks:1080",
                ),
                ("AGENT_EGRESS_PROXY_USERNAME", "agent"),
                ("AGENT_EGRESS_PROXY_PASSWORD", "s3cret"),
                ("NO_PROXY", "*"),
            ],
            "proxy.example.com",
        )
        .unwrap();
        assert!(!format!("{chart:?}").contains("s3cret"), "{chart:?}");
        assert_eq!(
            chart,
            Egress::Proxy {
                url: "socks5h://microsocks:1080".parse().unwrap(),
                credentials: Some(("agent".to_string(), "s3cret".to_string())),
            }
        );
        for bad in ["https://squid:3128", "socks4://s:1080", "squid:3128"] {
            assert!(
                egress(&[("AGENT_EGRESS_PROXY_URL", bad)], "p").is_err(),
                "{bad}"
            );
        }
    }

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    /// One connection through `serve`, then an echo of what follows.
    async fn proxy_server<F, Fut>(serve: F) -> Url
    where
        F: FnOnce(TcpStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Option<TcpStream>> + Send,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            if let Some(mut tcp) = serve(tcp).await {
                let mut buf = [0u8; 64];
                let read = tcp.read(&mut buf).await.unwrap();
                tcp.write_all(&buf[..read]).await.unwrap();
            }
        });
        format!("http://{address}").parse().unwrap()
    }

    async fn echoes(egress: &Egress) -> Result<(), Reason> {
        let mut tcp = dial(egress, "agent.proxy.test", 443).await?;
        tcp.write_all(b"tls").await.unwrap();
        let mut buf = [0u8; 3];
        tcp.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"tls");
        Ok(())
    }

    /// An HTTP proxy asking `Basic agent:s3cret`, answering `407` otherwise.
    async fn connect_proxy() -> Url {
        proxy_server(|mut tcp| async move {
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(tcp.read_u8().await.ok()?);
            }
            let head = String::from_utf8(head).unwrap();
            assert!(
                head.starts_with("CONNECT agent.proxy.test:443 HTTP/1.1\r\n"),
                "{head}"
            );
            // base64("agent:s3cret")
            if head.contains("Proxy-Authorization: Basic YWdlbnQ6czNjcmV0\r\n") {
                tcp.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await
                    .ok()?;
                Some(tcp)
            } else {
                drop(
                    tcp.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                        .await,
                );
                None
            }
        })
        .await
    }

    #[tokio::test]
    async fn the_tunnel_goes_through_a_connect_proxy_with_its_credentials() {
        let credentials = |password: &str| Some(("agent".to_string(), password.to_string()));
        let url = connect_proxy().await;
        echoes(&Egress::Proxy {
            url,
            credentials: credentials("s3cret"),
        })
        .await
        .unwrap();
        for credentials in [credentials("wrong"), None] {
            let url = connect_proxy().await;
            assert_eq!(
                echoes(&Egress::Proxy { url, credentials }).await,
                Err("connect")
            );
        }
    }

    /// A SOCKS5 proxy (RFC 1928) asking `agent` / `s3cret` (RFC 1929), or
    /// nothing; the name must reach it unresolved (`socks5h`).
    async fn socks_proxy(auth: bool) -> Url {
        let url = proxy_server(move |mut tcp| async move {
            let mut greeting = [0u8; 2];
            tcp.read_exact(&mut greeting).await.ok()?;
            let mut methods = vec![0u8; usize::from(greeting[1])];
            tcp.read_exact(&mut methods).await.ok()?;
            let method = if auth { 2 } else { 0 };
            if !methods.contains(&method) {
                drop(tcp.write_all(&[5, 0xff]).await);
                return None;
            }
            tcp.write_all(&[5, method]).await.ok()?;
            if auth {
                assert_eq!(tcp.read_u8().await.ok()?, 1);
                let mut user = vec![0u8; usize::from(tcp.read_u8().await.ok()?)];
                tcp.read_exact(&mut user).await.ok()?;
                let mut password = vec![0u8; usize::from(tcp.read_u8().await.ok()?)];
                tcp.read_exact(&mut password).await.ok()?;
                let ok = user == b"agent" && password == b"s3cret";
                tcp.write_all(&[1, u8::from(!ok)]).await.ok()?;
                if !ok {
                    return None;
                }
            }
            let mut request = [0u8; 4];
            tcp.read_exact(&mut request).await.ok()?;
            assert_eq!(request, [5, 1, 0, 3], "CONNECT to a domain name");
            let mut name = vec![0u8; usize::from(tcp.read_u8().await.ok()?)];
            tcp.read_exact(&mut name).await.ok()?;
            assert_eq!(name, b"agent.proxy.test");
            assert_eq!(tcp.read_u16().await.ok()?, 443);
            tcp.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.ok()?;
            Some(tcp)
        })
        .await;
        // `set_scheme` refuses to leave a special scheme like `http`.
        url.as_str().replacen("http", "socks5h", 1).parse().unwrap()
    }

    #[tokio::test]
    async fn the_tunnel_goes_through_a_socks5_proxy_with_its_credentials() {
        let url = socks_proxy(false).await;
        echoes(&Egress::Proxy {
            url,
            credentials: None,
        })
        .await
        .unwrap();
        let credentials = |password: &str| Some(("agent".to_string(), password.to_string()));
        let url = socks_proxy(true).await;
        echoes(&Egress::Proxy {
            url,
            credentials: credentials("s3cret"),
        })
        .await
        .unwrap();
        let url = socks_proxy(true).await;
        assert_eq!(
            echoes(&Egress::Proxy {
                url,
                credentials: credentials("wrong")
            })
            .await,
            Err("connect")
        );
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
