use std::{env, sync::Arc};

use kube::Client;
use rustls::pki_types::{pem::PemObject as _, CertificateDer, PrivateKeyDer};
use tracing::{info, instrument};

use deadpool_redis::redis;

use crate::redis_pool::{RedisPool, RedisPoolError};
use crate::traits::ObjectRedis;

pub mod oidc_conf;
pub mod oidc_error;
pub mod redis_pool;
pub mod token_audience;
pub mod traits;

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
}

/// Everything that can stop [`State::new`] from booting.
#[derive(Debug, thiserror::Error)]
pub enum StateInitError {
    #[error("failed to create the Redis pool: {0}")]
    Redis(#[from] RedisPoolError),
    #[error("failed to create the Kubernetes client: {0}")]
    Kube(#[from] kube::Error),
    #[error("OIDC discovery failed: {0}")]
    Oidc(#[from] oidc_error::OidcError),
}

impl State {
    /// Boot the shared application state: connect to Redis, build the Kubernetes
    /// client, and confirm the OIDC provider is reachable.
    ///
    /// # Errors
    ///
    /// Returns [`StateInitError`] if the Redis pool cannot be built, the
    /// Kubernetes client cannot be created, or OIDC discovery fails.
    #[instrument(name = "StateInit")]
    pub async fn new() -> Result<Self, StateInitError> {
        let redis_url = env::var("REDIS_URL").unwrap_or("redis://127.0.0.1:6379".to_string());
        let pool = RedisPool::from_url(&redis_url)?;
        info!(cluster = pool.is_cluster(), "Connected to Redis");
        let client = Client::try_default().await?;
        info!("Connected to Kubernetes");
        let oidc_client = oidc_conf::OidcConf::new();
        oidc_client.oidc_core().await?;
        info!("OIDC discovery successful");
        let oidc_cluster_redirect_base_url = env::var("API_CLUSTER_OIDC_BASE_REDIRECT_URL")
            .unwrap_or("https://localhost:5437".to_string());
        let oidc_front_redirect_base_url = env::var("API_CLUSTER_OIDC_FRONT_REDIRECT_URL")
            .unwrap_or("https://localhost:4200/auth/callback/".to_string());
        let lease_namespace = env::var("LEASE_NAMESPACE").unwrap_or("default".to_string());
        let lease_name = env::var("HOSTNAME").unwrap_or("NOT_A_POD".to_string());
        Ok(Self {
            client,
            redis: pool,
            oidc_client,
            oidc_cluster_redirect_base_url,
            oidc_front_redirect_base_url,
            is_leader: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            lease_namespace,
            lease_name,
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
        }
    }

    /// Whether Redis is reached in cluster mode.
    #[must_use]
    pub fn redis_is_cluster(&self) -> bool {
        self.redis.is_cluster()
    }

    /// Read a string value.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn redis_get(&self, key: &str) -> Result<Option<String>, RedisPoolError> {
        self.redis.query(redis::cmd("GET").arg(key)).await
    }

    /// Write a string value, with an optional TTL in seconds.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self, value))]
    pub async fn redis_set(
        &self,
        key: &str,
        value: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<(), RedisPoolError> {
        match ttl_seconds {
            Some(ttl) => {
                self.redis
                    .query::<()>(redis::cmd("SETEX").arg(key).arg(ttl).arg(value))
                    .await
            }
            None => {
                self.redis
                    .query::<()>(redis::cmd("SET").arg(key).arg(value))
                    .await
            }
        }
    }

    /// Increment a counter, giving it a TTL the first time it appears.
    ///
    /// This is the counter behind rate limiting and fail2login: the TTL is set
    /// on the transition from absent to 1, so the window starts with the first
    /// request and the key cannot outlive it.
    ///
    /// INCR and EXPIRE run in a single atomic Lua script. Doing EXPIRE as a
    /// second round-trip left a window where a transient failure (or a crash)
    /// between the two would leave the counter with no TTL — it would then
    /// accumulate forever and never reset, permanently locking the subject out.
    /// The script also re-arms the TTL whenever the key somehow has none
    /// (`TTL < 0`), so a previously stuck counter self-heals.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn incr_with_ttl(&self, key: &str, ttl_seconds: i64) -> Result<u64, RedisPoolError> {
        const INCR_WITH_TTL: &str = r"
            local v = redis.call('INCR', KEYS[1])
            if redis.call('TTL', KEYS[1]) < 0 then
                redis.call('EXPIRE', KEYS[1], ARGV[1])
            end
            return v
        ";
        let value: i64 = self
            .redis
            .query(
                redis::cmd("EVAL")
                    .arg(INCR_WITH_TTL)
                    .arg(1)
                    .arg(key)
                    .arg(ttl_seconds),
            )
            .await?;
        // INCR never returns a negative count; clamp defensively and convert
        // without a lossy sign cast.
        Ok(u64::try_from(value).unwrap_or(0))
    }

    /// Whether a key currently exists.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn key_exists(&self, key: &str) -> Result<bool, RedisPoolError> {
        let existing: i64 = self.redis.query(redis::cmd("EXISTS").arg(key)).await?;
        Ok(existing > 0)
    }

    /// Remaining TTL of a key, `None` when it has none or does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn key_ttl(&self, key: &str) -> Result<Option<u64>, RedisPoolError> {
        // Redis answers -1 for "no expiry" and -2 for "no such key"; both mean
        // there is no delay to report.
        let ttl: i64 = self.redis.query(redis::cmd("TTL").arg(key)).await?;
        Ok(u64::try_from(ttl).ok())
    }

    /// Set a marker key. A `ttl_seconds` of 0 means it never expires.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn set_flag(&self, key: &str, ttl_seconds: u64) -> Result<(), RedisPoolError> {
        let ttl = (ttl_seconds > 0).then_some(ttl_seconds);
        self.redis_set(key, "1", ttl).await
    }

