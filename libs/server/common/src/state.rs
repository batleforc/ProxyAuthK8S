//! The shared application state handed to every handler and to the controller.
//!
//! Construction lives here; the Redis access methods are in [`redis`].

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use jwt_validator::JwtValidator;
use kube::Client;
use kube::runtime::events::Recorder;
use tracing::{info, instrument, warn};
use weebo_kit_runtime::Health;

use crate::{
    config, error::StateInitError, kv::Kv, oidc_conf, oidc_config_cache::OidcConfigCache,
    redis_pool::RedisPool,
};

mod redis;

/// First delay between two boot-time OIDC discovery attempts.
pub const OIDC_DISCOVERY_RETRY_BASE: Duration = Duration::from_secs(1);
/// Longest delay between two boot-time OIDC discovery attempts.
pub const OIDC_DISCOVERY_RETRY_MAX: Duration = Duration::from_secs(60);

/// Delay before discovery attempt `attempt + 1` (`attempt` is 1-based): `base`
/// doubled per failure, capped at `max`, never overflowing.
#[must_use]
pub fn oidc_discovery_retry_delay(attempt: u32, base: Duration, max: Duration) -> Duration {
    let factor = 2u32.saturating_pow(attempt.saturating_sub(1));
    base.saturating_mul(factor).min(max)
}

#[derive(Clone)]
pub struct State {
    pub client: Client,
    kv: Kv,
    pub oidc_client: oidc_conf::OidcConf,
    pub oidc_cluster_redirect_base_url: String,
    pub oidc_front_redirect_base_url: String,
    pub is_leader: Arc<std::sync::atomic::AtomicBool>,
    pub lease_namespace: String,
    pub lease_name: String,
    /// Shared across every request so the JWKS cache and the per-issuer HTTP
    /// clients are process-wide: one per request would refetch the issuer's keys
    /// on every call.
    pub jwt_validator: Arc<JwtValidator>,
    /// Short-lived cache for `oidc_provider.config_from`, so resolving it does
    /// not put an apiserver Secret GET behind every proxied request.
    pub oidc_config_cache: Arc<OidcConfigCache>,
    /// Set once the service-wide OIDC provider has been discovered. Until then
    /// `/management/ready` answers 503 (see [`State::discover_oidc_until_ready`]).
    pub oidc_ready: Arc<AtomicBool>,
    /// The controller's Event recorder, built on first use: one per process,
    /// so kube folds a repeated Event into a series instead of creating a new
    /// one each time (RFC 0011).
    pub event_recorder: Arc<OnceLock<Recorder>>,
    /// The controller's watch: `controller::run` reports its failures here,
    /// `/management/ready` shows them without failing on them (RFC 0011,
    /// decision 26).
    pub controller_health: Arc<Health>,
    /// Where the `Agent` clusters' gateway pods are (RFC 0012, decision 26).
    pub agent_gateway: Arc<crate::agent_gateway::GatewayClient>,
}

impl State {
    /// Boot the shared application state: connect to Redis and build the
    /// Kubernetes client.
    ///
    /// OIDC discovery is *not* done here: an IdP outage must not keep the pod
    /// from starting. Run [`State::discover_oidc_until_ready`] next to the
    /// server; the pod reports not-ready until it succeeds.
    ///
    /// Settings come from the process-wide [`config::Config`].
    ///
    /// # Errors
    ///
    /// Returns [`StateInitError`] if the Redis pool cannot be built or the
    /// Kubernetes client cannot be created.
    #[instrument(name = "StateInit")]
    pub async fn new() -> Result<Self, StateInitError> {
        let config = config::get();
        let pool = RedisPool::from_url(&config.redis.url)?;
        info!(cluster = pool.is_cluster(), "Connected to Redis");
        let client = Client::try_default().await?;
        info!("Connected to Kubernetes");
        let oidc_client = oidc_conf::OidcConf::new();
        Ok(Self {
            client,
            kv: Kv::Redis(pool),
            oidc_client,
            oidc_cluster_redirect_base_url: config.oidc.cluster_redirect_base_url.clone(),
            oidc_front_redirect_base_url: config.oidc.front_redirect_base_url.clone(),
            is_leader: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            lease_namespace: config.leader_election.lease_namespace.clone(),
            lease_name: config.leader_election.lease_name.clone(),
            jwt_validator: Arc::new(JwtValidator::with_max_document_bytes(
                config.proxy.jwks_max_document_bytes,
            )),
            oidc_config_cache: Arc::new(OidcConfigCache::new()),
            oidc_ready: Arc::new(AtomicBool::new(false)),
            event_recorder: Arc::default(),
            controller_health: Arc::default(),
            agent_gateway: Arc::new(crate::agent_gateway::GatewayClient::new(
                &config.agent_gateway,
            )),
        })
    }

