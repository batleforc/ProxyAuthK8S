//! Per-cluster cache of the upstream HTTP client and TLS configuration.
//!
//! Without it every proxied request resolved the cluster's TLS material (CA
//! and mTLS client certificate, possibly read from a Kubernetes Secret or
//! ConfigMap: one apiserver round-trip each) and built a fresh
//! `reqwest::Client`, so nothing was ever kept alive: one TCP connect and one
//! TLS handshake to the target apiserver per request.
//!
//! - **Key**: the proxy identity (`namespace/name`) plus a fingerprint of
//!   everything the client is built from: the `cert` and `client_cert` sources
//!   of the spec and the object UID. Editing those fields (or deleting and
//!   recreating the proxy) produces a new key, hence a new client. The target
//!   URL is deliberately not part of it: the client is URL-agnostic and
//!   `reqwest` pools connections per scheme/host/port anyway.
//! - **TTL**: `PROXY_UPSTREAM_CLIENT_TTL` seconds (default 300, `0` disables
//!   caching). A Secret/ConfigMap-sourced certificate can rotate without the CR
//!   changing; the TTL bounds how long the old material is used before it is
//!   resolved again.
//! - **Bounded**: at most [`MAX_CACHED_PROXIES`] entries per cache.
//! - Only successes are cached: a failed resolution is retried on the next
//!   request.
//! - **Eviction**: entries of a deleted proxy are dropped by the controller's
//!   cleanup ([`evict_proxy`]) when it runs in this process; otherwise they
//!   simply expire.
//!
//! Concurrent misses for the same key may each build a client; the last one
//! stored wins. That only costs a duplicate build on a cold cache.

use std::{
    future::Future,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use crd::ProxyKubeApi;
use kube::ResourceExt as _;
use sha2::{Digest, Sha256};

use crate::oidc_cache::{TtlCache, caching_enabled};

/// Upper bound on cached entries per cache (one per proxied cluster in
/// practice; a full cache is pruned, then cleared).
pub const MAX_CACHED_PROXIES: usize = 1024;

/// Identity of a cached upstream client: which proxy, built from what.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UpstreamCacheKey {
    namespace: String,
    name: String,
    fingerprint: String,
}

impl UpstreamCacheKey {
    /// The key for `proxy` as it is configured right now.
    #[must_use]
    pub fn for_proxy(proxy: &ProxyKubeApi) -> Self {
        Self {
            namespace: proxy.namespace().unwrap_or_default(),
            name: proxy.name_any(),
            fingerprint: tls_fingerprint(proxy),
        }
    }

    /// Whether this key belongs to the proxy `namespace/name`.
    #[must_use]
    pub fn is_proxy(&self, namespace: &str, name: &str) -> bool {
        self.namespace == namespace && self.name == name
    }
}

/// SHA-256 of the spec fields the upstream client depends on (and the UID, so a
/// recreated proxy starts from scratch). Inline certificates are hashed, never
/// kept in the key.
fn tls_fingerprint(proxy: &ProxyKubeApi) -> String {
    let material = serde_json::json!({
        "uid": proxy.metadata.uid,
        "cert": proxy.spec.cert,
        "client_cert": proxy.spec.client_cert,
    });
    // `serde_json::Map` is ordered (no `preserve_order`), so the encoding is
    // stable; a serialisation failure is impossible for these plain types, and
    // would only make the fingerprint coarser, never shared across proxies.
    let encoded = serde_json::to_vec(&material).unwrap_or_default();
    URL_SAFE_NO_PAD.encode(Sha256::digest(encoded))
}

/// Cache hit/build counters, for tests and diagnostics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Lookups served from the cache.
    pub hits: u64,
    /// Lookups that ran the builder (miss, expired entry or caching disabled).
    pub builds: u64,
}

/// A bounded TTL cache of per-proxy upstream values.
pub struct UpstreamCache<V> {
    entries: TtlCache<UpstreamCacheKey, V>,
    hits: AtomicU64,
    builds: AtomicU64,
}

impl<V: Clone> UpstreamCache<V> {
    /// An empty cache holding at most `max_entries` entries.
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: TtlCache::with_max_entries(max_entries),
            hits: AtomicU64::new(0),
            builds: AtomicU64::new(0),
        }
    }

    /// The cached value for `key`, or the result of `build`, stored for `ttl`
    /// when it succeeds. A zero `ttl` bypasses the cache entirely; an error is
    /// returned as-is and never stored.
    ///
    /// # Errors
    ///
    /// Whatever `build` returns.
    pub async fn get_or_try_insert_with<E, Fut>(
        &self,
        key: &UpstreamCacheKey,
        ttl: Duration,
        build: impl FnOnce() -> Fut,
    ) -> Result<V, E>
    where
        Fut: Future<Output = Result<V, E>>,
    {
        if !ttl.is_zero()
            && let Some(value) = self.entries.get(key)
        {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return Ok(value);
        }
        self.builds.fetch_add(1, Ordering::Relaxed);
        let value = build().await?;
        self.entries.insert(key.clone(), value.clone(), ttl);
        Ok(value)
    }

    /// Store `value` for `ttl` (nothing when `ttl` is zero).
    pub fn insert(&self, key: UpstreamCacheKey, value: V, ttl: Duration) {
        self.entries.insert(key, value, ttl);
    }

    /// Drop every entry of the proxy `namespace/name`, whatever its fingerprint.
    pub fn evict_proxy(&self, namespace: &str, name: &str) {
        self.entries.retain(|key| !key.is_proxy(namespace, name));
    }

    /// Number of entries currently held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds no entry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Hit/build counters since the process started.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            builds: self.builds.load(Ordering::Relaxed),
        }
    }
}

