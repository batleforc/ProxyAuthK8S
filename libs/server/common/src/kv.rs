//! The key-value store behind [`crate::State`]: Redis in production, an
//! in-memory stand-in under the `test-util` feature (RFC 0015 §5.1).
//!
//! Every command the server sends is one method here, so the stand-in has a
//! closed list of behaviours to copy and `tests/kv_contract.rs` runs the same
//! cases against both.

use deadpool_redis::redis;

use crate::redis_pool::{RedisPool, RedisPoolError};

#[cfg(feature = "test-util")]
use crate::kv_memory::InMemoryKv;

#[derive(Clone)]
pub enum Kv {
    Redis(RedisPool),
    /// Never built by `State::new`, and absent from the release build: the
    /// `test-util` feature is enabled from dev-dependencies only (RFC 0015 T1).
    #[cfg(feature = "test-util")]
    Memory(InMemoryKv),
}

impl From<RedisPool> for Kv {
    fn from(pool: RedisPool) -> Self {
        Self::Redis(pool)
    }
}

#[cfg(feature = "test-util")]
impl From<InMemoryKv> for Kv {
    fn from(memory: InMemoryKv) -> Self {
        Self::Memory(memory)
    }
}

/// `INCR`, then `EXPIRE` whenever the key has no TTL: the window starts with
/// the first hit, and a counter left without one heals on the next.
const INCR_WITH_TTL: &str = r"
    local v = redis.call('INCR', KEYS[1])
    if redis.call('TTL', KEYS[1]) < 0 then
        redis.call('EXPIRE', KEYS[1], ARGV[1])
    end
    return v
";

/// Scored by expiry on Redis's own clock, so replicas with skewed clocks agree.
const PRESENCE_REFRESH: &str = r"
    local now = tonumber(redis.call('TIME')[1])
    redis.call('ZADD', KEYS[1], now + tonumber(ARGV[2]), ARGV[1])
    redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now)
    redis.call('EXPIRE', KEYS[1], tonumber(ARGV[2]) * 2)
    return 1
";

const PRESENCE_LIVE: &str = r"
    local now = tonumber(redis.call('TIME')[1])
    return redis.call('ZRANGEBYSCORE', KEYS[1], '(' .. now, '+inf')
";

impl Kv {
    #[must_use]
    pub fn is_cluster(&self) -> bool {
        match self {
            Self::Redis(pool) => pool.is_cluster(),
            #[cfg(feature = "test-util")]
            Self::Memory(_) => false,
        }
    }

    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the store cannot be reached.
    pub async fn ping(&self) -> Result<(), RedisPoolError> {
        match self {
            Self::Redis(pool) => pool.query::<String>(&redis::cmd("PING")).await.map(|_| ()),
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.ping(),
        }
    }

    /// `GET`.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails.
    pub async fn get(&self, key: &str) -> Result<Option<String>, RedisPoolError> {
        match self {
            Self::Redis(pool) => pool.query(redis::cmd("GET").arg(key)).await,
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.get(key),
        }
    }

    /// `GETDEL`: read and delete in one atomic step.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails.
    pub async fn take(&self, key: &str) -> Result<Option<String>, RedisPoolError> {
        match self {
            Self::Redis(pool) => pool.query(redis::cmd("GETDEL").arg(key)).await,
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.take(key),
        }
    }

