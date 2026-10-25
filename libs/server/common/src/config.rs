//! Every environment variable the server reads, parsed once into a typed
//! [`Config`].
//!
//! The groups mirror the configuration documentation page
//! (`docs/content/docs/configuration.mdx`): HTTP/TLS listener, OIDC, Redis,
//! leader election, proxy limits & security, observability.
//!
//! Parsing is deliberately lenient, exactly like the scattered readers it
//! replaces: an unset or unparsable value falls back to its default. The
//! difference is that every fallback caused by an *unparsable* value is now
//! recorded as a warning (see [`Config::warnings`]) and logged once when the
//! configuration is installed.
//!
//! # Process-wide access
//!
//! [`get`] returns the process-wide configuration. `main` loads it with
//! [`Config::from_env`] and installs it with [`init`] once tracing is up (so the
//! warnings reach the logs); any code path that never called [`init`] (tests,
//! tools) gets it loaded lazily from the environment on first [`get`].
//!
//! # Not read through this module
//!
//! - `PROXYAUTH_ALLOW_CROSS_NS_CERT` is mirrored in
//!   [`ProxySettings::allow_cross_namespace_cert`] for visibility, but the `crd`
//!   crate (which must not depend on `common`) still reads it itself, with the
//!   same parsing rule ([`crd::certificate::parse_allow_cross_namespace_cert`]).
//! - `RUST_LOG` is read by `tracing_subscriber::EnvFilter` in the `trace` crate
//!   (it drives both stdout and the OTLP export; default `info`), and
//!   `OTEL_EXPORTER_OTLP_*` by the OpenTelemetry SDK itself.

use std::{fmt, sync::OnceLock, time::Duration};

use tracing::warn;

use crate::{oidc_cache::DEFAULT_TOKEN_TTL, token_audience::AudienceValidationMode};

/// Default `SERVER_PORT`.
pub const DEFAULT_SERVER_PORT: u16 = 5437;
/// Default `SERVER_SHUTDOWN_TIMEOUT`, in seconds: long enough for in-flight
/// requests and most `watch`/`exec`/`port-forward` streams to drain on a rollout.
pub const DEFAULT_SHUTDOWN_TIMEOUT_SECS: u64 = 30;
/// Default `REDIS_URL` (local development).
pub const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";
/// Default `PROXY_DEBUG_BODY_MAX_BYTES`: the most a single request may buffer
/// for debug logging.
pub const DEFAULT_DEBUG_BODY_MAX_BYTES: usize = 10 * 1024 * 1024;
/// Default `PROXY_STREAM_CHANNEL_CAPACITY`: in-flight chunks between the client
/// payload reader and the upstream request body.
pub const DEFAULT_STREAM_CHANNEL_CAPACITY: usize = 32;
/// Default `PROXY_VIRTUAL_MAX_BODY_BYTES`: upper bound on a buffered virtual
/// API response.
pub const DEFAULT_VIRTUAL_MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
/// Default `PROXY_JWKS_MAX_DOCUMENT_BYTES`: upper bound on a fetched issuer
/// discovery document or JWKS.
pub const DEFAULT_JWKS_MAX_DOCUMENT_BYTES: usize = jwt_validator::MAX_DOCUMENT_BYTES;
/// Default `PROXY_UPSTREAM_CLIENT_TTL`: how long a cluster's upstream HTTP
/// client and TLS configuration are reused before the TLS material (CA, mTLS
/// client certificate) is resolved again.
pub const DEFAULT_UPSTREAM_CLIENT_TTL: Duration = Duration::from_secs(300);

/// Default `AGENT_GATEWAY_PORT`.
pub const DEFAULT_AGENT_GATEWAY_PORT: u16 = 8444;
/// Default `AGENT_GATEWAY_CONNECT_PORT`.
pub const DEFAULT_AGENT_GATEWAY_CONNECT_PORT: u16 = 8445;

/// The whole server configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerSettings,
    pub oidc: OidcSettings,
    pub redis: RedisSettings,
    pub leader_election: LeaderElectionSettings,
    pub proxy: ProxySettings,
    pub observability: ObservabilitySettings,
    pub agent_gateway: AgentGatewaySettings,
    /// Fallbacks taken while loading (unset values with a local-dev default
    /// that production must override, and unparsable values).
    warnings: Vec<String>,
}

/// HTTP(S) listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSettings {
    /// `SERVER_PORT` (default `5437`).
    pub port: u16,
    /// `SERVER_HTTPS` (default `false`): terminate TLS in the server.
    pub https: bool,
    /// `SERVER_CERT_PATH`: PEM certificate chain, required when `https`.
    pub cert_path: Option<String>,
    /// `SERVER_KEY_PATH`: PEM private key, required when `https`.
    pub key_path: Option<String>,
    /// `SERVER_SHUTDOWN_TIMEOUT` (default `30`): graceful-shutdown timeout in seconds.
    pub shutdown_timeout_secs: u64,
    /// `CORS_ALLOWED_ORIGINS`: which browser origins may call the API.
    pub cors_allowed_origins: CorsOrigins,
}

/// Parsed `CORS_ALLOWED_ORIGINS`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CorsOrigins {
    /// Unset or empty (the default): no cross-origin request is allowed.
    #[default]
    SameOriginOnly,
    /// `*`: any origin, the pre-0.2 default. Explicit opt-in only.
    Any,
    /// A comma-separated allow-list of exact origins.
    List(Vec<String>),
}

impl CorsOrigins {
    /// Parse the raw variable: `*` alone means any origin, anything else is an
    /// allow-list. Blank entries are dropped, and so is a `*` mixed with real
    /// origins: it must not widen the list, and actix-cors refuses to start
    /// with a wildcard in an allow-list. Nothing left means same-origin only.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        let origins: Vec<String> = value
            .split(',')
            .map(|origin| origin.trim().to_string())
            .filter(|origin| !origin.is_empty())
            .collect();
        if matches!(origins.as_slice(), [only] if only == "*") {
            return Self::Any;
        }
        let origins: Vec<String> = origins.into_iter().filter(|origin| origin != "*").collect();
        if origins.is_empty() {
            Self::SameOriginOnly
        } else {
            Self::List(origins)
        }
    }
}

