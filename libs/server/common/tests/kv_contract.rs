//! The same cases against `Kv::Memory` and a real Redis (RFC 0015 §6.1): the
//! in-memory stand-in may only be trusted where the two agree.
//!
//! Real Redis comes from `TEST_REDIS_URL` (default `redis://127.0.0.1:6379`).
//! When it is unreachable the `redis::` cases print `SKIPPED:` and return,
//! unless `REQUIRE_TEST_REDIS` is set, which turns that into a failure. Every
//! key carries a per-run prefix and is deleted at the end; nothing here flushes
//! the database.

#![expect(
    clippy::expect_used,
    reason = "test helpers: a panic is a failed test, with its message"
)]

use std::sync::Mutex;
use std::time::Duration;

use common::kv::Kv;
use common::kv_memory::{InMemoryKv, KvOp};
use common::redis_pool::RedisPool;

struct Backend {
    kv: Kv,
    real: bool,
    prefix: String,
    keys: Mutex<Vec<String>>,
}

impl Backend {
    fn memory(case: &str) -> Self {
        Self::with(InMemoryKv::new().into(), false, case)
    }

    async fn redis(case: &str) -> Option<Self> {
        let url = std::env::var("TEST_REDIS_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let pool = RedisPool::from_url_with_mode(&url, false).expect("redis pool should build");
        let backend = Self::with(pool.into(), true, case);
        if let Err(error) = backend.kv.ping().await {
            common::test_skip::redis_unavailable(&url, &error.to_string());
            return None;
        }
        Some(backend)
    }

    fn with(kv: Kv, real: bool, case: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        Self {
            kv,
            real,
            prefix: format!("kv_contract:{case}:{}:{nanos}", std::process::id()),
            keys: Mutex::new(Vec::new()),
        }
    }

    fn key(&self, name: &str) -> String {
        let key = format!("{}:{name}", self.prefix);
        self.keys.lock().expect("key list").push(key.clone());
        key
    }

    /// Let `duration` pass, plus a second: Redis counts expiry in whole
    /// seconds on its own clock, so the paused clock of the in-memory cases
    /// moves by the same amount for both to agree.
    async fn elapse(&self, duration: Duration) {
        let duration = duration.saturating_add(Duration::from_secs(1));
        if self.real {
            tokio::time::sleep(duration).await;
        } else {
            tokio::time::advance(duration).await;
        }
    }

    async fn cleanup(&self) {
        let keys = self.keys.lock().expect("key list").clone();
        for key in keys {
            self.kv.del(&key).await.expect("cleanup DEL");
        }
    }
}

/// Generate `memory::<case>` and `redis::<case>` for every case.
macro_rules! contract {
    ($($case:ident),* $(,)?) => {
        mod memory {
            $(
                #[tokio::test(start_paused = true)]
                async fn $case() {
                    let backend = super::Backend::memory(stringify!($case));
                    super::$case(&backend).await;
                }
            )*
        }
        mod redis {
            $(
                #[tokio::test]
                async fn $case() {
                    let Some(backend) = super::Backend::redis(stringify!($case)).await else {
                        return;
                    };
                    super::$case(&backend).await;
                    backend.cleanup().await;
                }
            )*
        }
    };
}

contract!(
    ping_answers,
    an_absent_key_reads_as_none,
    a_value_reads_back,
    a_value_without_ttl_reports_minus_one,
    an_absent_key_reports_minus_two,
    a_value_expires_after_its_ttl,
    a_ttl_of_zero_is_refused,
    take_returns_the_value_once,
    take_is_atomic_under_concurrency,
    incr_with_ttl_starts_the_window_on_the_first_hit,
    incr_with_ttl_keeps_the_deadline,
    incr_with_ttl_heals_a_counter_without_ttl,
    incr_with_ttl_refuses_a_non_integer,
    exists_follows_the_key,
    deleting_an_absent_key_is_fine,
    a_set_lists_its_members_and_vanishes_when_empty,
    removing_an_absent_member_is_fine,
    a_presence_member_is_live_until_its_ttl,
    a_member_expiring_now_is_not_live,
    a_withdrawn_member_is_gone_at_once,
    a_presence_set_expires_after_twice_the_ttl,
    reading_a_set_as_a_string_is_refused,
);

async fn ping_answers(backend: &Backend) {
    backend.kv.ping().await.expect("PING");
}

async fn an_absent_key_reads_as_none(backend: &Backend) {
    assert_eq!(
        backend.kv.get(&backend.key("absent")).await.expect("GET"),
        None
    );
}

async fn a_value_reads_back(backend: &Backend) {
    let key = backend.key("value");
    backend.kv.set(&key, "v1", None).await.expect("SET");
    assert_eq!(
        backend.kv.get(&key).await.expect("GET").as_deref(),
        Some("v1")
    );
    backend.kv.set(&key, "v2", None).await.expect("SET");
    assert_eq!(
        backend.kv.get(&key).await.expect("GET").as_deref(),
        Some("v2")
    );
}

async fn a_value_without_ttl_reports_minus_one(backend: &Backend) {
    let key = backend.key("forever");
    backend.kv.set(&key, "v", None).await.expect("SET");
    assert_eq!(backend.kv.ttl(&key).await.expect("TTL"), -1);
}

async fn an_absent_key_reports_minus_two(backend: &Backend) {
    assert_eq!(
        backend.kv.ttl(&backend.key("absent")).await.expect("TTL"),
        -2
    );
}

async fn a_value_expires_after_its_ttl(backend: &Backend) {
    let key = backend.key("short");
    backend.kv.set(&key, "v", Some(2)).await.expect("SETEX");
    assert!((1..=2).contains(&backend.kv.ttl(&key).await.expect("TTL")));
    backend.elapse(Duration::from_secs(2)).await;
    assert_eq!(backend.kv.get(&key).await.expect("GET"), None);
    assert_eq!(backend.kv.ttl(&key).await.expect("TTL"), -2);
}

async fn a_ttl_of_zero_is_refused(backend: &Backend) {
    let key = backend.key("zero");
    assert!(backend.kv.set(&key, "v", Some(0)).await.is_err());
    assert_eq!(backend.kv.get(&key).await.expect("GET"), None);
}

async fn take_returns_the_value_once(backend: &Backend) {
    let key = backend.key("once");
    backend.kv.set(&key, "code", Some(60)).await.expect("SETEX");
    assert_eq!(
        backend.kv.take(&key).await.expect("GETDEL").as_deref(),
        Some("code")
    );
    assert_eq!(backend.kv.take(&key).await.expect("GETDEL"), None);
    assert_eq!(backend.kv.get(&key).await.expect("GET"), None);
}

/// RFC 0015 T2: a single-use value (authorization code, CSRF state) is handed
/// to exactly one of many concurrent readers.
async fn take_is_atomic_under_concurrency(backend: &Backend) {
    let key = backend.key("race");
    backend.kv.set(&key, "code", Some(60)).await.expect("SETEX");
    let readers: Vec<_> = (0..16)
        .map(|_| {
            let kv = backend.kv.clone();
            let key = key.clone();
            tokio::spawn(async move { kv.take(&key).await.expect("GETDEL") })
        })
        .collect();
    let mut winners = Vec::new();
    for reader in readers {
        winners.extend(reader.await.expect("reader task"));
    }
    assert_eq!(winners, ["code"]);
}

async fn incr_with_ttl_starts_the_window_on_the_first_hit(backend: &Backend) {
    let key = backend.key("window");
    assert_eq!(backend.kv.incr_with_ttl(&key, 10).await.expect("INCR"), 1);
    assert!((9..=10).contains(&backend.kv.ttl(&key).await.expect("TTL")));
    assert_eq!(backend.kv.incr_with_ttl(&key, 10).await.expect("INCR"), 2);
}

/// RFC 0015 T2: later hits must not push the window back, or a subject who
/// keeps failing would never be let out.
async fn incr_with_ttl_keeps_the_deadline(backend: &Backend) {
    let key = backend.key("deadline");
    backend.kv.incr_with_ttl(&key, 10).await.expect("INCR");
    backend.elapse(Duration::from_secs(2)).await;
    assert_eq!(backend.kv.incr_with_ttl(&key, 10).await.expect("INCR"), 2);
    assert!(backend.kv.ttl(&key).await.expect("TTL") <= 7);
}

/// RFC 0015 T2: a counter left without a TTL gets one on the next hit, so it
/// cannot lock a subject out forever.
async fn incr_with_ttl_heals_a_counter_without_ttl(backend: &Backend) {
    let key = backend.key("stuck");
    backend.kv.set(&key, "5", None).await.expect("SET");
    assert_eq!(backend.kv.incr_with_ttl(&key, 10).await.expect("INCR"), 6);
    assert!((9..=10).contains(&backend.kv.ttl(&key).await.expect("TTL")));
}

async fn incr_with_ttl_refuses_a_non_integer(backend: &Backend) {
    let key = backend.key("text");
    backend.kv.set(&key, "x", None).await.expect("SET");
    assert!(backend.kv.incr_with_ttl(&key, 10).await.is_err());
}

async fn exists_follows_the_key(backend: &Backend) {
    let key = backend.key("flag");
    assert!(!backend.kv.exists(&key).await.expect("EXISTS"));
    backend.kv.set(&key, "1", None).await.expect("SET");
    assert!(backend.kv.exists(&key).await.expect("EXISTS"));
    backend.kv.del(&key).await.expect("DEL");
    assert!(!backend.kv.exists(&key).await.expect("EXISTS"));
}

async fn deleting_an_absent_key_is_fine(backend: &Backend) {
    backend.kv.del(&backend.key("absent")).await.expect("DEL");
}

async fn a_set_lists_its_members_and_vanishes_when_empty(backend: &Backend) {
    let set = backend.key("index");
    backend.kv.set_add(&set, "b").await.expect("SADD");
    backend.kv.set_add(&set, "a").await.expect("SADD");
    backend.kv.set_add(&set, "a").await.expect("SADD");
    let mut members = backend.kv.set_members(&set).await.expect("SMEMBERS");
    members.sort();
    assert_eq!(members, ["a", "b"]);
    backend.kv.set_remove(&set, "a").await.expect("SREM");
    backend.kv.set_remove(&set, "b").await.expect("SREM");
    assert!(
        backend
            .kv
            .set_members(&set)
            .await
            .expect("SMEMBERS")
            .is_empty()
    );
    assert!(!backend.kv.exists(&set).await.expect("EXISTS"));
}

async fn removing_an_absent_member_is_fine(backend: &Backend) {
    let set = backend.key("index");
    backend.kv.set_remove(&set, "ghost").await.expect("SREM");
    backend.kv.set_add(&set, "a").await.expect("SADD");
    backend.kv.set_remove(&set, "ghost").await.expect("SREM");
    assert_eq!(backend.kv.set_members(&set).await.expect("SMEMBERS"), ["a"]);
}

async fn a_presence_member_is_live_until_its_ttl(backend: &Backend) {
    let key = backend.key("presence");
    backend
        .kv
        .presence_refresh(&key, "pod-a", 2)
        .await
        .expect("refresh");
    assert_eq!(
        backend.kv.presence_members(&key).await.expect("members"),
        ["pod-a"]
    );
    backend.elapse(Duration::from_secs(2)).await;
    assert!(
        backend
            .kv
            .presence_members(&key)
            .await
            .expect("members")
            .is_empty()
    );
}

/// The live bound is exclusive (`(now`): a member scored at `now` is gone.
async fn a_member_expiring_now_is_not_live(backend: &Backend) {
    let key = backend.key("presence");
    backend
        .kv
        .presence_refresh(&key, "pod-a", 0)
        .await
        .expect("refresh");
    assert!(
        backend
            .kv
            .presence_members(&key)
            .await
            .expect("members")
            .is_empty()
    );
}

async fn a_withdrawn_member_is_gone_at_once(backend: &Backend) {
    let key = backend.key("presence");
    backend
        .kv
        .presence_refresh(&key, "pod-a", 60)
        .await
        .expect("refresh");
    backend
        .kv
        .presence_refresh(&key, "pod-b", 60)
        .await
        .expect("refresh");
    backend
        .kv
        .presence_remove(&key, "pod-a")
        .await
        .expect("ZREM");
    assert_eq!(
        backend.kv.presence_members(&key).await.expect("members"),
        ["pod-b"]
    );
    backend
        .kv
        .presence_remove(&key, "ghost")
        .await
        .expect("ZREM");
}

async fn a_presence_set_expires_after_twice_the_ttl(backend: &Backend) {
    let key = backend.key("presence");
    backend
        .kv
        .presence_refresh(&key, "pod-a", 1)
        .await
        .expect("refresh");
    assert!(backend.kv.exists(&key).await.expect("EXISTS"));
    backend.elapse(Duration::from_secs(2)).await;
    assert!(!backend.kv.exists(&key).await.expect("EXISTS"));
}

async fn reading_a_set_as_a_string_is_refused(backend: &Backend) {
    let set = backend.key("index");
    backend.kv.set_add(&set, "a").await.expect("SADD");
    assert!(backend.kv.get(&set).await.is_err());
}

/// What a closed port cannot produce: one command failing while the store
/// answers the others (RFC 0015 decision 20).
#[tokio::test]
async fn memory_an_injected_failure_hits_only_its_operation_until_healed() {
    let memory = InMemoryKv::new();
    let kv: Kv = memory.clone().into();
    kv.set("k", "v", None).await.expect("SET");

    memory.fail_on(KvOp::Get);
    assert!(kv.get("k").await.is_err());
    assert!(kv.exists("k").await.expect("EXISTS"));

    memory.heal();
    assert_eq!(kv.get("k").await.expect("GET").as_deref(), Some("v"));
}