/// Upstream `reqwest` clients (standard and virtual API paths).
pub static UPSTREAM_CLIENTS: LazyLock<UpstreamCache<reqwest::Client>> =
    LazyLock::new(|| UpstreamCache::new(MAX_CACHED_PROXIES));

/// Upstream TLS configurations (upgrade path: exec/attach/port-forward).
pub static UPSTREAM_TLS_CONFIGS: LazyLock<UpstreamCache<Arc<rustls::ClientConfig>>> =
    LazyLock::new(|| UpstreamCache::new(MAX_CACHED_PROXIES));

/// Drop the cached client and TLS configuration of the proxy `namespace/name`.
pub fn evict_proxy(namespace: &str, name: &str) {
    UPSTREAM_CLIENTS.evict_proxy(namespace, name);
    UPSTREAM_TLS_CONFIGS.evict_proxy(namespace, name);
}

/// How long to cache an upstream client/TLS configuration: the configured
/// `PROXY_UPSTREAM_CLIENT_TTL`, or zero (no caching) when caching is disabled.
///
/// Like the OIDC caches, this one is off under the `test-util` feature: test
/// binaries reuse proxy names and wiremock ports across tests, and a pooled
/// keep-alive connection to a server a previous test tore down would leak
/// between tests. A test that wants it calls [`enable_for_tests`].
#[must_use]
pub fn upstream_client_ttl() -> Duration {
    if enabled() {
        crate::config::get().proxy.upstream_client_ttl
    } else {
        Duration::ZERO
    }
}

#[cfg(not(feature = "test-util"))]
fn enabled() -> bool {
    caching_enabled()
}

#[cfg(feature = "test-util")]
static ENABLED_FOR_TESTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "test-util")]
fn enabled() -> bool {
    caching_enabled() || ENABLED_FOR_TESTS.load(Ordering::Relaxed)
}