/// Service-wide OIDC client (dashboard and API).
#[derive(Clone, PartialEq, Eq)]
pub struct OidcSettings {
    /// `OIDC_CLIENT_ID` (default `proxy-auth-k8s`).
    pub client_id: String,
    /// `OIDC_CLIENT_SECRET` (absent = public client). Redacted from `Debug`.
    pub client_secret: Option<String>,
    /// `OIDC_ISSUER_URL` (default `https://authelia.k8s.localhost`).
    pub issuer_url: String,
    /// `OIDC_SCOPES` (default `openid email profile`).
    pub scopes: String,
    /// `OIDC_AUDIENCE` (default `proxy-auth-k8s`).
    pub audience: String,
    /// `OIDC_REDIRECT_URL`.
    pub redirect_url: Option<String>,
    /// `OIDC_AUDIENCE_VALIDATION` (default `enforce`; unknown values enforce).
    pub audience_validation: AudienceValidationMode,
    /// `OIDC_TOKEN_CACHE_TTL` in seconds (default `30`, `0` disables).
    pub token_cache_ttl: Duration,
    /// `API_CLUSTER_OIDC_BASE_REDIRECT_URL` (default `https://localhost:5437`).
    pub cluster_redirect_base_url: String,
    /// `API_CLUSTER_OIDC_FRONT_REDIRECT_URL` (default
    /// `https://localhost:4200/auth/callback/`).
    pub front_redirect_base_url: String,
}

impl fmt::Debug for OidcSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OidcSettings")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "***REDACTED***"),
            )
            .field("issuer_url", &self.issuer_url)
            .field("scopes", &self.scopes)
            .field("audience", &self.audience)
            .field("redirect_url", &self.redirect_url)
            .field("audience_validation", &self.audience_validation)
            .field("token_cache_ttl", &self.token_cache_ttl)
            .field("cluster_redirect_base_url", &self.cluster_redirect_base_url)
            .field("front_redirect_base_url", &self.front_redirect_base_url)
            .finish()
    }
}

/// Redis connectivity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedisSettings {
    /// `REDIS_URL` (default `redis://127.0.0.1:6379`, warned); several
    /// comma-separated URLs select cluster mode.
    pub url: String,
    /// `REDIS_CLUSTER` (`true`/`1`): force cluster mode with a single URL.
    pub cluster: bool,
}

/// Controller leader election.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderElectionSettings {
    /// `LEASE_NAMESPACE` (default `default`, warned).
    pub lease_namespace: String,
    /// `HOSTNAME` (default `NOT_A_POD`): this replica's identity in the `Lease`.
    pub lease_name: String,
}

/// Proxy limits and security switches.
///
/// The `api` redirect handlers read these through [`get`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxySettings {
    /// `TRUSTED_PROXY_COUNT` (default `0`): trusted reverse-proxy hops in front
    /// of the service, used to pick the client address out of `X-Forwarded-For`.
    pub trusted_proxy_count: usize,
    /// `PROXY_DEBUG_BODY_MAX_BYTES` (default 10 MiB, must be > 0).
    pub debug_body_max_bytes: usize,
    /// `PROXY_STREAM_CHANNEL_CAPACITY` (default `32`, must be > 0).
    pub stream_channel_capacity: usize,
    /// `PROXY_VIRTUAL_MAX_BODY_BYTES` (default 32 MiB, must be > 0).
    pub virtual_max_body_bytes: usize,
    /// `PROXY_JWKS_MAX_DOCUMENT_BYTES` (default 64 KiB, must be > 0): bound on
    /// an issuer's discovery document or JWKS, `jwks_url` of agents included.
    pub jwks_max_document_bytes: usize,
    /// `PROXY_UPSTREAM_CLIENT_TTL` in seconds (default `300`, `0` disables):
    /// lifetime of a cached per-cluster upstream client / TLS configuration
    /// (see [`crate::upstream_cache`]). Bounds how long a rotated
    /// Secret/ConfigMap certificate can go unnoticed.
    pub upstream_client_ttl: Duration,
    /// `THROTTLE_FAIL_CLOSED` (default `false`): deny requests when Redis fails.
    pub throttle_fail_closed: bool,
    /// `PROXYAUTH_ALLOW_CROSS_NS_CERT` (default `false`).
    ///
    /// Informational: the `crd` crate reads the variable itself (it cannot
    /// depend on `common`) with the same parsing rule.
    pub allow_cross_namespace_cert: bool,
}

/// What this process runs (`PROXYAUTH_ROLE`, RFC 0012 decision 26).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Role {
    /// The proxy and its controller (default).
    #[default]
    Proxy,
    /// The agent gateway only: the agents' listener, the tunnels, `CONNECT`.
    Gateway,
    /// Both in one pod, for a small installation.
    Embedded,
}

impl Role {
    /// `proxy`/`gateway`/`embedded`, case-insensitive and trimmed.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "proxy" => Some(Self::Proxy),
            "gateway" => Some(Self::Gateway),
            "embedded" => Some(Self::Embedded),
            _ => None,
        }
    }

    #[must_use]
    pub fn serves_proxy(self) -> bool {
        matches!(self, Self::Proxy | Self::Embedded)
    }

    #[must_use]
    pub fn serves_gateway(self) -> bool {
        matches!(self, Self::Gateway | Self::Embedded)
    }
}

/// Who ends the agents' TLS (RFC 0012 §5.3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AgentTlsMode {
    /// The ingress ends the outer TLS; the gateway runs a mutual TLS under
    /// the agent CA inside the WebSocket.
    #[default]
    Inner,
    /// The gateway ends the TLS itself with the chart's certificate, and
    /// reads the agent's client certificate from it.
    Passthrough { cert_path: String, key_path: String },
}

