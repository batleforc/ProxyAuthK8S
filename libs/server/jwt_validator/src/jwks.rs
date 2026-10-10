//! Fetching and caching an issuer's JWKS.
//!
//! Two layers of cache, both per issuer URL: the discovery document (to learn
//! `jwks_uri`) and the JWKS itself. Both are in-process rather than in Redis —
//! a JWKS is a few hundred bytes, refetching it costs one HTTPS call per replica
//! per TTL, and keeping it local means the token path never depends on Redis
//! being up.
//!
//! Key rotation is handled by refetching when a token names a `kid` the cached
//! set does not contain, which is what lets a provider roll a key without a
//! restart. That refetch is rate-limited per issuer: without a floor, a caller
//! sending tokens with random `kid`s would turn every request into an outbound
//! HTTPS call to the provider — an amplification vector pointed at the IdP.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::{Jwk, JwkSet};
use tokio::sync::RwLock;

use crate::error::JwtValidationError;

/// How long a fetched JWKS is served without refetching.
const JWKS_TTL: Duration = Duration::from_secs(15 * 60);
/// How long a fetched discovery document is served without refetching. Longer
/// than the JWKS TTL: `jwks_uri` changes far less often than the keys behind it.
const DISCOVERY_TTL: Duration = Duration::from_secs(60 * 60);
/// Floor between two unknown-`kid` refetches for the same issuer.
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
/// How far past its TTL an entry may still be served when the issuer cannot be
/// reached.
///
/// The stale fallback trades a brief provider outage for continued service, but
/// it must not be unbounded: an attacker who can keep the JWKS endpoint
/// unreachable would otherwise keep a *revoked* signing key trusted forever.
/// Beyond this, a fetch failure fails closed.
const MAX_STALE: Duration = Duration::from_secs(24 * 60 * 60);

struct Cached<T> {
    value: T,
    fetched_at: Instant,
}

impl<T> Cached<T> {
    fn is_fresh(&self, ttl: Duration) -> bool {
        self.fetched_at.elapsed() < ttl
    }
}

/// Per-issuer JWKS cache shared by every request.
///
/// Built once at start-up and held in the server state, so the cache is process
/// wide rather than per request.
#[derive(Default)]
pub struct JwksCache {
    discovery: RwLock<HashMap<String, Cached<String>>>,
    jwks: RwLock<HashMap<String, Cached<Arc<JwkSet>>>>,
    last_forced_refresh: RwLock<HashMap<String, Instant>>,
}

impl std::fmt::Debug for JwksCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JwksCache")
    }
}

