//! Redis access on [`State`]: every command the API and controller send goes
//! through these methods, so key layout and atomicity rules live in one place.
//! The commands themselves are in [`crate::kv`].

use tracing::{info, instrument, warn};

use super::State;
use crate::redis_pool::RedisPoolError;
use crate::traits::ObjectRedis;

impl State {
    /// Whether Redis is reached in cluster mode.
    #[must_use]
    pub fn redis_is_cluster(&self) -> bool {
        self.kv.is_cluster()
    }

    /// Round-trip a `PING` to Redis, for the readiness probe.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when no connection can be obtained or the
    /// command fails.
    pub async fn redis_ping(&self) -> Result<(), RedisPoolError> {
        self.kv.ping().await
    }

    /// Read a string value.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn redis_get(&self, key: &str) -> Result<Option<String>, RedisPoolError> {
        self.kv.get(key).await
    }

    /// Read a string value and delete it in the same atomic step (`GETDEL`).
    ///
    /// Use it for single-use values (authorization codes, CSRF states): with a
    /// separate `GET` then `DEL`, two concurrent requests can both read the
    /// value before either deletes it.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn redis_take(&self, key: &str) -> Result<Option<String>, RedisPoolError> {
        self.kv.take(key).await
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
        self.kv.set(key, value, ttl_seconds).await
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
        let value = self.kv.incr_with_ttl(key, ttl_seconds).await?;
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
        self.kv.exists(key).await
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
        let ttl = self.kv.ttl(key).await?;
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
        self.kv.del(key).await
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
        self.kv.set_add(&index_key(prefix), key).await
    }

    /// Remove a key from the index of cached objects.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn index_remove(&self, prefix: &str, key: &str) -> Result<(), RedisPoolError> {
        self.kv.set_remove(&index_key(prefix), key).await
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
        let keys = self.kv.set_members(&index_key(prefix)).await?;

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
                    // Best effort: a stale index entry is retried on the next list.
                    if let Err(error) = self.index_remove(prefix, &key).await {
                        warn!(%error, key, "could not drop a stale index entry");
                    }
                }
            }
        }
        Ok(objects)
    }

    /// A cached `ProxyKubeApi` by `ns/name`, whichever prefix it is under
    /// (RFC 0014, decision 42): the trusted-mode key first, then the legacy one.
    ///
    /// # Errors
    ///
    /// As [`Self::get_object_from_redis`].
    pub async fn get_proxy_from_redis(
        &self,
        path: &str,
    ) -> Result<Option<crd::ProxyKubeApi>, RedisPoolError> {
        if let Some(proxy) = self
            .get_object_from_redis(crd::REDIS_TRUSTED_PREFIX, path)
            .await?
        {
            return Ok(Some(proxy));
        }
        self.get_object_from_redis(crd::REDIS_PREFIX, path).await
    }

    /// Every cached `ProxyKubeApi`, under both prefixes. A cluster caught
    /// mid-switch between them is listed once, from the trusted-mode key.
    ///
    /// # Errors
    ///
    /// As [`Self::list_objects`].
    pub async fn list_proxies(&self) -> Result<Vec<crd::ProxyKubeApi>, RedisPoolError> {
        let mut proxies: Vec<crd::ProxyKubeApi> =
            self.list_objects(crd::REDIS_TRUSTED_PREFIX).await?;
        let trusted: std::collections::HashSet<String> =
            proxies.iter().map(crd::ProxyKubeApi::to_path).collect();
        let legacy: Vec<crd::ProxyKubeApi> = self.list_objects(crd::REDIS_PREFIX).await?;
        proxies.extend(
            legacy
                .into_iter()
                .filter(|proxy| !trusted.contains(&proxy.to_path())),
        );
        Ok(proxies)
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

impl State {
    /// Mark `member` alive in the presence set `key` for `ttl_seconds`, and
    /// drop the members that expired. One sorted set per key, scored by
    /// expiry on Redis's own clock, so replicas with skewed clocks agree.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn presence_refresh(
        &self,
        key: &str,
        member: &str,
        ttl_seconds: u64,
    ) -> Result<(), RedisPoolError> {
        self.kv.presence_refresh(key, member, ttl_seconds).await
    }

    /// Withdraw `member` from the presence set `key` at once.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn presence_remove(&self, key: &str, member: &str) -> Result<(), RedisPoolError> {
        self.kv.presence_remove(key, member).await
    }

    /// The members of the presence set `key` that have not expired.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the Redis command fails.
    #[instrument(skip(self))]
    pub async fn presence_members(&self, key: &str) -> Result<Vec<String>, RedisPoolError> {
        self.kv.presence_members(key).await
    }
}

/// Key of the Set indexing every object cached under a prefix.
fn index_key(prefix: &str) -> String {
    format!("{prefix}:index")
}
