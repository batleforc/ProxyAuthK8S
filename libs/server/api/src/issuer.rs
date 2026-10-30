//! The proxy as the OIDC issuer of its `Issuer` clusters (RFC 0014 §4.4,
//! §5.2, §5.3): the discovery document and JWKS each apiserver loads from
//! `/issuer/{ns}/{cluster}`, and the short-lived JWT the proxy signs for each
//! asserted identity. The keys are the controller's (`proxyauth-issuer-<UID>`
//! in the proxy's namespace); a replica only reads them, behind a 30 s cache
//! refreshed by one request per cluster at a time, and keeps serving the last
//! good set through an API outage until its keys retire (decision 44).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use common::oidc_cache::TtlCache;
use crd::ProxyKubeApi;
use crd::upstream_auth::issuer_key::{
    self, DOCUMENT_MAX_AGE_SECONDS, IssuerKeySet, KEYS_CACHE_SECONDS,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use k8s_openapi::api::core::v1::Secret;
use kube::ResourceExt as _;
use secrecy::SecretString;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tracing::warn;

use crate::SharedState;
use crate::cluster::redirect::upstream_identity::UpstreamIdentity;

/// The budget of one read of a key Secret.
const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// After a failed read, the set in hand is served this long before the next
/// try (plus up to as much again, so the replicas do not retry together).
const RETRY_AFTER: Duration = Duration::from_secs(5);
/// Issued JWTs kept for reuse, across every cluster.
const MAX_CACHED_JWTS: usize = 10_000;

fn now() -> i64 {
    k8s_openapi::jiff::Timestamp::now().as_second()
}

/// A cluster's keys as last read, and until when they count as fresh.
#[derive(Clone)]
struct Held {
    keys: Arc<IssuerKeySet>,
    fresh_until: Instant,
}

#[derive(Default)]
struct Slot {
    /// One refresh at a time per cluster.
    refresh: tokio::sync::Mutex<()>,
    held: RwLock<Option<Held>>,
}

impl Slot {
    fn held(&self) -> Option<Held> {
        self.held.read().ok().and_then(|held| held.clone())
    }

    fn set(&self, held: Held) {
        if let Ok(mut slot) = self.held.write() {
            *slot = Some(held);
        }
    }

    /// The set in hand, if its keys have not all retired.
    fn servable(&self, at: i64) -> Option<Arc<IssuerKeySet>> {
        self.held()
            .map(|held| held.keys)
            .filter(|keys| keys.serve_until(at).is_none_or(|until| until > at))
    }
}

/// One slot per key Secret, created only for clusters the proxy serves.
static SLOTS: LazyLock<Mutex<HashMap<String, Arc<Slot>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn slot(name: &str) -> Option<Arc<Slot>> {
    let mut slots = SLOTS.lock().ok()?;
    if !slots.contains_key(name) {
        // A deleted or recreated cluster (new UID, new name) leaves its slot,
        // keys included: dropped once unused and unread for `SLOT_IDLE`, long
        // enough to keep serving an idle cluster through an outage (decision 44).
        let now = Instant::now();
        slots.retain(|_, slot| {
            Arc::strong_count(slot) > 1
                || slot.held().is_some_and(|held| {
                    now < held.fresh_until.checked_add(SLOT_IDLE).unwrap_or(now)
                })
        });
    }
    Some(Arc::clone(slots.entry(name.to_string()).or_default()))
}

/// How long a slot nobody reads keeps its keys.
const SLOT_IDLE: Duration = Duration::from_secs(86_400);

/// A cheap spread for retries: the replicas need not agree, only differ.
fn jitter() -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    Duration::from_millis(u64::from(nanos % 5_000))
}

/// `now + after`, or now when that cannot be represented.
fn later(after: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(after).unwrap_or(now)
}

