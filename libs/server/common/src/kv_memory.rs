//! An in-memory stand-in for Redis, compiled under the `test-util` feature only
//! (RFC 0015 §6.1).
//!
//! It copies what the server relies on, read from the commands and Lua scripts
//! of [`crate::kv`], not from the Redis documentation: `GETDEL`, the TTL rounding
//! of `TTL`, the self-healing `INCR` script, the presence scripts with their
//! exclusive `(now` bound. `tests/kv_contract.rs` runs the same cases against
//! a real Redis, so a divergence fails a test.
//!
//! Time is read from `tokio::time::Instant`, so `start_paused` and `advance`
//! move both the TTLs and the presence clock (which Redis reads from `TIME`).
//! [`InMemoryKv::fail_on`] makes one operation fail while the store stays up,
//! the case a closed port cannot produce.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use deadpool_redis::redis::{ErrorKind, RedisError};
use tokio::time::Instant;

use crate::redis_pool::RedisPoolError;

/// One operation of [`crate::kv::Kv`], to make it fail with [`InMemoryKv::fail_on`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KvOp {
    Ping,
    Get,
    Take,
    Set,
    Incr,
    Exists,
    Ttl,
    Del,
    SetAdd,
    SetRemove,
    SetMembers,
    PresenceRefresh,
    PresenceRemove,
    PresenceMembers,
}

enum Value {
    Str(String),
    Set(BTreeSet<String>),
    /// Member to score, the score being an expiry in Unix seconds.
    Scored(BTreeMap<String, u64>),
}

struct Entry {
    value: Value,
    expires_at: Option<Instant>,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<String, Entry>,
    failing: HashSet<KvOp>,
}

/// Cloned handles share one store, as clones of a Redis pool share one server.
#[derive(Clone)]
pub struct InMemoryKv {
    inner: Arc<Mutex<Inner>>,
    origin: Instant,
    origin_unix: u64,
}

