//! How the proxy finds the agent gateway pod holding a cluster's tunnel
//! (RFC 0012, decisions 25 and 26).
//!
//! A gateway pod publishes, in the presence set of each `ProxyKubeApi` UID it
//! holds a tunnel for, its pod name. The proxy reads that set, keeps a pod
//! name that is a DNS label, and composes `<pod>.<service>:<port>` from its
//! own settings: never an address read from Redis (T19).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use http::{HeaderMap, HeaderValue, header};
use tracing::warn;

use crate::State;
use crate::config::AgentGatewaySettings;

/// How long a gateway pod's presence lives without a refresh.
pub const PRESENCE_TTL: Duration = Duration::from_secs(30);
/// How often a gateway pod refreshes its presences.
pub const PRESENCE_REFRESH: Duration = Duration::from_secs(10);
/// Names the cluster in a `CONNECT`, by its `ProxyKubeApi` UID.
pub const AGENT_HEADER: &str = "x-proxyauth-agent";
/// How long a presence read is reused before Redis is asked again.
const FRESH: Duration = Duration::from_secs(5);
/// How long a presence read still serves while Redis cannot be reached.
const STALE: Duration = Duration::from_secs(60);

/// The presence set of the cluster whose `ProxyKubeApi` has `uid`.
#[must_use]
pub fn presence_key(uid: &str) -> String {
    format!("{}:agent:{uid}", crd::REDIS_PREFIX)
}

/// A pod name usable as the first label of a DNS name.
#[must_use]
pub fn is_pod_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GatewayError {
    #[error("no agent gateway is configured (AGENT_GATEWAY_SERVICE, AGENT_GATEWAY_TOKEN)")]
    NotConfigured,
    #[error("no agent tunnel for this cluster")]
    NoTunnel,
}

/// Where to send a `CONNECT` for one cluster, and what to present.
#[derive(Clone, PartialEq, Eq)]
pub struct GatewayRoute {
    /// `host:port` of the gateway pod.
    pub address: String,
    pub headers: HeaderMap,
}

impl std::fmt::Debug for GatewayRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The headers carry the gateway token.
        f.debug_struct("GatewayRoute")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

struct Presences {
    read_at: Instant,
    pods: Vec<String>,
}

pub struct GatewayClient {
    service: Option<String>,
    port: u16,
    token: Option<String>,
    presences: Mutex<HashMap<String, Presences>>,
    rotation: AtomicUsize,
    /// Tests only: every route goes to this `host:port`.
    #[cfg(feature = "test-util")]
    pub fixed_address: Mutex<Option<String>>,
}

impl GatewayClient {
    #[must_use]
    pub fn new(settings: &AgentGatewaySettings) -> Self {
        Self::with(
            settings.service.clone(),
            settings.connect_port,
            settings.token.clone(),
        )
    }

    #[must_use]
    pub fn with(service: Option<String>, port: u16, token: Option<String>) -> Self {
        Self {
            service,
            port,
            token,
            presences: Mutex::new(HashMap::new()),
            rotation: AtomicUsize::new(0),
            #[cfg(feature = "test-util")]
            fixed_address: Mutex::new(None),
        }
    }

    /// No gateway: every `Agent` cluster answers `agent unavailable`.
    #[must_use]
    pub fn disabled() -> Self {
        Self::with(None, 0, None)
    }

    /// The route to a gateway pod holding a tunnel for `uid`, reading the
    /// presences again when the last read is older than a few seconds.
    pub async fn route(&self, state: &State, uid: &str) -> Result<GatewayRoute, GatewayError> {
        let (Some(_), Some(_)) = (&self.service, &self.token) else {
            return Err(GatewayError::NotConfigured);
        };
        let cached = self.cached(uid, FRESH);
        let pods = match cached {
            Some(pods) => pods,
            None => match state.presence_members(&presence_key(uid)).await {
                Ok(members) => {
                    let pods: Vec<String> =
                        members.into_iter().filter(|pod| is_pod_name(pod)).collect();
                    self.store(uid, pods.clone());
                    pods
                }
                Err(error) => {
                    warn!(%error, uid, "could not read the agent presences; using the last ones");
                    self.cached(uid, STALE).unwrap_or_default()
                }
            },
        };
        self.pick(uid, &pods)
    }

    /// The route from the presences already read: what reqwest's
    /// `Proxy::custom`, which is synchronous, asks for each connection.
    #[must_use]
    pub fn cached_route(&self, uid: &str) -> Option<GatewayRoute> {
        let pods = self.cached(uid, STALE)?;
        self.pick(uid, &pods).ok()
    }

