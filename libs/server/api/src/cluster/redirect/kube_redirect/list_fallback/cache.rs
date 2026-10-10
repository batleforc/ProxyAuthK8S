//! The short-TTL cache for a resolved allow/deny set, and the per-key locks
//! that stop concurrent cache misses from all paying for the same resolution.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use actix_web::web;
use common::State;
use crd::ProxyKubeApi;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;
use tracing::warn;

use crate::cluster::redirect::kube_redirect::list_fallback::{
    review::RulesOutcome, tuning::cache_ttl_seconds,
};
use crate::model::user::User;

/// Deterministic, fixed-length identity for a cache key: the raw username and
/// groups could contain anything a client sends, so hash rather than embed
/// them directly.
pub(super) fn identity_fingerprint(user: Option<&User>) -> String {
    let mut hasher = Sha256::new();
    match user {
        Some(user) => {
            hasher.update(user.username.as_bytes());
            let mut groups: Vec<&str> = user.groups.iter().map(String::as_str).collect();
            groups.sort_unstable();
            for group in groups {
                hasher.update(b"\0");
                hasher.update(group.as_bytes());
            }
        }
        None => hasher.update(b"anonymous"),
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A cache/lock key scoped to `(proxy, caller identity, probe)` — including
/// the probe matters as soon as more than one mapper shares this module's
/// caching path, so two unrelated (resource, verb) checks for the same caller
/// never collide on the same entry.
pub(super) fn cache_key(
    proxy: &ProxyKubeApi,
    user: Option<&User>,
    probe: virtual_api::AccessProbe,
) -> String {
    format!(
        "proxyk8sauth:listfallback:{}:{}:{}:{}:{}",
        proxy.to_path(),
        identity_fingerprint(user),
        probe.group,
        probe.resource,
        probe.verb,
    )
}

pub(super) async fn load_cached_outcome(
    state: &web::Data<State>,
    proxy: &ProxyKubeApi,
    user: Option<&User>,
    probe: virtual_api::AccessProbe,
) -> Option<RulesOutcome> {
    if cache_ttl_seconds() == 0 {
        return None;
    }
    let raw = state
        .redis_get(&cache_key(proxy, user, probe))
        .await
        .ok()??;
    serde_json::from_str(&raw).ok()
}

pub(super) async fn store_cached_outcome(
    state: &web::Data<State>,
    proxy: &ProxyKubeApi,
    user: Option<&User>,
    probe: virtual_api::AccessProbe,
    outcome: &RulesOutcome,
) {
    let ttl = cache_ttl_seconds();
    if ttl == 0 {
        return;
    }
    let Ok(raw) = serde_json::to_string(outcome) else {
        return;
    };
    if let Err(err) = state
        .redis_set(&cache_key(proxy, user, probe), &raw, Some(ttl))
        .await
    {
        warn!(%err, "could not cache the resolved namespace allow-set");
    }
}

/// Per-key async locks deduplicating concurrent full-resolution work (rules
/// review, per-item checks, and the store) for the same cache key. An entry
/// is removed as soon as nothing else references it (see
/// [`release_outcome_lock`]), so this does not grow without bound over the
/// process lifetime.
///
/// This only dedupes within one process: the cache itself is shared (Redis)
/// across replicas, but this lock is not, so two different pods can still
/// each resolve the same cold key concurrently. Deduplicating across pods
/// too would need a distributed lock instead of this in-memory one.
static OUTCOME_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();

pub(super) fn acquire_outcome_lock(key: &str) -> Arc<AsyncMutex<()>> {
    let registry = OUTCOME_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut map = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    map.entry(key.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

/// Drop `key`'s entry once nothing else still holds a clone of it. Runs under
/// the same registry mutex as [`acquire_outcome_lock`], so a concurrent
/// caller can never observe (or race against) a torn removal.
pub(super) fn release_outcome_lock(key: &str) {
    let registry = OUTCOME_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut map = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = map.get(key)
        && Arc::strong_count(entry) == 1
    {
        map.remove(key);
    }
}