/// `proxy`'s keys. The errors name the Secret, never its content.
async fn keys(state: &common::State, proxy: &ProxyKubeApi) -> Result<Arc<IssuerKeySet>, String> {
    let name = issuer_key::secret_name(proxy).map_err(|e| e.to_string())?;
    let uid = proxy.uid().ok_or("the ProxyKubeApi has no UID")?;
    let slot = slot(&name).ok_or("the key cache is unavailable")?;
    let fresh = |slot: &Slot| {
        slot.held()
            .filter(|held| Instant::now() < held.fresh_until)
            .map(|held| held.keys)
    };
    if let Some(keys) = fresh(&slot) {
        return Ok(keys);
    }
    let _refreshing = match slot.refresh.try_lock() {
        Ok(guard) => guard,
        // Another request refreshes: serve what is in hand meanwhile.
        Err(_) => match slot.servable(now()) {
            Some(keys) => return Ok(keys),
            None => slot.refresh.lock().await,
        },
    };
    if let Some(keys) = fresh(&slot) {
        return Ok(keys);
    }
    let namespace = state.lease_namespace.as_str();
    let api: kube::Api<Secret> = kube::Api::namespaced(state.client.clone(), namespace);
    let read = match tokio::time::timeout(READ_TIMEOUT, api.get(&name)).await {
        Ok(Ok(secret)) => issuer_key::read(&secret, &uid).map_err(|e| e.to_string()),
        Ok(Err(e)) => Err(format!("cannot read Secret {namespace}/{name}: {e}")),
        Err(_) => Err(format!("reading Secret {namespace}/{name} timed out")),
    };
    match read {
        Ok(keys) => {
            let keys = Arc::new(keys);
            slot.set(Held {
                keys: Arc::clone(&keys),
                fresh_until: later(Duration::from_secs(KEYS_CACHE_SECONDS.unsigned_abs())),
            });
            Ok(keys)
        }
        Err(message) => match slot.servable(now()) {
            Some(keys) => {
                warn!(cluster = %proxy.to_path(), %message, "serving the last good Issuer keys");
                slot.set(Held {
                    keys: Arc::clone(&keys),
                    fresh_until: later(RETRY_AFTER.saturating_add(jitter())),
                });
                Ok(keys)
            }
            None => Err(message),
        },
    }
}

/// The JWTs already signed, by identity, cluster, issuer and key (§5.2):
/// the JWT and its `exp`.
static JWTS: LazyLock<TtlCache<String, (Arc<SecretString>, i64)>> =
    LazyLock::new(|| TtlCache::with_max_entries(MAX_CACHED_JWTS));

static JTI: AtomicU64 = AtomicU64::new(0);