/// The agent gateway (RFC 0012 §4.1, decisions 26 and 27): its listeners and
/// bounds when this process is one, and how the proxy reaches it.
#[derive(Clone, PartialEq, Eq)]
pub struct AgentGatewaySettings {
    /// `PROXYAUTH_ROLE` (default `proxy`).
    pub role: Role,
    /// `AGENT_GATEWAY_PORT` (default `8444`): the agents' listener, never the
    /// user API's port.
    pub port: u16,
    /// `AGENT_GATEWAY_CONNECT_PORT` (default `8445`): `CONNECT` from the proxy,
    /// inside the relay cluster only.
    pub connect_port: u16,
    /// `AGENT_GATEWAY_SERVICE`: the gateway's headless Service as a DNS
    /// suffix (`proxyauth-gateway.proxyauth.svc`); a gateway pod is
    /// `<pod>.<service>`. Unset on a proxy: no `Agent` cluster is served.
    pub service: Option<String>,
    /// `AGENT_GATEWAY_TOKEN`: what the proxy presents in `Proxy-Authorization`
    /// and the gateway expects (T18). Redacted from `Debug`. Unset on a
    /// gateway: every `CONNECT` is refused.
    pub token: Option<String>,
    /// `POD_NAMESPACE` (default: `LEASE_NAMESPACE`): where the agent CA Secret
    /// lives.
    pub namespace: String,
    /// `AGENT_GATEWAY_TUNNELS_PER_IDENTITY` (default `4`, > 0): open tunnels
    /// per agent identity on this pod, agent replicas together.
    pub tunnels_per_identity: usize,
    /// `AGENT_GATEWAY_STREAMS_PER_TUNNEL` (default `128`, > 0): 32 MiB of
    /// yamux window per tunnel at worst.
    pub streams_per_tunnel: usize,
    /// `AGENT_GATEWAY_MAX_STREAMS_PER_TUNNEL` (default `512`, at least
    /// `streams_per_tunnel`): ceiling of a per-cluster override.
    pub max_streams_per_tunnel: usize,
    /// `AGENT_GATEWAY_HANDSHAKE_TIMEOUT` in seconds (default `10`, > 0).
    pub handshake_timeout: Duration,
    /// `AGENT_GATEWAY_ENROLL_PER_MINUTE` (default `10`, > 0): enrolments per
    /// real client address.
    pub enroll_per_minute: usize,
    /// `AGENT_GATEWAY_ENROLL_PER_MINUTE_TOTAL` (default `120`, > 0):
    /// enrolments of every agent together.
    pub enroll_per_minute_total: usize,
    /// `AGENT_GATEWAY_TLS_MODE` (`inner`, default, or `passthrough`, its
    /// certificate at `AGENT_GATEWAY_TLS_CERT_PATH` and
    /// `AGENT_GATEWAY_TLS_KEY_PATH`, default `/agent-tls/tls.crt` and
    /// `/agent-tls/tls.key`).
    pub tls_mode: AgentTlsMode,
}

impl fmt::Debug for AgentGatewaySettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentGatewaySettings")
            .field("role", &self.role)
            .field("port", &self.port)
            .field("connect_port", &self.connect_port)
            .field("service", &self.service)
            .field("token", &self.token.as_ref().map(|_| "***REDACTED***"))
            .field("namespace", &self.namespace)
            .field("tunnels_per_identity", &self.tunnels_per_identity)
            .field("streams_per_tunnel", &self.streams_per_tunnel)
            .field("max_streams_per_tunnel", &self.max_streams_per_tunnel)
            .field("handshake_timeout", &self.handshake_timeout)
            .field("enroll_per_minute", &self.enroll_per_minute)
            .field("enroll_per_minute_total", &self.enroll_per_minute_total)
            .field("tls_mode", &self.tls_mode)
            .finish()
    }
}

/// Observability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservabilitySettings {
    /// `POD_NAME` (default `not_a_pod`): added to the OpenTelemetry resource.
    pub pod_name: String,
    /// `LOG_FORMAT` (default `text`; `json` for one JSON object per line).
    /// Unknown values fall back to `text`, with a warning.
    pub log_format: LogFormat,
    /// `METRICS_PROMETHEUS_ENABLED` (default `false`): serve the metrics in the
    /// Prometheus text format on `GET /management/metrics`, alongside the OTLP
    /// export.
    pub metrics_prometheus_enabled: bool,
    /// `WEEBO_METRICS_ADDR` (unset by default): also serve the Prometheus
    /// metrics on `GET /metrics` at this address, a listener of its own; set,
    /// it turns the Prometheus export on. What `weebo-kit-perf` scrapes
    /// (RFC 0011, decision 42).
    pub weebo_metrics_addr: Option<std::net::SocketAddr>,
}

/// `LOG_FORMAT`: shape of the stdout log lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable lines.
    #[default]
    Text,
    /// One JSON object per line.
    Json,
}

impl LogFormat {
    /// `text`/`json`, case-insensitive and trimmed; `None` otherwise.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

impl Config {
    /// Load the configuration from the process environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Load the configuration from `lookup`, which returns the raw value of a
    /// variable (`None` when unset). Pure: this is what tests use.
    #[must_use]
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let mut loader = Loader {
            lookup: &lookup,
            warnings: Vec::new(),
        };

        let server = ServerSettings {
            port: loader.parsed("SERVER_PORT", DEFAULT_SERVER_PORT, |v| v.parse().ok()),
            https: loader.parsed("SERVER_HTTPS", false, |v| v.parse().ok()),
            cert_path: loader.raw("SERVER_CERT_PATH"),
            key_path: loader.raw("SERVER_KEY_PATH"),
            shutdown_timeout_secs: loader.shutdown_timeout(),
            cors_allowed_origins: loader
                .raw("CORS_ALLOWED_ORIGINS")
                .map(|value| CorsOrigins::parse(&value))
                .unwrap_or_default(),
        };

        let oidc = OidcSettings {
            client_id: loader.or("OIDC_CLIENT_ID", "proxy-auth-k8s"),
            client_secret: loader.raw("OIDC_CLIENT_SECRET"),
            issuer_url: loader.or("OIDC_ISSUER_URL", "https://authelia.k8s.localhost"),
            scopes: loader.or("OIDC_SCOPES", "openid email profile"),
            audience: loader.or("OIDC_AUDIENCE", "proxy-auth-k8s"),
            redirect_url: loader.raw("OIDC_REDIRECT_URL"),
            audience_validation: loader.audience_validation(),
            token_cache_ttl: loader.parsed("OIDC_TOKEN_CACHE_TTL", DEFAULT_TOKEN_TTL, |v| {
                v.trim().parse::<u64>().ok().map(Duration::from_secs)
            }),
            cluster_redirect_base_url: loader.or(
                "API_CLUSTER_OIDC_BASE_REDIRECT_URL",
                "https://localhost:5437",
            ),
            front_redirect_base_url: loader.or(
                "API_CLUSTER_OIDC_FRONT_REDIRECT_URL",
                "https://localhost:4200/auth/callback/",
            ),
        };

