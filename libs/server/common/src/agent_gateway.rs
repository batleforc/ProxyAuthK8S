//! How the proxy finds the agent gateway pod holding a cluster's tunnel
//! (RFC 0012, decisions 25 and 26).
//!
//! A gateway pod publishes, in the presence set of each `ProxyKubeApi` UID it
//! holds a tunnel for, its pod name. The proxy reads that set, keeps the pod
//! names that are ready endpoints of the gateway's headless Service, and
//! dials the address the Kubernetes API gives for that pod: never an address
//! read from Redis (T19). A Deployment's pods have no DNS name of their own
//! (only pods that set `hostname` do), hence the EndpointSlice.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use http::{HeaderMap, HeaderValue, header};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use kube::api::{Api, ListParams};
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

/// Where gateway pod `pod` describes its tunnels of the cluster `uid`, next to
/// its member in [`presence_key`]; lives [`PRESENCE_TTL`].
#[must_use]
pub fn presence_info_key(uid: &str, pod: &str) -> String {
    format!("{}:{pod}", presence_key(uid))
}

/// How long a `OneShotToken` stays usable (RFC 0012 §4.1).
pub const BOOTSTRAP_TTL: Duration = Duration::from_secs(60 * 60);

/// Where the controller marks a `OneShotToken` of cluster `uid` as usable,
/// for [`BOOTSTRAP_TTL`]: by the token's SHA-256, never the token (T6). The
/// gateway takes it with `GETDEL`, so two enrolments cannot both use it, and
/// a wrong token burns nothing.
#[must_use]
pub fn bootstrap_key(uid: &str, token: &str) -> String {
    use sha2::{Digest as _, Sha256};
    let hash: String = Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{}:agent-bootstrap:{uid}:{hash}", crd::REDIS_PREFIX)
}

/// How long the refusal of an agent's protocols stays readable: rewritten at
/// each attempt, an agent retrying every 60 s keeps it (RFC 0013, decision 15).
pub const REFUSAL_TTL: Duration = Duration::from_secs(5 * 60);

/// Where a gateway pod records why it refused the protocols of cluster
/// `uid`'s agent, for [`REFUSAL_TTL`].
#[must_use]
pub fn agent_refused_key(uid: &str) -> String {
    format!("{}:agent-refused:{uid}", crd::REDIS_PREFIX)
}

/// The record behind [`agent_refused_key`]: `reason` is `protocol`, `version`
/// the agent's (already validated, else `invalid`), `protocols` its `min-max`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentRefusal {
    pub reason: String,
    pub version: String,
    pub protocols: String,
}

/// What a gateway pod reports about its tunnels of one cluster, for
/// `status.agent` (RFC 0012 §6.4).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PresenceInfo {
    pub tunnels: usize,
    pub versions: Vec<String>,
    pub ca_fingerprints: Vec<String>,
}

impl State {
    /// The live gateway pods of cluster `uid`, each with what it reported (an
    /// expired or unreadable record reads as empty).
    ///
    /// # Errors
    ///
    /// Returns [`crate::redis_pool::RedisPoolError`] when the presence set
    /// cannot be read.
    pub async fn agent_presences(
        &self,
        uid: &str,
    ) -> Result<Vec<(String, PresenceInfo)>, crate::redis_pool::RedisPoolError> {
        let mut presences = Vec::new();
        for pod in self.presence_members(&presence_key(uid)).await? {
            if !is_pod_name(&pod) {
                continue;
            }
            let info = self
                .redis_get(&presence_info_key(uid, &pod))
                .await
                .ok()
                .flatten()
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default();
            presences.push((pod, info));
        }
        Ok(presences)
    }
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
    /// `ip:port` of the gateway pods holding a tunnel.
    targets: Vec<String>,
}

struct Endpoints {
    read_at: Instant,
    /// Pod name → address, the ready endpoints of the headless Service.
    by_pod: HashMap<String, String>,
}