/// The cache key of a JWT: SHA-256 of a canonical JSON of what it asserts and
/// where (T11, T12); groups sorted and deduplicated, never joined by commas.
fn cache_key(identity: &UpstreamIdentity, uid: &str, issuer: &str, kid: &str) -> String {
    let mut groups: Vec<&str> = identity.groups.iter().map(String::as_str).collect();
    groups.sort_unstable();
    groups.dedup();
    let canonical = json!({
        "u": identity.username,
        "g": groups,
        "x": identity.extra,
        "c": uid,
        "i": issuer,
        "k": kid,
    });
    Sha256::digest(canonical.to_string().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The least a caller's token must have left for the proxy to sign a JWT
/// bounded by it (decision 61).
pub const MIN_SOURCE_TTL: Duration = Duration::from_secs(5);

/// The `exp` of a JWT signed at `iat`: `token_ttl_seconds`, never past the
/// caller's own token (decision 61).
fn expiry(iat: i64, ttl_seconds: u32, source_ttl: Option<Duration>) -> i64 {
    let own = iat.saturating_add(i64::from(ttl_seconds));
    source_ttl
        .and_then(|left| i64::try_from(left.as_secs()).ok())
        .map_or(own, |left| own.min(iat.saturating_add(left)))
}

/// How long before its `exp` a JWT stops being reused: the clock skew
/// margin of §5.2.
fn reuse_margin(ttl_seconds: u32) -> i64 {
    (i64::from(ttl_seconds) / 10).max(60)
}

/// `claims` signed ES256 by `key`, its `kid` in the header.
fn encode(key: &issuer_key::IssuerKey, claims: &Value) -> Result<String, &'static str> {
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(key.kid.clone());
    let der = STANDARD
        .decode(&key.pkcs8)
        .map_err(|_| "the Issuer key is not valid base64")?;
    jsonwebtoken::encode(&header, claims, &EncodingKey::from_ec_der(&der))
        .map_err(|_| "cannot sign with the Issuer key")
}

/// A JWT asserting `identity` to `proxy`'s apiserver, signed by the current
/// key; `source_ttl` is what is left of the caller's token, when readable.
///
/// # Errors
///
/// No `issuer_key`, no readable key, or no key active yet.
pub async fn sign(
    state: &common::State,
    proxy: &ProxyKubeApi,
    identity: &UpstreamIdentity,
    source_ttl: Option<Duration>,
) -> Result<SecretString, String> {
    let settings = proxy
        .spec
        .upstream_auth
        .as_ref()
        .and_then(|upstream| upstream.issuer_key.as_ref())
        .ok_or("no issuer_key")?;
    let keys = keys(state, proxy).await?;
    let iat = now();
    let key = keys.current(iat).ok_or("no Issuer key is active yet")?;
    let uid = proxy.uid().unwrap_or_default();
    let exp = expiry(iat, settings.token_ttl_seconds, source_ttl);
    let margin = reuse_margin(settings.token_ttl_seconds);
    let cache_key = cache_key(identity, &uid, &settings.issuer_url, &key.kid);
    if let Some((jwt, cached_exp)) = JWTS.get(&cache_key)
        && cached_exp <= exp
        && iat < cached_exp.saturating_sub(margin)
    {
        crate::metrics::record_issuer_token("cache_hit");
        return Ok((*jwt).clone());
    }
    let mut claims = json!({
        "iss": settings.issuer_url,
        "aud": proxy.to_identifier(),
        "sub": identity.username,
        "groups": identity.groups,
        "iat": iat,
        "nbf": iat.saturating_sub(30),
        "exp": exp,
        "jti": format!("{iat}-{}", JTI.fetch_add(1, Ordering::Relaxed)),
    });
    if !identity.extra.is_empty()
        && let Some(object) = claims.as_object_mut()
    {
        object.insert("extra".to_string(), json!(identity.extra));
    }
    let jwt = Arc::new(SecretString::from(encode(key, &claims)?));
    crate::metrics::record_issuer_token("signed");
    if let Ok(keep) = u64::try_from(exp.saturating_sub(margin).saturating_sub(iat)) {
        JWTS.insert(
            cache_key,
            (Arc::clone(&jwt), exp),
            Duration::from_secs(keep),
        );
    }
    Ok((*jwt).clone())
}

/// A cluster's two documents.
#[derive(Clone)]
struct Documents {
    discovery: Value,
    jwks: Value,
    /// Served no longer than this through an outage.
    serve_until: Option<i64>,
    /// Served without a lookup until then.
    fresh_until: Instant,
}

/// How long the documents are served without a lookup: below their
/// `max-age`, so a rotation still reaches the apiserver in time.
const DOCUMENTS_FRESH: Duration = Duration::from_secs(10);

/// Lookups (Redis, then the key Secret) the public routes run at once:
/// they are never rate limited (T15), so an invented `{ns}/{cluster}` loop
/// must not reach Redis without bound. Requests queue (first in, first
/// served) up to [`LOOKUP_WAIT`], then get the last good documents.
static LOOKUPS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
const LOOKUP_WAIT: Duration = Duration::from_secs(2);

/// The last good documents of each cluster, for an outage of Redis or the
/// API (decision 44). Positive entries only: an invented `{ns}/{cluster}`
/// never enters, so the memory is bounded by the clusters served.
static DOCUMENTS: LazyLock<Mutex<HashMap<String, Documents>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn documents_for(issuer_url: &str, keys: &IssuerKeySet, at: i64) -> Documents {
    let published: Vec<Value> = keys
        .published(at)
        .map(|key| {
            json!({
                "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256",
                "kid": key.kid, "x": key.x, "y": key.y,
            })
        })
        .collect();
    Documents {
        // `issuer` byte for byte as declared: go-oidc refuses any other.
        discovery: json!({
            "issuer": issuer_url,
            "jwks_uri": issuer_key::jwks_uri(issuer_url),
            "id_token_signing_alg_values_supported": ["ES256"],
            "response_types_supported": ["id_token"],
            "subject_types_supported": ["public"],
        }),
        jwks: json!({ "keys": published }),
        serve_until: keys.serve_until(at),
        fresh_until: later(DOCUMENTS_FRESH),
    }
}

/// The documents of `ns/cluster`: fresh when the cluster and its keys can be
/// read, else the last good ones while their keys have not retired.
async fn documents(state: &common::State, path: &str) -> Option<Documents> {
    let last_good = |fresh_only: bool| {
        let at = now();
        let store = DOCUMENTS.lock().ok()?;
        store
            .get(path)
            .filter(|documents| !fresh_only || Instant::now() < documents.fresh_until)
            .filter(|documents| documents.serve_until.is_none_or(|until| until > at))
            .cloned()
    };
    if let Some(documents) = last_good(true) {
        return Some(documents);
    }
    let Ok(Ok(_lookup)) = tokio::time::timeout(LOOKUP_WAIT, LOOKUPS.acquire()).await else {
        return last_good(false);
    };
    let fresh = async {
        let proxy = state.get_proxy_from_redis(path).await.map_err(|_| ())?;
        let Some(proxy) = proxy else {
            return Ok(None);
        };
        let Some(settings) = proxy
            .spec
            .upstream_auth
            .as_ref()
            .and_then(|upstream| upstream.issuer_key.as_ref())
        else {
            return Ok(None);
        };
        let keys = keys(state, &proxy).await.map_err(|_| ())?;
        Ok::<_, ()>(Some(documents_for(&settings.issuer_url, &keys, now())))
    };
    match fresh.await {
        Ok(Some(documents)) => {
            if let Ok(mut store) = DOCUMENTS.lock() {
                store.insert(path.to_string(), documents.clone());
            }
            Some(documents)
        }
        Ok(None) => {
            // Gone, or no longer an issuer: nothing to serve.
            if let Ok(mut store) = DOCUMENTS.lock() {
                store.remove(path);
            }
            None
        }
        Err(()) => last_good(false),
    }
}

fn document_response(document: Option<Value>) -> Response {
    let Some(document) = document else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = Json(document).into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("max-age={DOCUMENT_MAX_AGE_SECONDS}")) {
        response.headers_mut().insert(header::CACHE_CONTROL, value);
    }
    response
}