    /// Whether the service-wide OIDC provider has been discovered.
    #[must_use]
    pub fn is_oidc_ready(&self) -> bool {
        self.oidc_ready.load(Ordering::Acquire)
    }

    /// Retry OIDC discovery with exponential backoff (`base` doubled per
    /// failure, capped at `max`) until it succeeds, then mark the state ready.
    ///
    /// Never gives up: a pod whose IdP is down stays alive and not-ready, so it
    /// recovers on its own once the IdP is back instead of crash-looping.
    #[instrument(name = "OidcDiscovery", skip(self))]
    pub async fn discover_oidc_until_ready(&self, base: Duration, max: Duration) {
        let mut attempt: u32 = 0;
        loop {
            attempt = attempt.saturating_add(1);
            match self.oidc_client.oidc_core().await {
                Ok(_) => {
                    self.oidc_ready.store(true, Ordering::Release);
                    info!(attempt, "OIDC discovery successful");
                    return;
                }
                Err(err) => {
                    let delay = oidc_discovery_retry_delay(attempt, base, max);
                    warn!(
                        attempt,
                        retry_in_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                        "OIDC discovery failed, pod stays not-ready: {err}"
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    /// Assemble a `State` from already-built parts.
    ///
    /// `State::new()` performs Kubernetes and OIDC discovery at boot, which a
    /// test cannot do; this lets a test point every dependency at a stub. `kv`
    /// is a [`RedisPool`] or a [`crate::kv_memory::InMemoryKv`].
    #[cfg(feature = "test-util")]
    pub fn from_parts(
        client: Client,
        kv: impl Into<Kv>,
        oidc_client: oidc_conf::OidcConf,
        oidc_cluster_redirect_base_url: String,
        oidc_front_redirect_base_url: String,
    ) -> Self {
        Self {
            client,
            kv: kv.into(),
            oidc_client,
            oidc_cluster_redirect_base_url,
            oidc_front_redirect_base_url,
            is_leader: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            lease_namespace: "default".to_string(),
            lease_name: "test".to_string(),
            jwt_validator: Arc::new(JwtValidator::new()),
            oidc_config_cache: Arc::new(OidcConfigCache::new()),
            // Tests drive handlers directly; they opt into the not-ready path.
            oidc_ready: Arc::new(AtomicBool::new(true)),
            event_recorder: Arc::default(),
            controller_health: Arc::default(),
            agent_gateway: Arc::new(crate::agent_gateway::GatewayClient::disabled()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(60);

    #[test]
    fn first_retry_uses_the_base_delay() {
        assert_eq!(oidc_discovery_retry_delay(1, BASE, MAX), BASE);
        // Attempt 0 is treated as the first one rather than underflowing.
        assert_eq!(oidc_discovery_retry_delay(0, BASE, MAX), BASE);
    }

    #[test]
    fn delay_doubles_then_caps() {
        let delays: Vec<u64> = (1..=8)
            .map(|attempt| oidc_discovery_retry_delay(attempt, BASE, MAX).as_secs())
            .collect();
        assert_eq!(delays, vec![1, 2, 4, 8, 16, 32, 60, 60]);
    }

    #[test]
    fn delay_never_overflows() {
        assert_eq!(oidc_discovery_retry_delay(u32::MAX, BASE, MAX), MAX);
    }
}