impl Default for InMemoryKv {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryKv {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::default(),
            origin: Instant::now(),
            origin_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs()),
        }
    }

    /// Make every later `op` fail with [`RedisPoolError::Pool`], the store
    /// staying reachable for the others.
    pub fn fail_on(&self, op: KvOp) {
        self.lock().failing.insert(op);
    }

    /// Undo every [`Self::fail_on`].
    pub fn heal(&self) {
        self.lock().failing.clear();
    }

    pub(crate) fn ping(&self) -> Result<(), RedisPoolError> {
        self.begin(KvOp::Ping).map(|_| ())
    }

    pub(crate) fn get(&self, key: &str) -> Result<Option<String>, RedisPoolError> {
        let inner = self.begin(KvOp::Get)?;
        match inner.entries.get(key).map(|entry| &entry.value) {
            None => Ok(None),
            Some(Value::Str(value)) => Ok(Some(value.clone())),
            Some(_) => Err(wrong_type()),
        }
    }

    pub(crate) fn take(&self, key: &str) -> Result<Option<String>, RedisPoolError> {
        let mut inner = self.begin(KvOp::Take)?;
        match inner.entries.get(key).map(|entry| &entry.value) {
            None => Ok(None),
            Some(Value::Str(_)) => match inner.entries.remove(key).map(|entry| entry.value) {
                Some(Value::Str(value)) => Ok(Some(value)),
                _ => Ok(None),
            },
            Some(_) => Err(wrong_type()),
        }
    }

    pub(crate) fn set(
        &self,
        key: &str,
        value: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<(), RedisPoolError> {
        let mut inner = self.begin(KvOp::Set)?;
        let expires_at = match ttl_seconds {
            Some(0) => return Err(client_error("invalid expire time in 'setex' command")),
            Some(ttl) => deadline(ttl),
            None => None,
        };
        inner.entries.insert(
            key.to_string(),
            Entry {
                value: Value::Str(value.to_string()),
                expires_at,
            },
        );
        Ok(())
    }

    pub(crate) fn incr_with_ttl(&self, key: &str, ttl_seconds: i64) -> Result<i64, RedisPoolError> {
        let mut inner = self.begin(KvOp::Incr)?;
        let entry = inner.entries.entry(key.to_string()).or_insert(Entry {
            value: Value::Str("0".to_string()),
            expires_at: None,
        });
        let Value::Str(current) = &entry.value else {
            return Err(wrong_type());
        };
        let next = current
            .parse::<i64>()
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or_else(|| client_error("value is not an integer or out of range"))?;
        entry.value = Value::Str(next.to_string());
        if entry.expires_at.is_none() {
            // `EXPIRE` with a TTL of 0 or less deletes the key.
            match u64::try_from(ttl_seconds).ok().filter(|ttl| *ttl > 0) {
                Some(ttl) => entry.expires_at = deadline(ttl),
                None => {
                    inner.entries.remove(key);
                }
            }
        }
        Ok(next)
    }

    pub(crate) fn exists(&self, key: &str) -> Result<bool, RedisPoolError> {
        Ok(self.begin(KvOp::Exists)?.entries.contains_key(key))
    }

    pub(crate) fn ttl(&self, key: &str) -> Result<i64, RedisPoolError> {
        let inner = self.begin(KvOp::Ttl)?;
        let Some(entry) = inner.entries.get(key) else {
            return Ok(-2);
        };
        let Some(expires_at) = entry.expires_at else {
            return Ok(-1);
        };
        // Redis rounds the remaining milliseconds to the nearest second.
        let millis = expires_at
            .saturating_duration_since(Instant::now())
            .as_millis();
        let seconds = millis.saturating_add(500).checked_div(1000).unwrap_or(0);
        Ok(i64::try_from(seconds).unwrap_or(i64::MAX))
    }

    pub(crate) fn del(&self, key: &str) -> Result<(), RedisPoolError> {
        self.begin(KvOp::Del)?.entries.remove(key);
        Ok(())
    }

    pub(crate) fn set_add(&self, set: &str, member: &str) -> Result<(), RedisPoolError> {
        let mut inner = self.begin(KvOp::SetAdd)?;
        let entry = inner.entries.entry(set.to_string()).or_insert(Entry {
            value: Value::Set(BTreeSet::new()),
            expires_at: None,
        });
        let Value::Set(members) = &mut entry.value else {
            return Err(wrong_type());
        };
        members.insert(member.to_string());
        Ok(())
    }

    pub(crate) fn set_remove(&self, set: &str, member: &str) -> Result<(), RedisPoolError> {
        let mut inner = self.begin(KvOp::SetRemove)?;
        let Some(entry) = inner.entries.get_mut(set) else {
            return Ok(());
        };
        let Value::Set(members) = &mut entry.value else {
            return Err(wrong_type());
        };
        members.remove(member);
        if members.is_empty() {
            // Redis deletes a set when its last member goes.
            inner.entries.remove(set);
        }
        Ok(())
    }

    pub(crate) fn set_members(&self, set: &str) -> Result<Vec<String>, RedisPoolError> {
        let inner = self.begin(KvOp::SetMembers)?;
        match inner.entries.get(set).map(|entry| &entry.value) {
            None => Ok(Vec::new()),
            Some(Value::Set(members)) => Ok(members.iter().cloned().collect()),
            Some(_) => Err(wrong_type()),
        }
    }

    pub(crate) fn presence_refresh(
        &self,
        key: &str,
        member: &str,
        ttl_seconds: u64,
    ) -> Result<(), RedisPoolError> {
        let now = self.unix_now();
        let mut inner = self.begin(KvOp::PresenceRefresh)?;
        let entry = inner.entries.entry(key.to_string()).or_insert(Entry {
            value: Value::Scored(BTreeMap::new()),
            expires_at: None,
        });
        let Value::Scored(members) = &mut entry.value else {
            return Err(wrong_type());
        };
        members.insert(member.to_string(), now.saturating_add(ttl_seconds));
        members.retain(|_, expiry| *expiry > now);
        let keep_for = ttl_seconds.saturating_mul(2);
        if members.is_empty() || keep_for == 0 {
            inner.entries.remove(key);
        } else {
            entry.expires_at = deadline(keep_for);
        }
        Ok(())
    }

    pub(crate) fn presence_remove(&self, key: &str, member: &str) -> Result<(), RedisPoolError> {
        let mut inner = self.begin(KvOp::PresenceRemove)?;
        let Some(entry) = inner.entries.get_mut(key) else {
            return Ok(());
        };
        let Value::Scored(members) = &mut entry.value else {
            return Err(wrong_type());
        };
        members.remove(member);
        if members.is_empty() {
            inner.entries.remove(key);
        }
        Ok(())
    }

    pub(crate) fn presence_members(&self, key: &str) -> Result<Vec<String>, RedisPoolError> {
        let now = self.unix_now();
        let inner = self.begin(KvOp::PresenceMembers)?;
        match inner.entries.get(key).map(|entry| &entry.value) {
            None => Ok(Vec::new()),
            Some(Value::Scored(members)) => {
                // `ZRANGEBYSCORE (now +inf`: by score, then by member.
                let mut live: Vec<(u64, &String)> = members
                    .iter()
                    .filter(|(_, expiry)| **expiry > now)
                    .map(|(member, expiry)| (*expiry, member))
                    .collect();
                live.sort();
                Ok(live.into_iter().map(|(_, member)| member.clone()).collect())
            }
            Some(_) => Err(wrong_type()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Fail if `op` is set to, else drop the expired keys (Redis keeps a key
    /// until its deadline has passed, not when it is reached) and hand the store.
    fn begin(&self, op: KvOp) -> Result<MutexGuard<'_, Inner>, RedisPoolError> {
        let mut inner = self.lock();
        if inner.failing.contains(&op) {
            return Err(RedisPoolError::Pool(format!("injected failure on {op:?}")));
        }
        let now = Instant::now();
        inner
            .entries
            .retain(|_, entry| entry.expires_at.is_none_or(|at| at >= now));
        Ok(inner)
    }

    /// Unix seconds on the same clock as the TTLs, as Redis's `TIME`.
    fn unix_now(&self) -> u64 {
        let elapsed = Instant::now().saturating_duration_since(self.origin);
        self.origin_unix.saturating_add(elapsed.as_secs())
    }
}

fn deadline(ttl_seconds: u64) -> Option<Instant> {
    Instant::now().checked_add(Duration::from_secs(ttl_seconds))
}

fn wrong_type() -> RedisPoolError {
    RedisPoolError::Command(RedisError::from((
        ErrorKind::UnexpectedReturnType,
        "WRONGTYPE Operation against a key holding the wrong kind of value",
    )))
}

fn client_error(message: &'static str) -> RedisPoolError {
    RedisPoolError::Command(RedisError::from((ErrorKind::Client, message)))
}