impl JwksCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve `discovery_endpoint` to the issuer's `jwks_uri`.
    async fn jwks_uri(
        &self,
        client: &reqwest::Client,
        discovery_endpoint: &str,
    ) -> Result<String, JwtValidationError> {
        let cached = self
            .discovery
            .read()
            .await
            .get(discovery_endpoint)
            .map(|cached| (cached.value.clone(), cached.is_fresh(DISCOVERY_TTL)));
        if let Some((value, true)) = cached {
            return Ok(value);
        }

        let document: serde_json::Value =
            match self.fetch_discovery(client, discovery_endpoint).await {
                Ok(document) => document,
                // A `jwks_uri` does not change while the issuer is down. Serving the
                // expired one keeps authentication working through a provider blip
                // instead of failing every request the moment the TTL lapses.
                Err(e) => {
                    if let Some((value, _)) = cached {
                        tracing::warn!(
                            discovery_endpoint,
                            error = %e,
                            "could not refresh the discovery document; serving the expired jwks_uri"
                        );
                        return Ok(value);
                    }
                    return Err(e);
                }
            };

        let jwks_uri = document
            .get("jwks_uri")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| JwtValidationError::JwksInvalid {
                url: discovery_endpoint.to_string(),
                reason: "the discovery document has no jwks_uri".to_string(),
            })?
            .to_string();

        self.discovery.write().await.insert(
            discovery_endpoint.to_string(),
            Cached {
                value: jwks_uri.clone(),
                fetched_at: Instant::now(),
            },
        );
        Ok(jwks_uri)
    }

    async fn fetch_discovery(
        &self,
        client: &reqwest::Client,
        discovery_endpoint: &str,
    ) -> Result<serde_json::Value, JwtValidationError> {
        client
            .get(discovery_endpoint)
            .send()
            .await
            .map_err(|source| JwtValidationError::JwksFetch {
                url: discovery_endpoint.to_string(),
                source: Box::new(source),
            })?
            .error_for_status()
            .map_err(|source| JwtValidationError::JwksFetch {
                url: discovery_endpoint.to_string(),
                source: Box::new(source),
            })?
            .json()
            .await
            .map_err(|source| JwtValidationError::JwksFetch {
                url: discovery_endpoint.to_string(),
                source: Box::new(source),
            })
    }

    async fn fetch_jwks(
        &self,
        client: &reqwest::Client,
        jwks_uri: &str,
    ) -> Result<Arc<JwkSet>, JwtValidationError> {
        let set: JwkSet = client
            .get(jwks_uri)
            .send()
            .await
            .map_err(|source| JwtValidationError::JwksFetch {
                url: jwks_uri.to_string(),
                source: Box::new(source),
            })?
            .error_for_status()
            .map_err(|source| JwtValidationError::JwksFetch {
                url: jwks_uri.to_string(),
                source: Box::new(source),
            })?
            .json()
            .await
            .map_err(|source| JwtValidationError::JwksFetch {
                url: jwks_uri.to_string(),
                source: Box::new(source),
            })?;

        if set.keys.is_empty() {
            return Err(JwtValidationError::JwksInvalid {
                url: jwks_uri.to_string(),
                reason: "the JWKS carries no keys".to_string(),
            });
        }

        let set = Arc::new(set);
        self.jwks.write().await.insert(
            jwks_uri.to_string(),
            Cached {
                value: Arc::clone(&set),
                fetched_at: Instant::now(),
            },
        );
        Ok(set)
    }

    /// Whether an unknown-`kid` refetch for `jwks_uri` is allowed right now.
    ///
    /// Records the attempt as it allows it, so the floor applies even when the
    /// refetch itself fails — otherwise a provider that is down would be
    /// hammered once per request.
    async fn may_force_refresh(&self, jwks_uri: &str) -> bool {
        let mut last = self.last_forced_refresh.write().await;
        match last.get(jwks_uri) {
            Some(at) if at.elapsed() < MIN_REFRESH_INTERVAL => false,
            _ => {
                last.insert(jwks_uri.to_string(), Instant::now());
                true
            }
        }
    }

    /// Find the key a token's `kid` names, fetching or refreshing as needed.
    ///
    /// A token with no `kid` is only resolved when the issuer publishes exactly
    /// one key: picking one of several would mean trying keys until one verifies,
    /// which is how a signature check becomes an oracle.
    pub async fn key_for(
        &self,
        client: &reqwest::Client,
        discovery_endpoint: &str,
        kid: Option<&str>,
    ) -> Result<Jwk, JwtValidationError> {
        let jwks_uri = self.jwks_uri(client, discovery_endpoint).await?;

        let cached = {
            let guard = self.jwks.read().await;
            guard.get(&jwks_uri).map(|cached| {
                (
                    Arc::clone(&cached.value),
                    cached.is_fresh(JWKS_TTL),
                    cached.fetched_at.elapsed(),
                )
            })
        };

        let set = match cached {
            Some((set, true, _)) => set,
            other => match self.fetch_jwks(client, &jwks_uri).await {
                Ok(set) => set,
                // Signing keys outlive the cache TTL by a wide margin, so an
                // expired-but-known key set still verifies the tokens the issuer
                // minted. Failing every request the moment the TTL lapses would
                // turn a brief provider outage into a full authentication
                // outage; a genuinely rotated key still misses below and drives
                // the (rate-limited) forced refetch.
                Err(e) => match other {
                    Some((stale, _, age)) if age < MAX_STALE => {
                        tracing::warn!(
                            jwks_uri,
                            error = %e,
                            age_secs = age.as_secs(),
                            "could not refresh the JWKS; serving the expired key set"
                        );
                        stale
                    }
                    // Too old to keep trusting, or never fetched at all.
                    _ => return Err(e),
                },
            },
        };

        if let Some(key) = select_key(&set, kid) {
            return Ok(key.clone());
        }

        // Unknown kid: the provider may have rotated. Refetch once, rate-limited.
        if self.may_force_refresh(&jwks_uri).await {
            tracing::debug!(jwks_uri, ?kid, "kid not in the cached JWKS; refetching");
            let set = self.fetch_jwks(client, &jwks_uri).await?;
            if let Some(key) = select_key(&set, kid) {
                return Ok(key.clone());
            }
        }

        Err(JwtValidationError::UnknownKey {
            kid: kid.map(ToString::to_string),
        })
    }
}