        let redis = RedisSettings {
            url: loader.or_warn("REDIS_URL", DEFAULT_REDIS_URL),
            cluster: loader.parsed("REDIS_CLUSTER", false, |v| {
                if v.eq_ignore_ascii_case("true") || v == "1" {
                    Some(true)
                } else if v.eq_ignore_ascii_case("false") || v == "0" {
                    Some(false)
                } else {
                    None
                }
            }),
        };

        let leader_election = LeaderElectionSettings {
            lease_namespace: loader.or_warn("LEASE_NAMESPACE", "default"),
            lease_name: loader.or("HOSTNAME", "NOT_A_POD"),
        };

        let proxy = ProxySettings {
            trusted_proxy_count: loader
                .parsed("TRUSTED_PROXY_COUNT", 0, |v| v.trim().parse::<usize>().ok()),
            debug_body_max_bytes: loader
                .positive_usize("PROXY_DEBUG_BODY_MAX_BYTES", DEFAULT_DEBUG_BODY_MAX_BYTES),
            stream_channel_capacity: loader.positive_usize(
                "PROXY_STREAM_CHANNEL_CAPACITY",
                DEFAULT_STREAM_CHANNEL_CAPACITY,
            ),
            virtual_max_body_bytes: loader.positive_usize(
                "PROXY_VIRTUAL_MAX_BODY_BYTES",
                DEFAULT_VIRTUAL_MAX_BODY_BYTES,
            ),
            jwks_max_document_bytes: loader.positive_usize(
                "PROXY_JWKS_MAX_DOCUMENT_BYTES",
                DEFAULT_JWKS_MAX_DOCUMENT_BYTES,
            ),
            upstream_client_ttl: loader.parsed(
                "PROXY_UPSTREAM_CLIENT_TTL",
                DEFAULT_UPSTREAM_CLIENT_TTL,
                |v| v.trim().parse::<u64>().ok().map(Duration::from_secs),
            ),
            throttle_fail_closed: loader.flag("THROTTLE_FAIL_CLOSED"),
            allow_cross_namespace_cert: loader.flag(crd::certificate::ALLOW_CROSS_NS_CERT_ENV),
        };

        let observability = ObservabilitySettings {
            pod_name: loader.or("POD_NAME", "not_a_pod"),
            log_format: loader.parsed("LOG_FORMAT", LogFormat::Text, LogFormat::parse),
            metrics_prometheus_enabled: loader.flag("METRICS_PROMETHEUS_ENABLED"),
            weebo_metrics_addr: loader
                .parsed("WEEBO_METRICS_ADDR", None, |raw| raw.parse().ok().map(Some)),
        };

        let streams_per_tunnel = loader.positive_usize("AGENT_GATEWAY_STREAMS_PER_TUNNEL", 128);
        let mut max_streams_per_tunnel =
            loader.positive_usize("AGENT_GATEWAY_MAX_STREAMS_PER_TUNNEL", 512);
        if max_streams_per_tunnel < streams_per_tunnel {
            loader.warnings.push(format!(
                "AGENT_GATEWAY_MAX_STREAMS_PER_TUNNEL={max_streams_per_tunnel} is below \
                 AGENT_GATEWAY_STREAMS_PER_TUNNEL={streams_per_tunnel}, raising it to match"
            ));
            max_streams_per_tunnel = streams_per_tunnel;
        }
        let agent_gateway = AgentGatewaySettings {
            role: loader.parsed("PROXYAUTH_ROLE", Role::Proxy, Role::parse),
            port: loader.parsed("AGENT_GATEWAY_PORT", DEFAULT_AGENT_GATEWAY_PORT, |v| {
                v.trim().parse().ok()
            }),
            connect_port: loader.parsed(
                "AGENT_GATEWAY_CONNECT_PORT",
                DEFAULT_AGENT_GATEWAY_CONNECT_PORT,
                |v| v.trim().parse().ok(),
            ),
            service: loader
                .raw("AGENT_GATEWAY_SERVICE")
                .filter(|v| !v.is_empty()),
            token: loader.raw("AGENT_GATEWAY_TOKEN").filter(|v| !v.is_empty()),
            namespace: loader
                .raw("POD_NAMESPACE")
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| leader_election.lease_namespace.clone()),
            tunnels_per_identity: loader.positive_usize("AGENT_GATEWAY_TUNNELS_PER_IDENTITY", 4),
            streams_per_tunnel,
            max_streams_per_tunnel,
            handshake_timeout: loader.parsed(
                "AGENT_GATEWAY_HANDSHAKE_TIMEOUT",
                Duration::from_secs(10),
                |v| {
                    v.trim()
                        .parse::<u64>()
                        .ok()
                        .filter(|secs| *secs > 0)
                        .map(Duration::from_secs)
                },
            ),
            enroll_per_minute: loader.positive_usize("AGENT_GATEWAY_ENROLL_PER_MINUTE", 10),
            enroll_per_minute_total: loader
                .positive_usize("AGENT_GATEWAY_ENROLL_PER_MINUTE_TOTAL", 120),
            tls_mode: if loader.parsed("AGENT_GATEWAY_TLS_MODE", "inner", |v| {
                match v.trim().to_ascii_lowercase().as_str() {
                    "inner" => Some("inner"),
                    "passthrough" => Some("passthrough"),
                    _ => None,
                }
            }) == "passthrough"
            {
                AgentTlsMode::Passthrough {
                    cert_path: loader.or("AGENT_GATEWAY_TLS_CERT_PATH", "/agent-tls/tls.crt"),
                    key_path: loader.or("AGENT_GATEWAY_TLS_KEY_PATH", "/agent-tls/tls.key"),
                }
            } else {
                AgentTlsMode::Inner
            },
        };

        Self {
            server,
            oidc,
            redis,
            leader_election,
            proxy,
            observability,
            agent_gateway,
            warnings: loader.warnings,
        }
    }

    /// Fallbacks taken while loading, one human-readable line each.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Log every loading warning at `warn` level.
    pub fn log_warnings(&self) {
        for message in &self.warnings {
            warn!("{message}");
        }
    }
}