async fn discovery(
    State(state): State<SharedState>,
    Path((ns, cluster)): Path<(String, String)>,
) -> Response {
    let found = documents(&state, &format!("{ns}/{cluster}")).await;
    document_response(found.map(|documents| documents.discovery))
}

async fn jwks(
    State(state): State<SharedState>,
    Path((ns, cluster)): Path<(String, String)>,
) -> Response {
    let found = documents(&state, &format!("{ns}/{cluster}")).await;
    document_response(found.map(|documents| documents.jwks))
}

/// `/issuer/{ns}/{cluster}/…`: apart from `/clusters`, where every path is
/// relayed to the apiserver, which serves its own `/.well-known`. Public,
/// never banned nor rate limited (T15), out of the OpenAPI document.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route(
            "/issuer/{ns}/{cluster}/.well-known/openid-configuration",
            get(discovery),
        )
        .route("/issuer/{ns}/{cluster}/jwks", get(jwks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn identity(groups: &[&str]) -> UpstreamIdentity {
        UpstreamIdentity {
            username: "proxyauth:alice".into(),
            groups: groups.iter().map(ToString::to_string).collect(),
            extra: BTreeMap::new(),
        }
    }

    /// A slot nobody holds and nobody read lately goes when another is
    /// made; one in use stays.
    #[test]
    fn an_idle_slot_is_dropped_when_a_new_one_is_made() {
        let held = slot("test-held").unwrap();
        drop(slot("test-idle"));
        let _new = slot("test-new").unwrap();
        let slots = SLOTS.lock().unwrap();
        assert!(!slots.contains_key("test-idle"));
        assert!(slots.contains_key("test-held"));
        drop(held);
    }

    /// Decision 61: never past the caller's token, `token_ttl_seconds`
    /// otherwise.
    #[test]
    fn an_issued_jwt_never_outlives_the_source_token() {
        assert_eq!(expiry(1_000, 300, None), 1_300);
        assert_eq!(expiry(1_000, 300, Some(Duration::from_secs(120))), 1_120);
        assert_eq!(expiry(1_000, 300, Some(Duration::from_secs(3_600))), 1_300);
    }

    /// T11: a group change never reuses a cached JWT.
    #[test]
    fn a_group_change_never_reuses_a_cached_jwt() {
        let before = cache_key(&identity(&["proxyauth:dev"]), "u", "i", "k");
        let after = cache_key(
            &identity(&["proxyauth:dev", "proxyauth:ops"]),
            "u",
            "i",
            "k",
        );
        assert_ne!(before, after);
        // Order and duplicates do not matter; a comma never merges two.
        assert_eq!(
            cache_key(&identity(&["b", "a", "a"]), "u", "i", "k"),
            cache_key(&identity(&["a", "b"]), "u", "i", "k")
        );
        assert_ne!(
            cache_key(&identity(&["a,b"]), "u", "i", "k"),
            cache_key(&identity(&["a", "b"]), "u", "i", "k")
        );
    }

    /// T12: two clusters, issuers or keys never share a cached JWT.
    #[test]
    fn two_identities_never_share_a_cached_jwt() {
        let base = cache_key(&identity(&["g"]), "u1", "i", "k");
        assert_ne!(base, cache_key(&identity(&["g"]), "u2", "i", "k"));
        assert_ne!(base, cache_key(&identity(&["g"]), "u1", "j", "k"));
        assert_ne!(base, cache_key(&identity(&["g"]), "u1", "i", "l"));
        let mut bob = identity(&["g"]);
        bob.username = "proxyauth:bob".into();
        assert_ne!(base, cache_key(&bob, "u1", "i", "k"));
    }

    /// A key in the controller's format: what the JWKS publishes verifies
    /// what the proxy signs, `aud` and `iss` included (T9).
    #[test]
    fn a_signed_jwt_verifies_against_the_published_key() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let point = pair.public_key_raw();
        let key = issuer_key::IssuerKey {
            kid: "k1".into(),
            pkcs8: STANDARD.encode(pair.serialize_der()),
            x: URL_SAFE_NO_PAD.encode(&point[1..33]),
            y: URL_SAFE_NO_PAD.encode(&point[33..65]),
            created_at: 0,
            activate_at: 0,
            retire_at: None,
        };
        let claims = json!({
            "iss": "https://p.example/issuer/a/b", "aud": "proxyk8sauth:a/b",
            "sub": "proxyauth:alice", "groups": ["proxyauth:dev"],
            "iat": now(), "nbf": now() - 30, "exp": now() + 300,
        });
        let jwt = encode(&key, &claims).unwrap();
        let decoding = jsonwebtoken::DecodingKey::from_ec_components(&key.x, &key.y).unwrap();
        let mut validation = jsonwebtoken::Validation::new(Algorithm::ES256);
        validation.set_audience(&["proxyk8sauth:a/b"]);
        validation.set_issuer(&["https://p.example/issuer/a/b"]);
        let decoded = jsonwebtoken::decode::<Value>(&jwt, &decoding, &validation).unwrap();
        assert_eq!(decoded.claims["sub"], "proxyauth:alice");
        assert_eq!(decoded.header.kid.as_deref(), Some("k1"));
        // Another cluster's audience is refused.
        validation.set_audience(&["proxyk8sauth:a/c"]);
        assert!(jsonwebtoken::decode::<Value>(&jwt, &decoding, &validation).is_err());
    }

    #[test]
    fn the_discovery_names_the_issuer_byte_for_byte_and_the_jwks_every_live_key() {
        let key = |kid: &str, retire_at| issuer_key::IssuerKey {
            kid: kid.into(),
            pkcs8: "secret".into(),
            x: "x".into(),
            y: "y".into(),
            created_at: 0,
            activate_at: 0,
            retire_at,
        };
        let keys = IssuerKeySet {
            keys: vec![key("old", Some(10)), key("new", None)],
        };
        let documents = documents_for("https://p.example/issuer/a/b/", &keys, 20);
        assert_eq!(
            documents.discovery["issuer"],
            "https://p.example/issuer/a/b/"
        );
        assert_eq!(
            documents.discovery["jwks_uri"],
            "https://p.example/issuer/a/b/jwks"
        );
        let kids: Vec<&str> = documents.jwks["keys"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|k| k["kid"].as_str())
            .collect();
        assert_eq!(kids, ["new"]);
        // T8: the private key never reaches a document.
        assert!(!documents.jwks.to_string().contains("secret"));
    }
}
