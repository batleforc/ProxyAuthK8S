//! The shared application state handed to every handler and to the controller.
//!
//! Construction lives here; the Redis access methods are in [`redis`].

use std::sync::Arc;

use jwt_validator::JwtValidator;
use kube::Client;
use tracing::{info, instrument};

use crate::{
    config, error::StateInitError, oidc_conf, oidc_config_cache::OidcConfigCache,
    redis_pool::RedisPool,
};

mod redis;

#[derive(Clone)]
pub struct State {
    pub client: Client,
    redis: RedisPool,
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
}

impl State {
    /// Boot the shared application state: connect to Redis, build the Kubernetes
    /// client, and confirm the OIDC provider is reachable.
    ///
    /// Settings come from the process-wide [`config::Config`].
    ///
    /// # Errors
    ///
    /// Returns [`StateInitError`] if the Redis pool cannot be built, the
    /// Kubernetes client cannot be created, or OIDC discovery fails.
    #[instrument(name = "StateInit")]
    pub async fn new() -> Result<Self, StateInitError> {
        let config = config::get();
        let pool = RedisPool::from_url(&config.redis.url)?;
        info!(cluster = pool.is_cluster(), "Connected to Redis");
        let client = Client::try_default().await?;
        info!("Connected to Kubernetes");
        let oidc_client = oidc_conf::OidcConf::new();
        oidc_client.oidc_core().await?;
        info!("OIDC discovery successful");
        Ok(Self {
            client,
            redis: pool,
            oidc_client,
            oidc_cluster_redirect_base_url: config.oidc.cluster_redirect_base_url.clone(),
            oidc_front_redirect_base_url: config.oidc.front_redirect_base_url.clone(),
            is_leader: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            lease_namespace: config.leader_election.lease_namespace.clone(),
            lease_name: config.leader_election.lease_name.clone(),
            jwt_validator: Arc::new(JwtValidator::new()),
            oidc_config_cache: Arc::new(OidcConfigCache::new()),
        })
    }

    /// Assemble a `State` from already-built parts.
    ///
    /// `State::new()` performs Kubernetes and OIDC discovery at boot, which a
    /// test cannot do; this lets a test point every dependency at a stub.
    #[cfg(feature = "test-util")]
    pub fn from_parts(
        client: Client,
        redis: RedisPool,
        oidc_client: oidc_conf::OidcConf,
        oidc_cluster_redirect_base_url: String,
        oidc_front_redirect_base_url: String,
    ) -> Self {
        Self {
            client,
            redis,
            oidc_client,
            oidc_cluster_redirect_base_url,
            oidc_front_redirect_base_url,
            is_leader: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            lease_namespace: "default".to_string(),
            lease_name: "test".to_string(),
            jwt_validator: Arc::new(JwtValidator::new()),
            oidc_config_cache: Arc::new(OidcConfigCache::new()),
        }
    }
}
