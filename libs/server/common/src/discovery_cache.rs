//! Cache for OIDC provider discovery documents.
//!
//! Discovery is on the hot path twice over. Resolving a caller's identity runs
//! [`OidcConf::oidc_core`](crate::oidc_conf::OidcConf::oidc_core), which fetches
//! `/.well-known/openid-configuration` to build the client, and then
//! [`OidcConf::ensure_token_audience`](crate::oidc_conf::OidcConf::ensure_token_audience),
//! which fetches the *same* document again through a separate raw request to
//! find the introspection endpoint. Uncached, one authenticated request cost two
//! round trips to the provider before any real work happened — and on the
//! unauthenticated `/auth/*` surface an anonymous caller could drive them at
//! will.
//!
//! A discovery document is close to immutable in practice; providers publish it
//! precisely so clients will cache it. An hour is the same TTL
//! `jwt_validator`'s JWKS cache uses for the same document, so the two agree on
//! how stale a provider's advertised endpoints may get.
//!
//! Two maps rather than one, because the two consumers need different shapes:
//! the typed [`CoreProviderMetadata`] must come from `discover_async`, which
//! performs the issuer check guarding against a provider mix-up, and
//! deserializing it out of a shared raw blob would quietly drop that check.
//! The second map holds only the introspection endpoint, which is RFC 8414 and
//! not a field [`CoreProviderMetadata`] carries.
//!
//! So the document is still fetched twice per issuer — once per consumer — but
//! twice *per hour* rather than twice per request. Collapsing it to a single
//! fetch would mean parsing the metadata ourselves and re-implementing the
//! issuer check by hand; the second fetch is the cheaper price.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use openidconnect::core::CoreProviderMetadata;
use tokio::sync::RwLock;

/// How long a fetched discovery document is served without refetching.
const TTL: Duration = Duration::from_secs(60 * 60);

struct Cached<T> {
    value: T,
    fetched_at: Instant,
}

impl<T> Cached<T> {
    fn fresh(value: T) -> Self {
        Self {
            value,
            fetched_at: Instant::now(),
        }
    }

    fn is_fresh(&self) -> bool {
        self.fetched_at.elapsed() < TTL
    }
}

/// Per-issuer cache of discovery results, shared by every request.
#[derive(Default)]
pub struct DiscoveryCache {
    metadata: RwLock<HashMap<String, Cached<CoreProviderMetadata>>>,
    /// `None` means the provider was reached and advertises no introspection
    /// endpoint — a real answer worth remembering. A *failed* fetch is never
    /// stored, so a blip cannot suppress introspection for an hour.
    introspection: RwLock<HashMap<String, Cached<Option<String>>>>,
}

impl std::fmt::Debug for DiscoveryCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiscoveryCache")
    }
}

impl DiscoveryCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached provider metadata for `issuer_url`, if still fresh.
    pub async fn metadata(&self, issuer_url: &str) -> Option<CoreProviderMetadata> {
        let guard = self.metadata.read().await;
        guard
            .get(issuer_url)
            .filter(|cached| cached.is_fresh())
            .map(|cached| cached.value.clone())
    }

    /// Record freshly discovered provider metadata.
    pub async fn put_metadata(&self, issuer_url: String, metadata: CoreProviderMetadata) {
        self.metadata
            .write()
            .await
            .insert(issuer_url, Cached::fresh(metadata));
    }

    /// The cached introspection endpoint for `issuer_url`.
    ///
    /// The outer `Option` is "do we know?"; the inner one is "does the provider
    /// have one?".
    pub async fn introspection_endpoint(&self, issuer_url: &str) -> Option<Option<String>> {
        let guard = self.introspection.read().await;
        guard
            .get(issuer_url)
            .filter(|cached| cached.is_fresh())
            .map(|cached| cached.value.clone())
    }

    /// Record what the provider advertised. Call only after a *successful*
    /// fetch: storing a transport failure as `None` would disable introspection
    /// until the entry expired.
    pub async fn put_introspection_endpoint(&self, issuer_url: String, endpoint: Option<String>) {
        self.introspection
            .write()
            .await
            .insert(issuer_url, Cached::fresh(endpoint));
    }
}

#[cfg(test)]
mod tests {
    use super::DiscoveryCache;

    #[tokio::test]
    async fn an_unknown_issuer_is_a_miss() {
        let cache = DiscoveryCache::new();
        assert!(cache.metadata("https://issuer.example.com").await.is_none());
        assert!(
            cache
                .introspection_endpoint("https://issuer.example.com")
                .await
                .is_none()
        );
    }

    /// "The provider has no introspection endpoint" is an answer, not a miss —
    /// remembering it is what stops a refetch on every single request for the
    /// many providers that do not implement RFC 7662.
    #[tokio::test]
    async fn a_provider_without_introspection_is_remembered_as_such() {
        let cache = DiscoveryCache::new();
        cache
            .put_introspection_endpoint("https://issuer.example.com".to_string(), None)
            .await;

        assert_eq!(
            cache
                .introspection_endpoint("https://issuer.example.com")
                .await,
            Some(None),
            "a known-absent endpoint must be a hit carrying None, not a miss"
        );
    }

    #[tokio::test]
    async fn an_endpoint_is_served_back() {
        let cache = DiscoveryCache::new();
        cache
            .put_introspection_endpoint(
                "https://issuer.example.com".to_string(),
                Some("https://issuer.example.com/introspect".to_string()),
            )
            .await;

        assert_eq!(
            cache
                .introspection_endpoint("https://issuer.example.com")
                .await,
            Some(Some("https://issuer.example.com/introspect".to_string()))
        );
    }

    /// Two clusters may front different providers; serving one issuer's
    /// endpoints for another would send tokens to the wrong place.
    #[tokio::test]
    async fn issuers_do_not_share_entries() {
        let cache = DiscoveryCache::new();
        cache
            .put_introspection_endpoint(
                "https://a.example.com".to_string(),
                Some("https://a.example.com/introspect".to_string()),
            )
            .await;

        assert!(
            cache
                .introspection_endpoint("https://b.example.com")
                .await
                .is_none()
        );
    }
}