    /// Delete a key, whether or not it exists.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn delete_key(&self, key: &str) -> Result<(), RedisPoolError> {
        self.redis.query::<()>(redis::cmd("DEL").arg(key)).await
    }

    /// Add a key to the index of cached objects.
    ///
    /// The index exists because `KEYS` cannot be used in cluster mode (it only
    /// answers for the node it reached) and is an O(N) blocking scan even in
    /// single-node mode. The index is a plain Set the controller keeps in sync.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn index_add(&self, prefix: &str, key: &str) -> Result<(), RedisPoolError> {
        self.redis
            .query::<()>(redis::cmd("SADD").arg(index_key(prefix)).arg(key))
            .await
    }

    /// Remove a key from the index of cached objects.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn index_remove(&self, prefix: &str, key: &str) -> Result<(), RedisPoolError> {
        self.redis
            .query::<()>(redis::cmd("SREM").arg(index_key(prefix)).arg(key))
            .await
    }

    /// Every object cached under `prefix`.
    ///
    /// Members are read one by one rather than with `MGET`: in cluster mode the
    /// keys are spread over several slots and a multi-key read across slots is
    /// rejected. An index entry whose object is gone is skipped and pruned.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when a Redis command fails.
    #[instrument(skip(self))]
    pub async fn list_objects<T: ObjectRedis>(
        &self,
        prefix: &str,
    ) -> Result<Vec<T>, RedisPoolError> {
        let keys: Vec<String> = self
            .redis
            .query(redis::cmd("SMEMBERS").arg(index_key(prefix)))
            .await?;

        let mut objects = Vec::with_capacity(keys.len());
        for key in keys {
            match self.redis_get(&key).await? {
                Some(json) => {
                    if let Some(object) = T::from_json(&json) {
                        objects.push(object);
                    }
                }
                None => {
                    // The object went away without the index being updated.
                    let _ = self.index_remove(prefix, &key).await;
                }
            }
        }
        Ok(objects)
    }

    /// Fetch and deserialize a single cached object by prefix and key.
    ///
    /// `Ok(None)` means the key is absent. A value that exists but cannot be
    /// deserialized is a distinct failure ([`RedisPoolError::Deserialize`]), not
    /// silently treated as absent, so a corrupt cache entry cannot masquerade as
    /// a missing object.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails, or
    /// [`RedisPoolError::Deserialize`] when a stored value cannot be decoded.
    #[instrument(skip(self))]
    pub async fn get_object_from_redis<T: ObjectRedis>(
        &self,
        prefix: &str,
        key: &str,
    ) -> Result<Option<T>, RedisPoolError> {
        let full_key = format!("{prefix}:{key}");
        let Some(obj_json) = self.redis_get(&full_key).await? else {
            info!("Object not found in Redis with key {}", full_key);
            return Ok(None);
        };
        info!("Object found in Redis with key {}", full_key);
        T::from_json(&obj_json)
            .map(Some)
            .ok_or_else(|| RedisPoolError::Deserialize(full_key))
    }
}

/// Key of the Set indexing every object cached under a prefix.
fn index_key(prefix: &str) -> String {
    format!("{prefix}:index")
}

#[derive(Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub https: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerConfig {
    #[must_use]
    pub fn new() -> Self {
        let port = env::var("SERVER_PORT")
            .unwrap_or("5437".to_string())
            .parse()
            .unwrap_or(5437);
        let https = env::var("SERVER_HTTPS")
            .unwrap_or("false".to_string())
            .parse()
            .unwrap_or(false);
        let cert_path = env::var("SERVER_CERT_PATH").ok();
        let key_path = env::var("SERVER_KEY_PATH").ok();
        Self {
            port,
            https,
            cert_path,
            key_path,
        }
    }

    /// Build the rustls server configuration from the configured certificate and
    /// private-key files.
    ///
    /// # Errors
    ///
    /// Returns [`TlsConfigError`] when HTTPS is enabled but a path is missing, a
    /// PEM file cannot be read, or rustls rejects the certificate/key pair.
    pub fn rustls_config(&self) -> Result<rustls::ServerConfig, TlsConfigError> {
        let cert_path = self
            .cert_path
            .as_ref()
            .ok_or(TlsConfigError::MissingPath("SERVER_CERT_PATH"))?;
        let key_path = self
            .key_path
            .as_ref()
            .ok_or(TlsConfigError::MissingPath("SERVER_KEY_PATH"))?;

        let cert_chain = CertificateDer::pem_file_iter(cert_path)
            .map_err(|err| TlsConfigError::Cert(err.to_string()))?
            .flatten()
            .collect();
        let key_der = PrivateKeyDer::from_pem_file(key_path)
            .map_err(|err| TlsConfigError::Key(err.to_string()))?;
        Ok(rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, key_der)?)
    }
}

/// Everything that can stop [`ServerConfig::rustls_config`] from producing a TLS
/// configuration.
#[derive(Debug, thiserror::Error)]
pub enum TlsConfigError {
    #[error("HTTPS is enabled but {0} is not set")]
    MissingPath(&'static str),
    #[error("failed to load the certificate chain: {0}")]
    Cert(String),
    #[error("failed to load the private key: {0}")]
    Key(String),
    #[error("rustls rejected the certificate/key pair: {0}")]
    Build(#[from] rustls::Error),
}