/// Pick the key a `kid` names, or the only key when the token carries none.
fn select_key<'a>(set: &'a JwkSet, kid: Option<&str>) -> Option<&'a Jwk> {
    match kid {
        Some(kid) => set.find(kid),
        None if set.keys.len() == 1 => set.keys.first(),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{JwksCache, select_key};
    use jsonwebtoken::jwk::JwkSet;

    fn jwks(kids: &[&str]) -> JwkSet {
        let keys: Vec<serde_json::Value> = kids
            .iter()
            .map(|kid| {
                serde_json::json!({
                    "kty": "RSA",
                    "use": "sig",
                    "alg": "RS256",
                    "kid": kid,
                    "n": "lVgy3w7yOs_K329uYDNJiQ4od66rGGnOtWBxu7xwIM0MaRrGNePrU4hJ_f_YxN5l85smux3FZ7ms0-tQmtmHhQUzCEWNcx-SuTIf5iSDDFjsxOe2QvG0IcdV6Sb-b-4-rOTVM8MMXEoPibvfIKYRMGKe5tkxJS56Bbn3g7_fEx8_4M95mymPJu9RVPPJp6okOehU7kehtBqLXdMIgbh_nnOu4ZrRNU7Xs2ozogQbrIX9d_uD0EmbQEG8ASZ-ErYILQBwIJoiQ-u7EYylD0l3fIXht18lYtLU0JU16sz591R_Fmc00BBMZQKY2yEQOgl-PJMAL9_w10cCyM5p_uaW7w",
                    "e": "AQAB",
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({ "keys": keys })).expect("jwks should parse")
    }

    #[test]
    fn a_kid_selects_its_key() {
        let set = jwks(&["a", "b"]);
        assert_eq!(
            select_key(&set, Some("b")).and_then(|k| k.common.key_id.as_deref()),
            Some("b")
        );
        assert!(select_key(&set, Some("missing")).is_none());
    }

    /// Trying every key until one verifies turns the signature check into an
    /// oracle, so a token with no `kid` is only resolvable when the choice is
    /// unambiguous.
    #[test]
    fn a_token_without_a_kid_resolves_only_against_a_single_key_set() {
        assert!(select_key(&jwks(&["only"]), None).is_some());
        assert!(select_key(&jwks(&["a", "b"]), None).is_none());
    }

    #[tokio::test]
    async fn a_forced_refresh_is_rate_limited_per_issuer() {
        let cache = JwksCache::new();
        assert!(cache.may_force_refresh("https://a.example.com/jwks").await);
        // Second attempt inside the floor is refused…
        assert!(!cache.may_force_refresh("https://a.example.com/jwks").await);
        // …but a different issuer is unaffected.
        assert!(cache.may_force_refresh("https://b.example.com/jwks").await);
    }
}