/// The process-wide configuration.
static CONFIG: OnceLock<Config> = OnceLock::new();

/// Install `config` as the process-wide configuration and log its warnings.
///
/// Call it once from `main`, after tracing is initialised. If a configuration
/// is already installed (an earlier [`init`], or a [`get`] that loaded it
/// lazily), that one is kept and returned, and a warning is logged.
pub fn init(config: Config) -> &'static Config {
    let mut installed = false;
    let current = CONFIG.get_or_init(|| {
        installed = true;
        config
    });
    if installed {
        current.log_warnings();
    } else {
        warn!("configuration already initialised; keeping the existing one");
    }
    current
}

/// The process-wide configuration, loaded from the environment on first use
/// when [`init`] was never called.
pub fn get() -> &'static Config {
    CONFIG.get_or_init(|| {
        let config = Config::from_env();
        config.log_warnings();
        config
    })
}

/// Reads raw values through the lookup and records fallbacks.
struct Loader<'a, F: Fn(&str) -> Option<String>> {
    lookup: &'a F,
    warnings: Vec<String>,
}

impl<F: Fn(&str) -> Option<String>> Loader<'_, F> {
    /// The raw value, as `std::env::var(key).ok()` would return it (an empty
    /// value is kept).
    fn raw(&self, key: &str) -> Option<String> {
        (self.lookup)(key)
    }

    /// The raw value, or `default` when unset (an empty value is kept).
    fn or(&self, key: &str, default: &str) -> String {
        self.raw(key).unwrap_or_else(|| default.to_string())
    }

    /// The raw value, or `default` (with a warning) when unset or empty: for
    /// local-dev defaults a production deployment must override.
    fn or_warn(&mut self, key: &str, default: &str) -> String {
        match self.raw(key).filter(|value| !value.is_empty()) {
            Some(value) => value,
            None => {
                self.warnings.push(format!(
                    "{key} is not set, falling back to the local-dev default `{default}`"
                ));
                default.to_string()
            }
        }
    }

    /// `parse(raw)`, or `default` when unset or when `parse` rejects the value
    /// (recorded as a warning).
    fn parsed<T: fmt::Debug>(
        &mut self,
        key: &str,
        default: T,
        parse: impl FnOnce(&str) -> Option<T>,
    ) -> T {
        let Some(raw) = self.raw(key) else {
            return default;
        };
        if let Some(value) = parse(&raw) {
            return value;
        }
        self.warnings.push(format!(
            "{key}=`{raw}` is not a valid value, using the default {default:?}"
        ));
        default
    }

    /// A strictly positive `usize` (no surrounding whitespace allowed), else
    /// `default`.
    fn positive_usize(&mut self, key: &str, default: usize) -> usize {
        self.parsed(key, default, |v| {
            v.parse::<usize>().ok().filter(|value| *value > 0)
        })
    }

    /// A boolean switch: `true`/`1`/`yes`/`on`/`enabled` (case-insensitive,
    /// trimmed) turn it on, anything else leaves it off. A value that is not a
    /// recognisable "off" either is recorded as a warning.
    fn flag(&mut self, key: &str) -> bool {
        let raw = self.raw(key).unwrap_or_default();
        if is_truthy(&raw) {
            return true;
        }
        let normalized = raw.trim().to_ascii_lowercase();
        if !matches!(
            normalized.as_str(),
            "" | "false" | "0" | "no" | "off" | "disabled"
        ) {
            self.warnings.push(format!(
                "{key}=`{raw}` is not a recognised boolean, treating it as false"
            ));
        }
        false
    }

    /// `SERVER_SHUTDOWN_TIMEOUT`: trimmed, empty means unset.
    fn shutdown_timeout(&mut self) -> u64 {
        const KEY: &str = "SERVER_SHUTDOWN_TIMEOUT";
        let raw = self.raw(KEY);
        parse_shutdown_timeout(raw.as_deref()).unwrap_or_else(|value| {
            self.warnings.push(format!(
                "{KEY}=`{value}` is not a number of seconds, using the default {DEFAULT_SHUTDOWN_TIMEOUT_SECS}"
            ));
            DEFAULT_SHUTDOWN_TIMEOUT_SECS
        })
    }

    /// `OIDC_AUDIENCE_VALIDATION`: unknown values fail closed to `Enforce`,
    /// with a warning.
    fn audience_validation(&mut self) -> AudienceValidationMode {
        const KEY: &str = "OIDC_AUDIENCE_VALIDATION";
        let raw = self.raw(KEY).unwrap_or_default();
        let mode = AudienceValidationMode::from_str_value(&raw);
        let normalized = raw.trim().to_ascii_lowercase();
        if mode == AudienceValidationMode::Enforce && !matches!(normalized.as_str(), "" | "enforce")
        {
            self.warnings.push(format!(
                "{KEY}=`{raw}` is not one of enforce/warn/off, enforcing audience validation"
            ));
        }
        mode
    }
}

/// Whether a switch value turns the switch on.
fn is_truthy(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on" | "enabled"
    )
}

