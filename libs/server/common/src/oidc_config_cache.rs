//! Short-lived cache for an `oidc_provider.config_from` Secret.
//!
//! `get_oidc_conf` runs on the request path — `User::get_user_info_with_proxy`
//! is called for every proxied request — so resolving `config_from` without a
//! cache would put one apiserver Secret GET behind every request a cluster
//! serves. That scales apiserver load with proxy traffic and adds a round trip
//! to the hot path for a value that changes about never.
//!
//! The TTL is deliberately short. This holds an OAuth client secret, so a
//! rotation has to take effect promptly; 30 seconds collapses a burst of traffic
//! into a single read while keeping the window in which a rotated credential is
//! still served bounded and small.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crd::authentication_configuration::OidcProviderOverrides;
use tokio::sync::RwLock;

/// How long a resolved Secret is reused before being read again.
const TTL: Duration = Duration::from_secs(30);

struct Cached {
    value: OidcProviderOverrides,
    fetched_at: Instant,
}

/// Process-wide cache of resolved `config_from` Secrets, keyed by
/// [`crd::authentication_configuration::OidcConfigSource::cache_key`].
#[derive(Default)]
pub struct OidcConfigCache {
    entries: RwLock<HashMap<String, Cached>>,
}

impl std::fmt::Debug for OidcConfigCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never derive: the entries hold resolved client secrets.
        f.write_str("OidcConfigCache")
    }
}

impl OidcConfigCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached overrides for `key`, if still fresh.
    pub async fn get(&self, key: &str) -> Option<OidcProviderOverrides> {
        let entries = self.entries.read().await;
        entries
            .get(key)
            .filter(|cached| cached.fetched_at.elapsed() < TTL)
            .map(|cached| cached.value.clone())
    }

    /// Record a freshly resolved value.
    pub async fn put(&self, key: String, value: OidcProviderOverrides) {
        self.entries.write().await.insert(
            key,
            Cached {
                value,
                fetched_at: Instant::now(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::OidcConfigCache;
    use crd::authentication_configuration::OidcProviderOverrides;

    fn overrides(secret: &str) -> OidcProviderOverrides {
        OidcProviderOverrides {
            client_secret: Some(secret.to_string()),
            ..OidcProviderOverrides::default()
        }
    }

    #[tokio::test]
    async fn a_stored_value_is_served_back() {
        let cache = OidcConfigCache::new();
        assert!(cache.get("secret/default/oidc").await.is_none());

        cache
            .put("secret/default/oidc".to_string(), overrides("s3cr3t"))
            .await;

        assert_eq!(
            cache
                .get("secret/default/oidc")
                .await
                .and_then(|o| o.client_secret)
                .as_deref(),
            Some("s3cr3t")
        );
    }

    /// Two references that resolve to different Secrets must never share an
    /// entry — one cluster's client secret reaching another would be worse than
    /// no cache at all.
    #[tokio::test]
    async fn different_keys_do_not_share_an_entry() {
        let cache = OidcConfigCache::new();
        cache
            .put("secret/tenant-a/oidc".to_string(), overrides("a"))
            .await;
        cache
            .put("secret/tenant-b/oidc".to_string(), overrides("b"))
            .await;

        assert_eq!(
            cache
                .get("secret/tenant-a/oidc")
                .await
                .and_then(|o| o.client_secret)
                .as_deref(),
            Some("a")
        );
        assert!(cache.get("secret/tenant-c/oidc").await.is_none());
    }

    /// The entries hold client secrets; `Debug` must never widen a log line into
    /// a credential leak.
    #[tokio::test]
    async fn debug_never_renders_the_cached_secrets() {
        let cache = OidcConfigCache::new();
        cache
            .put("secret/default/oidc".to_string(), overrides("unmistakable"))
            .await;

        let rendered = format!("{cache:?}");
        assert!(!rendered.contains("unmistakable"), "{rendered}");
    }
}