pub struct GatewayClient {
    service: Option<String>,
    port: u16,
    token: Option<String>,
    presences: Mutex<HashMap<String, Presences>>,
    endpoints: Mutex<Option<Endpoints>>,
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
            endpoints: Mutex::new(None),
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
        let targets = match cached {
            Some(targets) => targets,
            None => match state.presence_members(&presence_key(uid)).await {
                Ok(members) => {
                    let pods: Vec<String> =
                        members.into_iter().filter(|pod| is_pod_name(pod)).collect();
                    let targets = self.resolve(state, &pods).await;
                    self.store(uid, targets.clone());
                    targets
                }
                Err(error) => {
                    warn!(%error, uid, "could not read the agent presences; using the last ones");
                    self.cached(uid, STALE).unwrap_or_default()
                }
            },
        };
        self.pick(uid, &targets)
    }

    /// The addresses of `pods` among the gateway's endpoints; a pod that is
    /// not one (a forged or stale presence) is dropped.
    async fn resolve(&self, state: &State, pods: &[String]) -> Vec<String> {
        #[cfg(feature = "test-util")]
        if let Some(fixed) = self
            .fixed_address
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return pods.iter().map(|_| fixed.clone()).collect();
        }
        let by_pod = self.endpoints(state).await;
        pods.iter()
            .filter_map(|pod| by_pod.get(pod).map(|address| target(address, self.port)))
            .collect()
    }

    /// The ready endpoints of the headless Service, reused for a few seconds,
    /// and for a minute while the apiserver cannot be read.
    async fn endpoints(&self, state: &State) -> HashMap<String, String> {
        let cached = |max_age: Duration| {
            self.endpoints
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .filter(|entry| entry.read_at.elapsed() < max_age)
                .map(|entry| entry.by_pod.clone())
        };
        if let Some(fresh) = cached(FRESH) {
            return fresh;
        }
        let Some((name, namespace)) = self.service.as_deref().and_then(service_parts) else {
            return HashMap::new();
        };
        let slices: Api<EndpointSlice> = Api::namespaced(state.client.clone(), namespace);
        let params = ListParams::default().labels(&format!("kubernetes.io/service-name={name}"));
        match slices.list(&params).await {
            Ok(list) => {
                let by_pod = pod_addresses(&list.items);
                *self
                    .endpoints
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(Endpoints {
                    read_at: Instant::now(),
                    by_pod: by_pod.clone(),
                });
                by_pod
            }
            Err(error) => {
                warn!(%error, "could not read the agent gateway's endpoints; using the last ones");
                cached(STALE).unwrap_or_default()
            }
        }
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
            .map(|entry| entry.targets.clone())
    }

    fn store(&self, uid: &str, targets: Vec<String>) {
        self.presences
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                uid.to_string(),
                Presences {
                    read_at: Instant::now(),
                    targets,
                },
            );
    }

    fn pick(&self, uid: &str, targets: &[String]) -> Result<GatewayRoute, GatewayError> {
        let (Some(_), Some(token)) = (&self.service, &self.token) else {
            return Err(GatewayError::NotConfigured);
        };
        if targets.is_empty() {
            return Err(GatewayError::NoTunnel);
        }
        let turn = self.rotation.fetch_add(1, Ordering::Relaxed);
        let address = targets
            .get(turn.checked_rem(targets.len()).unwrap_or(0))
            .ok_or(GatewayError::NoTunnel)?
            .clone();
        let mut headers = HeaderMap::new();
        let bearer = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| GatewayError::NotConfigured)?;
        headers.insert(header::PROXY_AUTHORIZATION, bearer);
        let agent = HeaderValue::from_str(uid).map_err(|_| GatewayError::NoTunnel)?;
        headers.insert(AGENT_HEADER, agent);
        Ok(GatewayRoute { address, headers })
    }
}

/// `<name>.<namespace>.svc` (the settings' form) as its name and namespace.
fn service_parts(service: &str) -> Option<(&str, &str)> {
    let mut labels = service.split('.');
    Some((labels.next()?, labels.next()?))
}

/// `ip:port`, bracketed for IPv6.
fn target(address: &str, port: u16) -> String {
    if address.contains(':') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

/// The address of each ready pod of `slices`, by pod name.
fn pod_addresses(slices: &[EndpointSlice]) -> HashMap<String, String> {
    slices
        .iter()
        .flat_map(|slice| &slice.endpoints)
        .filter(|endpoint| {
            endpoint
                .conditions
                .as_ref()
                .and_then(|conditions| conditions.ready)
                .unwrap_or(true)
        })
        .filter_map(|endpoint| {
            let pod = endpoint
                .target_ref
                .as_ref()
                .filter(|target| target.kind.as_deref() == Some("Pod"))?
                .name
                .clone()?;
            Some((pod, endpoint.addresses.first()?.clone()))
        })
        .collect()
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
    fn a_bootstrap_key_holds_the_token_hash_never_the_token() {
        let key = bootstrap_key("uid-1", "s3cret-token");
        assert!(
            key.starts_with("proxyk8sauth:agent-bootstrap:uid-1:"),
            "{key}"
        );
        assert!(!key.contains("s3cret"), "{key}");
        assert_eq!(key.rsplit(':').next().map(str::len), Some(64));
        assert_ne!(key, bootstrap_key("uid-2", "s3cret-token"));
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

    /// T19: a presence names a pod; only the ready pods of the gateway's
    /// Service resolve, to the address its EndpointSlice gives.
    #[test]
    fn a_route_names_a_gateway_pod_never_an_address() {
        let slice: EndpointSlice = serde_json::from_value(serde_json::json!({
            "addressType": "IPv4",
            "metadata": { "name": "gw-abcde" },
            "endpoints": [
                { "addresses": ["10.42.0.11"], "conditions": { "ready": true },
                  "targetRef": { "kind": "Pod", "name": "gw-0" } },
                { "addresses": ["10.42.0.12"], "conditions": { "ready": false },
                  "targetRef": { "kind": "Pod", "name": "gw-1" } },
                { "addresses": ["10.42.0.13"],
                  "targetRef": { "kind": "Node", "name": "gw-2" } },
            ],
        }))
        .unwrap();
        let by_pod = pod_addresses(&[slice]);
        assert_eq!(by_pod.len(), 1, "{by_pod:?}");
        assert_eq!(target(&by_pod["gw-0"], 8445), "10.42.0.11:8445");
        assert_eq!(target("fd00::1", 8445), "[fd00::1]:8445");
        assert_eq!(
            service_parts("gw-pods.proxyauth.svc"),
            Some(("gw-pods", "proxyauth"))
        );

        let route = client()
            .pick("aa", &["10.42.0.11:8445".to_string()])
            .unwrap();
        assert_eq!(route.address, "10.42.0.11:8445");
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