    /// `SETEX` with a TTL in seconds, `SET` without one.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails, including for a TTL
    /// of 0, which Redis refuses.
    pub async fn set(
        &self,
        key: &str,
        value: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<(), RedisPoolError> {
        match self {
            Self::Redis(pool) => match ttl_seconds {
                Some(ttl) => {
                    pool.query::<()>(redis::cmd("SETEX").arg(key).arg(ttl).arg(value))
                        .await
                }
                None => {
                    pool.query::<()>(redis::cmd("SET").arg(key).arg(value))
                        .await
                }
            },
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.set(key, value, ttl_seconds),
        }
    }

    /// `INCR` and, when the key has no TTL, `EXPIRE`, as one atomic script.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails.
    pub async fn incr_with_ttl(&self, key: &str, ttl_seconds: i64) -> Result<i64, RedisPoolError> {
        match self {
            Self::Redis(pool) => {
                pool.query(
                    redis::cmd("EVAL")
                        .arg(INCR_WITH_TTL)
                        .arg(1)
                        .arg(key)
                        .arg(ttl_seconds),
                )
                .await
            }
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.incr_with_ttl(key, ttl_seconds),
        }
    }

    /// `EXISTS`.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails.
    pub async fn exists(&self, key: &str) -> Result<bool, RedisPoolError> {
        match self {
            Self::Redis(pool) => {
                let existing: i64 = pool.query(redis::cmd("EXISTS").arg(key)).await?;
                Ok(existing > 0)
            }
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.exists(key),
        }
    }

    /// `TTL`, as Redis answers it: -2 for no key, -1 for no expiry, else the
    /// remaining seconds, rounded.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails.
    pub async fn ttl(&self, key: &str) -> Result<i64, RedisPoolError> {
        match self {
            Self::Redis(pool) => pool.query(redis::cmd("TTL").arg(key)).await,
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.ttl(key),
        }
    }

    /// `DEL`, whether or not the key exists.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails.
    pub async fn del(&self, key: &str) -> Result<(), RedisPoolError> {
        match self {
            Self::Redis(pool) => pool.query::<()>(redis::cmd("DEL").arg(key)).await,
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.del(key),
        }
    }

    /// `SADD`.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails.
    pub async fn set_add(&self, set: &str, member: &str) -> Result<(), RedisPoolError> {
        match self {
            Self::Redis(pool) => {
                pool.query::<()>(redis::cmd("SADD").arg(set).arg(member))
                    .await
            }
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.set_add(set, member),
        }
    }

    /// `SREM`.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails.
    pub async fn set_remove(&self, set: &str, member: &str) -> Result<(), RedisPoolError> {
        match self {
            Self::Redis(pool) => {
                pool.query::<()>(redis::cmd("SREM").arg(set).arg(member))
                    .await
            }
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.set_remove(set, member),
        }
    }

    /// `SMEMBERS`, in no particular order.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails.
    pub async fn set_members(&self, set: &str) -> Result<Vec<String>, RedisPoolError> {
        match self {
            Self::Redis(pool) => pool.query(redis::cmd("SMEMBERS").arg(set)).await,
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.set_members(set),
        }
    }

    /// Mark `member` alive in the sorted set `key` for `ttl_seconds`, drop the
    /// expired members and keep the set for twice the TTL.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails.
    pub async fn presence_refresh(
        &self,
        key: &str,
        member: &str,
        ttl_seconds: u64,
    ) -> Result<(), RedisPoolError> {
        match self {
            Self::Redis(pool) => pool
                .query::<i64>(
                    redis::cmd("EVAL")
                        .arg(PRESENCE_REFRESH)
                        .arg(1)
                        .arg(key)
                        .arg(member)
                        .arg(ttl_seconds),
                )
                .await
                .map(|_| ()),
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.presence_refresh(key, member, ttl_seconds),
        }
    }

    /// `ZREM`.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails.
    pub async fn presence_remove(&self, key: &str, member: &str) -> Result<(), RedisPoolError> {
        match self {
            Self::Redis(pool) => pool
                .query::<i64>(redis::cmd("ZREM").arg(key).arg(member))
                .await
                .map(|_| ()),
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.presence_remove(key, member),
        }
    }

    /// The members of `key` whose expiry is strictly after now.
    ///
    /// # Errors
    ///
    /// Returns [`RedisPoolError`] when the command fails.
    pub async fn presence_members(&self, key: &str) -> Result<Vec<String>, RedisPoolError> {
        match self {
            Self::Redis(pool) => {
                pool.query(redis::cmd("EVAL").arg(PRESENCE_LIVE).arg(1).arg(key))
                    .await
            }
            #[cfg(feature = "test-util")]
            Self::Memory(memory) => memory.presence_members(key),
        }
    }
}