/// Turn the upstream client cache on in a `test-util` build, for the rest of
/// the process. Only call it from a test binary whose tests use distinct proxy
/// names.
#[cfg(feature = "test-util")]
pub fn enable_for_tests() {
    ENABLED_FOR_TESTS.store(true, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crd::{
        ProxyKubeApiSpec,
        certificate::{CertSource, ClientCertificate},
        service::Service,
    };

    fn proxy(ns: &str, name: &str, cert: CertSource) -> ProxyKubeApi {
        let mut proxy = ProxyKubeApi::new(
            name,
            ProxyKubeApiSpec {
                enabled: true,
                cert,
                client_cert: None,
                service: Service::ExternalService {
                    url: "https://api.example:6443".to_string(),
                },
                auth_config: None,
                security_config: None,
                expose_via_dashboard: false,
                dashboard_group: None,
                proxy_group: None,
                virtual_apis: Vec::new(),
            },
        );
        proxy.metadata.namespace = Some(ns.to_string());
        proxy
    }

    fn secret(name: &str) -> CertSource {
        CertSource::Secret {
            name: name.to_string(),
            key: "ca.crt".to_string(),
            namespace: None,
        }
    }

    #[test]
    fn key_changes_with_the_tls_relevant_spec_only() {
        let base = proxy("ns", "a", secret("ca"));
        let key = UpstreamCacheKey::for_proxy(&base);
        assert_eq!(key, UpstreamCacheKey::for_proxy(&base.clone()));

        // Another proxy, same spec: distinct.
        assert_ne!(
            key,
            UpstreamCacheKey::for_proxy(&proxy("ns", "b", secret("ca")))
        );
        assert_ne!(
            key,
            UpstreamCacheKey::for_proxy(&proxy("other", "a", secret("ca")))
        );

        // CA source edited.
        assert_ne!(
            key,
            UpstreamCacheKey::for_proxy(&proxy("ns", "a", secret("other-ca")))
        );
        assert_ne!(
            key,
            UpstreamCacheKey::for_proxy(&proxy("ns", "a", CertSource::Insecure(true)))
        );

        // mTLS client certificate added.
        let mut mtls = base.clone();
        mtls.spec.client_cert = Some(ClientCertificate {
            cert: secret("client"),
            key: secret("client-key"),
        });
        assert_ne!(key, UpstreamCacheKey::for_proxy(&mtls));

        // Recreated under the same name.
        let mut recreated = base.clone();
        recreated.metadata.uid = Some("new-uid".to_string());
        assert_ne!(key, UpstreamCacheKey::for_proxy(&recreated));

        // Fields the client does not depend on keep the key.
        let mut retargeted = base.clone();
        retargeted.spec.service = Service::ExternalService {
            url: "https://elsewhere:6443".to_string(),
        };
        retargeted.spec.expose_via_dashboard = true;
        assert_eq!(key, UpstreamCacheKey::for_proxy(&retargeted));
    }

    #[test]
    fn inline_certificates_are_not_kept_in_the_key() {
        let key = UpstreamCacheKey::for_proxy(&proxy(
            "ns",
            "a",
            CertSource::Cert("SECRET-PEM-MATERIAL".to_string()),
        ));
        assert!(!format!("{key:?}").contains("SECRET-PEM-MATERIAL"));
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime should build")
            .block_on(future)
    }

    #[test]
    fn hits_are_served_from_the_cache_until_the_ttl_expires() {
        let cache: UpstreamCache<u32> = UpstreamCache::new(8);
        let key = UpstreamCacheKey::for_proxy(&proxy("ns", "a", secret("ca")));
        let ttl = Duration::from_millis(50);
        block_on(async {
            let first: Result<u32, ()> = cache
                .get_or_try_insert_with(&key, ttl, || async { Ok(1) })
                .await;
            let second: Result<u32, ()> = cache
                .get_or_try_insert_with(&key, ttl, || async { Ok(2) })
                .await;
            assert_eq!((first, second), (Ok(1), Ok(1)));
            assert_eq!(cache.stats(), CacheStats { hits: 1, builds: 1 });

            tokio::time::sleep(Duration::from_millis(60)).await;
            let expired: Result<u32, ()> = cache
                .get_or_try_insert_with(&key, ttl, || async { Ok(3) })
                .await;
            assert_eq!(expired, Ok(3), "an expired entry is rebuilt");
            assert_eq!(cache.stats(), CacheStats { hits: 1, builds: 2 });
        });
    }

    #[test]
    fn failures_are_not_cached() {
        let cache: UpstreamCache<u32> = UpstreamCache::new(8);
        let key = UpstreamCacheKey::for_proxy(&proxy("ns", "a", secret("ca")));
        let ttl = Duration::from_secs(60);
        block_on(async {
            let failed = cache
                .get_or_try_insert_with(&key, ttl, || async { Err::<u32, _>("boom") })
                .await;
            assert_eq!(failed, Err("boom"));
            assert!(cache.is_empty());
            let retried = cache
                .get_or_try_insert_with(&key, ttl, || async { Ok::<_, &str>(7) })
                .await;
            assert_eq!(retried, Ok(7));
            assert_eq!(cache.stats().builds, 2);
        });
    }

    #[test]
    fn zero_ttl_bypasses_the_cache() {
        let cache: UpstreamCache<u32> = UpstreamCache::new(8);
        let key = UpstreamCacheKey::for_proxy(&proxy("ns", "a", secret("ca")));
        block_on(async {
            for expected in [1, 2] {
                let value = cache
                    .get_or_try_insert_with(&key, Duration::ZERO, || async {
                        Ok::<_, ()>(expected)
                    })
                    .await;
                assert_eq!(value, Ok(expected));
            }
        });
        assert!(cache.is_empty());
        assert_eq!(cache.stats(), CacheStats { hits: 0, builds: 2 });
    }

    #[test]
    fn evicting_a_proxy_drops_all_its_fingerprints_only() {
        let cache: UpstreamCache<u32> = UpstreamCache::new(8);
        let ttl = Duration::from_secs(60);
        let a1 = UpstreamCacheKey::for_proxy(&proxy("ns", "a", secret("ca")));
        let a2 = UpstreamCacheKey::for_proxy(&proxy("ns", "a", secret("ca2")));
        let b = UpstreamCacheKey::for_proxy(&proxy("ns", "b", secret("ca")));
        cache.insert(a1, 1, ttl);
        cache.insert(a2, 2, ttl);
        cache.insert(b.clone(), 3, ttl);
        cache.evict_proxy("ns", "a");
        assert_eq!(cache.len(), 1);
        assert!(b.is_proxy("ns", "b"));
    }

    #[test]
    fn caching_is_off_under_test_util_unless_enabled() {
        // This unit test runs without `test-util` in `cargo test -p common`, but
        // with it when the workspace unifies features; both must hold.
        if cfg!(feature = "test-util") && !enabled() {
            assert_eq!(upstream_client_ttl(), Duration::ZERO);
        } else {
            assert_eq!(
                upstream_client_ttl(),
                crate::config::get().proxy.upstream_client_ttl
            );
        }
    }
}
