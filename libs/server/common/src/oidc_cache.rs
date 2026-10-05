//! Process-wide caches that keep the identity provider off the hot path.
//!
//! Without them every authenticated proxied request re-ran OIDC discovery (once
//! for the core client, once more for the introspection endpoint) before the
//! `/userinfo` call, so each request cost 2–3 IdP round-trips.
//!
//! - **Discovery** documents are cached per issuer URL for
//!   [`DISCOVERY_TTL`]: they change on key rotation / config changes only.
//! - **Validated tokens** (the resolved user after `/userinfo` + audience
//!   checks) are cached for `OIDC_TOKEN_CACHE_TTL` seconds (default 30, `0`
//!   disables), never past the JWT `exp` when it can be read. The key is the
//!   SHA-256 of the token together with the issuer, client id and audience, so a
//!   token validated for one provider/cluster is never reused for another, and
//!   the raw token is never kept in memory. Trade-off: a token revoked at the
//!   IdP keeps working for at most the TTL.
//!
//! Only successes are cached; failures always go back to the IdP.

use std::{
    collections::HashMap,
    hash::Hash,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

/// Whether the OIDC caches are populated.
///
/// Off under the `test-util` feature (enabled only from dev-dependencies):
/// wiremock reuses ports across tests in one process, so two tests can share an
/// issuer URL and would otherwise see each other's cached discovery documents
/// or validated tokens. A real issuer URL is stable, so production keeps them.
#[must_use]
pub const fn caching_enabled() -> bool {
    !cfg!(feature = "test-util")
}

/// How long a discovery document is reused before being fetched again.
pub const DISCOVERY_TTL: Duration = Duration::from_secs(300);

/// Default lifetime of a validated-token entry (`OIDC_TOKEN_CACHE_TTL`).
pub(crate) const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(30);

/// Upper bound on cached entries per cache; expired entries are pruned first,
/// and the cache is cleared if it is still full (bounds memory under a flood of
/// distinct tokens).
const MAX_ENTRIES: usize = 10_000;

/// A small TTL map guarded by a mutex (entries are tiny and lookups short).
///
/// Bounded: once `max_entries` is reached, expired entries are pruned first and
/// the map is cleared if it is still full.
pub struct TtlCache<K, V> {
    entries: Mutex<HashMap<K, (Instant, V)>>,
    max_entries: usize,
}

impl<K: Eq + Hash, V: Clone> Default for TtlCache<K, V> {
    fn default() -> Self {
        Self::with_max_entries(MAX_ENTRIES)
    }
}

impl<K: Eq + Hash, V: Clone> TtlCache<K, V> {
    /// An empty cache holding at most `max_entries` entries (at least one).
    #[must_use]
    pub fn with_max_entries(max_entries: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_entries: max_entries.max(1),
        }
    }

    /// Drop every entry whose key does not satisfy `keep`.
    pub fn retain(&self, mut keep: impl FnMut(&K) -> bool) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|key, _| keep(key));
        }
    }

    /// Number of entries currently held (expired ones included until pruned).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().map_or(0, |entries| entries.len())
    }

    /// Whether the cache holds no entry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The cached value for `key`, if present and not expired.
    pub fn get(&self, key: &K) -> Option<V> {
        let mut entries = self.entries.lock().ok()?;
        match entries.get(key) {
            Some((expires_at, value)) if *expires_at > Instant::now() => Some(value.clone()),
            Some(_) => {
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    /// Store `value` for `ttl`. A zero `ttl` stores nothing.
    pub fn insert(&self, key: K, value: V, ttl: Duration) {
        if ttl.is_zero() {
            return;
        }
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        if entries.len() >= self.max_entries && !entries.contains_key(&key) {
            let now = Instant::now();
            entries.retain(|_, (expires_at, _)| *expires_at > now);
            if entries.len() >= self.max_entries {
                entries.clear();
            }
        }
        entries.insert(key, (Instant::now() + ttl, value));
    }
}

/// Lifetime of a validated-token entry, from `OIDC_TOKEN_CACHE_TTL` (seconds),
/// as loaded once into the process-wide [`crate::config::Config`].
#[must_use]
pub fn token_cache_ttl() -> Duration {
    crate::config::get().oidc.token_cache_ttl
}

/// Cache key for a validated token: never the raw token, and scoped to the
/// provider/client/audience it was validated against.
#[must_use]
pub fn token_cache_key(token: &str, issuer_url: &str, client_id: &str, audience: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [token, issuer_url, client_id, audience] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

/// Seconds until the JWT's `exp` claim, if `token` is a JWT carrying one.
///
/// The signature is not checked here: only call this for a token that has
/// already been validated by the IdP. Returns `Some(ZERO)` for an expired token.
#[must_use]
pub fn jwt_time_to_expiry(token: &str) -> Option<Duration> {
    let payload = token.split('.').nth(1)?;
    let claims: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    let exp = claims.get("exp")?.as_u64()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(Duration::from_secs(exp.saturating_sub(now)))
}

/// TTL to cache a validated `token` for: the configured TTL, capped by the
/// token's own remaining lifetime when it is a JWT.
#[must_use]
pub fn token_ttl_for(token: &str) -> Duration {
    let ttl = token_cache_ttl();
    jwt_time_to_expiry(token).map_or(ttl, |left| ttl.min(left))
}

/// Shared HTTP client for every IdP call: no redirects, bounded timeouts, and a
/// single connection pool instead of a new client (and TLS setup) per request.
pub static OIDC_HTTP_CLIENT: LazyLock<Result<reqwest::Client, String>> = LazyLock::new(|| {
    reqwest::ClientBuilder::new()
        // An OIDC/OAuth exchange must talk to the exact endpoint it targeted,
        // never a location the provider hands back.
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())
});

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt_with(claims: &serde_json::Value) -> String {
        format!(
            "e30.{}.sig",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap())
        )
    }

    #[test]
    fn cache_returns_fresh_entries_and_drops_expired_ones() {
        let cache: TtlCache<&str, u32> = TtlCache::default();
        cache.insert("fresh", 1, Duration::from_secs(60));
        cache.insert("stale", 2, Duration::from_nanos(1));
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(cache.get(&"fresh"), Some(1));
        assert_eq!(cache.get(&"stale"), None);
        assert_eq!(cache.get(&"missing"), None);
    }

    #[test]
    fn cache_is_bounded_and_retain_filters_keys() {
        let cache: TtlCache<u32, u32> = TtlCache::with_max_entries(2);
        cache.insert(1, 1, Duration::from_secs(60));
        cache.insert(2, 2, Duration::from_secs(60));
        // Overwriting an existing key never evicts.
        cache.insert(2, 20, Duration::from_secs(60));
        assert_eq!(cache.len(), 2);
        // A third live key clears the full cache before being stored.
        cache.insert(3, 3, Duration::from_secs(60));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&3), Some(3));

        cache.insert(4, 4, Duration::from_secs(60));
        cache.retain(|key| *key != 3);
        assert_eq!(cache.get(&3), None);
        assert_eq!(cache.get(&4), Some(4));
    }

    #[test]
    fn zero_ttl_disables_caching() {
        let cache: TtlCache<&str, u32> = TtlCache::default();
        cache.insert("k", 1, Duration::ZERO);
        assert_eq!(cache.get(&"k"), None);
    }

    #[test]
    fn key_is_scoped_to_provider_client_and_audience() {
        let base = token_cache_key("tok", "https://idp", "client", "aud");
        assert_ne!(
            base,
            token_cache_key("tok", "https://other-idp", "client", "aud")
        );
        assert_ne!(base, token_cache_key("tok", "https://idp", "other", "aud"));
        assert_ne!(
            base,
            token_cache_key("tok", "https://idp", "client", "other")
        );
        assert_ne!(
            base,
            token_cache_key("tok2", "https://idp", "client", "aud")
        );
        // Length-prefixing prevents boundary-shifting collisions.
        assert_ne!(
            token_cache_key("ab", "c", "client", "aud"),
            token_cache_key("a", "bc", "client", "aud")
        );
        assert!(!base.contains("tok"));
    }

    #[test]
    fn ttl_is_capped_by_jwt_expiry() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let soon = jwt_with(&serde_json::json!({ "exp": now + 5 }));
        assert!(token_ttl_for(&soon) <= Duration::from_secs(5));
        let expired = jwt_with(&serde_json::json!({ "exp": now - 10 }));
        assert_eq!(token_ttl_for(&expired), Duration::ZERO);
        assert_eq!(jwt_time_to_expiry("opaque-token"), None);
    }
}