/// Parse the graceful-shutdown timeout (seconds) from the raw value.
///
/// Returns the default when unset or blank; an unparsable value is reported as
/// `Err` with the trimmed raw value.
fn parse_shutdown_timeout(raw: Option<&str>) -> Result<u64, String> {
    match raw.map(str::trim) {
        None | Some("") => Ok(DEFAULT_SHUTDOWN_TIMEOUT_SECS),
        Some(value) => value.parse::<u64>().map_err(|_| value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn load(vars: &[(&str, &str)]) -> Config {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        Config::from_lookup(|key| vars.get(key).cloned())
    }

    #[test]
    fn an_empty_environment_yields_the_documented_defaults() {
        let config = load(&[]);
        assert_eq!(
            config.server,
            ServerSettings {
                port: 5437,
                https: false,
                cert_path: None,
                key_path: None,
                shutdown_timeout_secs: 30,
                cors_allowed_origins: CorsOrigins::SameOriginOnly,
            }
        );
        assert_eq!(config.oidc.client_id, "proxy-auth-k8s");
        assert_eq!(config.oidc.client_secret, None);
        assert_eq!(config.oidc.issuer_url, "https://authelia.k8s.localhost");
        assert_eq!(config.oidc.scopes, "openid email profile");
        assert_eq!(config.oidc.audience, "proxy-auth-k8s");
        assert_eq!(config.oidc.redirect_url, None);
        assert_eq!(
            config.oidc.audience_validation,
            AudienceValidationMode::Enforce
        );
        assert_eq!(config.oidc.token_cache_ttl, Duration::from_secs(30));
        assert_eq!(
            config.oidc.cluster_redirect_base_url,
            "https://localhost:5437"
        );
        assert_eq!(
            config.oidc.front_redirect_base_url,
            "https://localhost:4200/auth/callback/"
        );
        assert_eq!(
            config.redis,
            RedisSettings {
                url: "redis://127.0.0.1:6379".to_string(),
                cluster: false,
            }
        );
        assert_eq!(
            config.leader_election,
            LeaderElectionSettings {
                lease_namespace: "default".to_string(),
                lease_name: "NOT_A_POD".to_string(),
            }
        );
        assert_eq!(
            config.proxy,
            ProxySettings {
                trusted_proxy_count: 0,
                debug_body_max_bytes: 10 * 1024 * 1024,
                stream_channel_capacity: 32,
                virtual_max_body_bytes: 32 * 1024 * 1024,
                jwks_max_document_bytes: 64 * 1024,
                upstream_client_ttl: Duration::from_secs(300),
                throttle_fail_closed: false,
                allow_cross_namespace_cert: false,
            }
        );
        assert_eq!(
            config.observability,
            ObservabilitySettings {
                pod_name: "not_a_pod".to_string(),
                log_format: LogFormat::Text,
                metrics_prometheus_enabled: false,
                weebo_metrics_addr: None,
            }
        );
        assert_eq!(
            config.agent_gateway,
            AgentGatewaySettings {
                role: Role::Proxy,
                port: 8444,
                connect_port: 8445,
                service: None,
                token: None,
                namespace: "default".to_string(),
                tunnels_per_identity: 4,
                streams_per_tunnel: 128,
                max_streams_per_tunnel: 512,
                handshake_timeout: Duration::from_secs(10),
                enroll_per_minute: 10,
                enroll_per_minute_total: 120,
                tls_mode: AgentTlsMode::Inner,
            }
        );
    }

    #[test]
    fn the_agent_gateway_settings_are_read_and_bounded() {
        let config = load(&[
            ("PROXYAUTH_ROLE", "Gateway"),
            ("AGENT_GATEWAY_PORT", "9444"),
            ("AGENT_GATEWAY_SERVICE", "proxyauth-gateway.proxyauth.svc"),
            ("AGENT_GATEWAY_TOKEN", "s3cret"),
            ("POD_NAMESPACE", "proxyauth"),
            ("AGENT_GATEWAY_STREAMS_PER_TUNNEL", "256"),
            ("AGENT_GATEWAY_MAX_STREAMS_PER_TUNNEL", "200"),
            ("AGENT_GATEWAY_TUNNELS_PER_IDENTITY", "0"),
            ("AGENT_GATEWAY_HANDSHAKE_TIMEOUT", "0"),
            ("AGENT_GATEWAY_TLS_MODE", "Passthrough"),
            ("AGENT_GATEWAY_TLS_KEY_PATH", "/tls/key.pem"),
        ]);
        let gateway = &config.agent_gateway;
        assert_eq!(gateway.role, Role::Gateway);
        assert!(gateway.role.serves_gateway() && !gateway.role.serves_proxy());
        assert_eq!(gateway.port, 9444);
        assert_eq!(
            gateway.service.as_deref(),
            Some("proxyauth-gateway.proxyauth.svc")
        );
        assert_eq!(gateway.namespace, "proxyauth");
        // A ceiling below the default is raised to it, with a warning.
        assert_eq!(gateway.max_streams_per_tunnel, 256);
        assert!(
            config
                .warnings()
                .iter()
                .any(|w| w.contains("raising it to match"))
        );
        // Zero would disable a limit: refused, the default is kept.
        assert_eq!(gateway.tunnels_per_identity, 4);
        assert_eq!(gateway.handshake_timeout, Duration::from_secs(10));
        assert!(!format!("{gateway:?}").contains("s3cret"));
        assert_eq!(
            gateway.tls_mode,
            AgentTlsMode::Passthrough {
                cert_path: "/agent-tls/tls.crt".to_string(),
                key_path: "/tls/key.pem".to_string(),
            }
        );
    }

    #[test]
    fn an_unknown_role_falls_back_to_the_proxy() {
        let config = load(&[("PROXYAUTH_ROLE", "agent")]);
        assert_eq!(config.agent_gateway.role, Role::Proxy);
        assert!(Role::Embedded.serves_proxy() && Role::Embedded.serves_gateway());
    }

    #[test]
    fn only_the_local_dev_defaults_warn_when_unset() {
        let config = load(&[]);
        assert_eq!(
            config.warnings(),
            [
                "REDIS_URL is not set, falling back to the local-dev default `redis://127.0.0.1:6379`",
                "LEASE_NAMESPACE is not set, falling back to the local-dev default `default`",
            ]
        );
    }

    #[test]
    fn set_values_are_used() {
        let config = load(&[
            ("SERVER_PORT", "8443"),
            ("SERVER_HTTPS", "true"),
            ("SERVER_CERT_PATH", "/tls/crt.pem"),
            ("SERVER_KEY_PATH", "/tls/key.pem"),
            ("SERVER_SHUTDOWN_TIMEOUT", " 120 "),
            (
                "CORS_ALLOWED_ORIGINS",
                " https://a.example , ,https://b.example",
            ),
            ("OIDC_CLIENT_ID", "client"),
            ("OIDC_CLIENT_SECRET", "s3cr3t"),
            ("OIDC_ISSUER_URL", "https://idp.example"),
            ("OIDC_SCOPES", "openid"),
            ("OIDC_AUDIENCE", "aud"),
            ("OIDC_REDIRECT_URL", "https://app.example/cb"),
            ("OIDC_AUDIENCE_VALIDATION", "Warn"),
            ("OIDC_TOKEN_CACHE_TTL", " 0 "),
            ("API_CLUSTER_OIDC_BASE_REDIRECT_URL", "https://api.example"),
            (
                "API_CLUSTER_OIDC_FRONT_REDIRECT_URL",
                "https://ui.example/cb/",
            ),
            ("REDIS_URL", "redis://redis:6379"),
            ("REDIS_CLUSTER", "TRUE"),
            ("LEASE_NAMESPACE", "proxyauth"),
            ("HOSTNAME", "pod-0"),
            ("TRUSTED_PROXY_COUNT", " 2 "),
            ("PROXY_DEBUG_BODY_MAX_BYTES", "1024"),
            ("PROXY_STREAM_CHANNEL_CAPACITY", "8"),
            ("PROXY_VIRTUAL_MAX_BODY_BYTES", "2048"),
            ("PROXY_JWKS_MAX_DOCUMENT_BYTES", "1048576"),
            ("PROXY_UPSTREAM_CLIENT_TTL", " 60 "),
            ("THROTTLE_FAIL_CLOSED", " Yes "),
            ("PROXYAUTH_ALLOW_CROSS_NS_CERT", "enabled"),
            ("POD_NAME", "proxyauth-0"),
            ("LOG_FORMAT", " JSON "),
            ("METRICS_PROMETHEUS_ENABLED", "true"),
            ("WEEBO_METRICS_ADDR", "127.0.0.1:9464"),
        ]);
        assert!(config.warnings().is_empty(), "{:?}", config.warnings());
        assert_eq!(
            config.server,
            ServerSettings {
                port: 8443,
                https: true,
                cert_path: Some("/tls/crt.pem".to_string()),
                key_path: Some("/tls/key.pem".to_string()),
                shutdown_timeout_secs: 120,
                cors_allowed_origins: CorsOrigins::List(vec![
                    "https://a.example".to_string(),
                    "https://b.example".to_string(),
                ]),
            }
        );
        assert_eq!(config.oidc.client_id, "client");
        assert_eq!(config.oidc.client_secret.as_deref(), Some("s3cr3t"));
        assert_eq!(config.oidc.issuer_url, "https://idp.example");
        assert_eq!(config.oidc.scopes, "openid");
        assert_eq!(config.oidc.audience, "aud");
        assert_eq!(
            config.oidc.redirect_url.as_deref(),
            Some("https://app.example/cb")
        );
        assert_eq!(
            config.oidc.audience_validation,
            AudienceValidationMode::Warn
        );
        assert_eq!(config.oidc.token_cache_ttl, Duration::ZERO);
        assert_eq!(config.oidc.cluster_redirect_base_url, "https://api.example");
        assert_eq!(
            config.oidc.front_redirect_base_url,
            "https://ui.example/cb/"
        );
        assert_eq!(config.redis.url, "redis://redis:6379");
        assert!(config.redis.cluster);
        assert_eq!(config.leader_election.lease_namespace, "proxyauth");
        assert_eq!(config.leader_election.lease_name, "pod-0");
        assert_eq!(
            config.proxy,
            ProxySettings {
                trusted_proxy_count: 2,
                debug_body_max_bytes: 1024,
                stream_channel_capacity: 8,
                virtual_max_body_bytes: 2048,
                jwks_max_document_bytes: 1024 * 1024,
                upstream_client_ttl: Duration::from_secs(60),
                throttle_fail_closed: true,
                allow_cross_namespace_cert: true,
            }
        );
        assert_eq!(
            config.observability,
            ObservabilitySettings {
                pod_name: "proxyauth-0".to_string(),
                log_format: LogFormat::Json,
                metrics_prometheus_enabled: true,
                weebo_metrics_addr: Some("127.0.0.1:9464".parse().expect("static addr")),
            }
        );
    }

    #[test]
    fn log_format_parsing() {
        assert_eq!(LogFormat::parse("text"), Some(LogFormat::Text));
        assert_eq!(LogFormat::parse(" Text "), Some(LogFormat::Text));
        assert_eq!(LogFormat::parse("json"), Some(LogFormat::Json));
        assert_eq!(LogFormat::parse("JSON"), Some(LogFormat::Json));
        assert_eq!(LogFormat::parse(""), None);
        assert_eq!(LogFormat::parse("logfmt"), None);
    }

    #[test]
    fn cors_star_alone_is_an_explicit_any_origin() {
        assert_eq!(CorsOrigins::parse("*"), CorsOrigins::Any);
        assert_eq!(CorsOrigins::parse(" * "), CorsOrigins::Any);
    }

    #[test]
    fn cors_blank_means_same_origin_only() {
        assert_eq!(CorsOrigins::parse(""), CorsOrigins::SameOriginOnly);
        assert_eq!(CorsOrigins::parse(" , ,"), CorsOrigins::SameOriginOnly);
    }

    #[test]
    fn cors_star_inside_a_list_is_dropped_not_a_wildcard() {
        // Only a lone `*` opts into any origin; mixed with real origins it is
        // dropped rather than widening the list.
        assert_eq!(
            CorsOrigins::parse("https://a.example,*"),
            CorsOrigins::List(vec!["https://a.example".to_string()])
        );
    }

    #[test]
    fn empty_values_keep_their_historical_meaning() {
        let config = load(&[
            // Plain `unwrap_or` reads kept an empty value as-is...
            ("OIDC_CLIENT_ID", ""),
            ("HOSTNAME", ""),
            ("OIDC_CLIENT_SECRET", ""),
            // ...while the warned local-dev defaults treat it as unset.
            ("REDIS_URL", ""),
            ("LEASE_NAMESPACE", ""),
            ("CORS_ALLOWED_ORIGINS", " , "),
            ("SERVER_SHUTDOWN_TIMEOUT", "  "),
        ]);
        assert_eq!(config.oidc.client_id, "");
        assert_eq!(config.leader_election.lease_name, "");
        assert_eq!(config.oidc.client_secret.as_deref(), Some(""));
        assert_eq!(config.redis.url, DEFAULT_REDIS_URL);
        assert_eq!(config.leader_election.lease_namespace, "default");
        assert_eq!(
            config.server.cors_allowed_origins,
            CorsOrigins::SameOriginOnly
        );
        assert_eq!(config.server.shutdown_timeout_secs, 30);
    }

    #[test]
    fn unparsable_values_fall_back_with_a_warning() {
        let config = load(&[
            ("REDIS_URL", "redis://redis:6379"),
            ("LEASE_NAMESPACE", "ns"),
            ("SERVER_PORT", " 8443"),
            ("SERVER_HTTPS", "yes"),
            ("SERVER_SHUTDOWN_TIMEOUT", "30s"),
            ("OIDC_TOKEN_CACHE_TTL", "-1"),
            ("OIDC_AUDIENCE_VALIDATION", "strict"),
            ("REDIS_CLUSTER", "maybe"),
            ("TRUSTED_PROXY_COUNT", "one"),
            ("PROXY_DEBUG_BODY_MAX_BYTES", "0"),
            ("PROXY_STREAM_CHANNEL_CAPACITY", " 8"),
            ("PROXY_VIRTUAL_MAX_BODY_BYTES", "big"),
            ("PROXY_JWKS_MAX_DOCUMENT_BYTES", "0"),
            ("PROXY_UPSTREAM_CLIENT_TTL", "5m"),
            ("THROTTLE_FAIL_CLOSED", "sure"),
            ("PROXYAUTH_ALLOW_CROSS_NS_CERT", "nope"),
            ("LOG_FORMAT", "logfmt"),
            ("METRICS_PROMETHEUS_ENABLED", "maybe"),
        ]);
        assert_eq!(config.server.port, 5437);
        assert!(!config.server.https);
        assert_eq!(config.server.shutdown_timeout_secs, 30);
        assert_eq!(config.oidc.token_cache_ttl, Duration::from_secs(30));
        assert_eq!(
            config.oidc.audience_validation,
            AudienceValidationMode::Enforce
        );
        assert!(!config.redis.cluster);
        assert_eq!(config.proxy.trusted_proxy_count, 0);
        assert_eq!(
            config.proxy.debug_body_max_bytes,
            DEFAULT_DEBUG_BODY_MAX_BYTES
        );
        assert_eq!(
            config.proxy.stream_channel_capacity,
            DEFAULT_STREAM_CHANNEL_CAPACITY
        );
        assert_eq!(
            config.proxy.virtual_max_body_bytes,
            DEFAULT_VIRTUAL_MAX_BODY_BYTES
        );
        assert_eq!(
            config.proxy.jwks_max_document_bytes,
            DEFAULT_JWKS_MAX_DOCUMENT_BYTES
        );
        assert_eq!(
            config.proxy.upstream_client_ttl,
            DEFAULT_UPSTREAM_CLIENT_TTL
        );
        assert!(!config.proxy.throttle_fail_closed);
        assert!(!config.proxy.allow_cross_namespace_cert);
        assert_eq!(config.observability.log_format, LogFormat::Text);
        assert!(!config.observability.metrics_prometheus_enabled);

        let warned: Vec<&str> = config
            .warnings()
            .iter()
            .map(|w| w.split('=').next().unwrap_or_default())
            .collect();
        assert_eq!(
            warned,
            [
                "SERVER_PORT",
                "SERVER_HTTPS",
                "SERVER_SHUTDOWN_TIMEOUT",
                "OIDC_AUDIENCE_VALIDATION",
                "OIDC_TOKEN_CACHE_TTL",
                "REDIS_CLUSTER",
                "TRUSTED_PROXY_COUNT",
                "PROXY_DEBUG_BODY_MAX_BYTES",
                "PROXY_STREAM_CHANNEL_CAPACITY",
                "PROXY_VIRTUAL_MAX_BODY_BYTES",
                "PROXY_JWKS_MAX_DOCUMENT_BYTES",
                "PROXY_UPSTREAM_CLIENT_TTL",
                "THROTTLE_FAIL_CLOSED",
                "PROXYAUTH_ALLOW_CROSS_NS_CERT",
                "LOG_FORMAT",
                "METRICS_PROMETHEUS_ENABLED",
            ]
        );
    }

    #[test]
    fn upstream_client_ttl_parsing() {
        let ttl = |raw: &str| {
            load(&[("PROXY_UPSTREAM_CLIENT_TTL", raw)])
                .proxy
                .upstream_client_ttl
        };
        assert_eq!(ttl("0"), Duration::ZERO, "0 disables the cache");
        assert_eq!(ttl(" 30 "), Duration::from_secs(30));
        assert_eq!(ttl("-1"), DEFAULT_UPSTREAM_CLIENT_TTL);
        assert_eq!(ttl(""), DEFAULT_UPSTREAM_CLIENT_TTL);
        assert_eq!(
            load(&[]).proxy.upstream_client_ttl,
            DEFAULT_UPSTREAM_CLIENT_TTL
        );
    }

    #[test]
    fn recognised_off_values_do_not_warn() {
        let config = load(&[
            ("REDIS_URL", "redis://redis:6379"),
            ("LEASE_NAMESPACE", "ns"),
            ("REDIS_CLUSTER", "0"),
            ("THROTTLE_FAIL_CLOSED", " Off "),
            ("PROXYAUTH_ALLOW_CROSS_NS_CERT", "false"),
            ("OIDC_AUDIENCE_VALIDATION", "enforce"),
            ("LOG_FORMAT", "text"),
            ("METRICS_PROMETHEUS_ENABLED", "false"),
        ]);
        assert!(config.warnings().is_empty(), "{:?}", config.warnings());
    }

    #[test]
    fn the_client_secret_is_redacted_from_debug() {
        let config = load(&[("OIDC_CLIENT_SECRET", "s3cr3t")]);
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("s3cr3t"));
        assert!(rendered.contains("REDACTED"));
    }

    #[test]
    fn shutdown_timeout_parsing() {
        assert_eq!(
            parse_shutdown_timeout(None),
            Ok(DEFAULT_SHUTDOWN_TIMEOUT_SECS)
        );
        assert_eq!(
            parse_shutdown_timeout(Some("  ")),
            Ok(DEFAULT_SHUTDOWN_TIMEOUT_SECS)
        );
        assert_eq!(parse_shutdown_timeout(Some("120")), Ok(120));
        assert_eq!(parse_shutdown_timeout(Some(" 0 ")), Ok(0));
        assert_eq!(parse_shutdown_timeout(Some("30s")), Err("30s".to_string()));
        assert_eq!(parse_shutdown_timeout(Some("-1")), Err("-1".to_string()));
    }
}