    /// Forget the presences of `uid`, after a gateway pod refused it.
    pub fn forget(&self, uid: &str) {
        self.presences
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(uid);
    }

    fn cached(&self, uid: &str, max_age: Duration) -> Option<Vec<String>> {
        let presences = self
            .presences
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        presences
            .get(uid)
            .filter(|entry| entry.read_at.elapsed() < max_age)
            .map(|entry| entry.pods.clone())
    }

    fn store(&self, uid: &str, pods: Vec<String>) {
        self.presences
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                uid.to_string(),
                Presences {
                    read_at: Instant::now(),
                    pods,
                },
            );
    }

    fn pick(&self, uid: &str, pods: &[String]) -> Result<GatewayRoute, GatewayError> {
        let (Some(service), Some(token)) = (&self.service, &self.token) else {
            return Err(GatewayError::NotConfigured);
        };
        if pods.is_empty() {
            return Err(GatewayError::NoTunnel);
        }
        let turn = self.rotation.fetch_add(1, Ordering::Relaxed);
        let pod = pods
            .get(turn.checked_rem(pods.len()).unwrap_or(0))
            .ok_or(GatewayError::NoTunnel)?;
        let address = self.address(pod, service);
        let mut headers = HeaderMap::new();
        let bearer = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| GatewayError::NotConfigured)?;
        headers.insert(header::PROXY_AUTHORIZATION, bearer);
        let agent = HeaderValue::from_str(uid).map_err(|_| GatewayError::NoTunnel)?;
        headers.insert(AGENT_HEADER, agent);
        Ok(GatewayRoute { address, headers })
    }

    #[cfg(feature = "test-util")]
    fn address(&self, pod: &str, service: &str) -> String {
        self.fixed_address
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| format!("{pod}.{service}:{}", self.port))
    }

    #[cfg(not(feature = "test-util"))]
    fn address(&self, pod: &str, service: &str) -> String {
        format!("{pod}.{service}:{}", self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> GatewayClient {
        GatewayClient::with(
            Some("proxyauth-gateway.proxyauth.svc".into()),
            8445,
            Some("t0ken".into()),
        )
    }

    #[test]
    fn only_dns_labels_are_pod_names() {
        assert!(is_pod_name("proxyauth-gateway-7d9f8-x2k4p"));
        for forged in [
            "",
            "10.0.0.1",
            "evil.example.com",
            "UPPER",
            "-lead",
            "a:443",
            "a/b",
        ] {
            assert!(!is_pod_name(forged), "{forged}");
        }
    }

    /// T19: the address is composed from this replica's settings.
    #[test]
    fn a_route_names_a_gateway_pod_never_an_address() {
        let client = client();
        let route = client.pick("aa", &["gw-0".to_string()]).unwrap();
        assert_eq!(route.address, "gw-0.proxyauth-gateway.proxyauth.svc:8445");
        assert_eq!(route.headers[header::PROXY_AUTHORIZATION], "Bearer t0ken");
        assert_eq!(route.headers[AGENT_HEADER], "aa");
        assert!(!format!("{route:?}").contains("t0ken"));
    }

    #[test]
    fn routes_rotate_over_the_pods_and_need_one() {
        let client = client();
        let pods = vec!["gw-0".to_string(), "gw-1".to_string()];
        let first = client.pick("aa", &pods).unwrap().address;
        let second = client.pick("aa", &pods).unwrap().address;
        assert_ne!(first, second);
        assert_eq!(client.pick("aa", &[]), Err(GatewayError::NoTunnel));
    }

    #[test]
    fn without_a_service_or_a_token_nothing_is_routed() {
        let pods = vec!["gw-0".to_string()];
        assert_eq!(
            GatewayClient::disabled().pick("aa", &pods),
            Err(GatewayError::NotConfigured)
        );
        let no_token = GatewayClient::with(Some("svc".into()), 8445, None);
        assert_eq!(no_token.pick("aa", &pods), Err(GatewayError::NotConfigured));
    }

    #[test]
    fn cached_routes_follow_the_last_read_and_forget_clears_them() {
        let client = client();
        assert!(client.cached_route("aa").is_none());
        client.store("aa", vec!["gw-0".into()]);
        assert!(client.cached_route("aa").is_some());
        client.forget("aa");
        assert!(client.cached_route("aa").is_none());
    }
}
