//! The RFC 8693 token exchange of `TokenExchange` (RFC 0010 §6.2), and the
//! in-memory cache of what it returns (§5.2).
//!
//! The caller's proxy token goes to the cluster's IdP as `subject_token`; the
//! token that comes back only ever lives here, in memory, in a
//! [`SecretString`]: never in Redis, a log, a metric or a response.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex as StdMutex},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use crd::upstream_auth::TokenExchangeSettings;
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;

use crate::oidc_cache::TtlCache;

pub const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
pub const ACCESS_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";

/// The IdP's answer is read up to this many bytes; past it, unreadable.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024;
/// The whole exchange, connection included.
pub const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);
/// An exchanged token is dropped this long before its own `exp`.
const EXPIRY_MARGIN: Duration = Duration::from_secs(30);

/// Why an exchange gave no token. Never a caller fault: the token was
/// validated before (gate 4), so none counts toward a ban (decision 7).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExchangeError {
    /// The IdP could not be reached, timed out or failed (`5xx`): `503`.
    #[error("the cluster's identity provider is unavailable: {0}")]
    Unavailable(String),
    /// The IdP refused the exchange (`4xx`, e.g. `invalid_grant`): `401`.
    #[error("the cluster's identity provider refused the exchange: {0}")]
    Refused(String),
    /// The answer had no `access_token` or was too large: `502`.
    #[error("the cluster's identity provider answered something unreadable: {0}")]
    Unreadable(String),
}

impl ExchangeError {
    /// The `result` label of `proxyauth_token_exchange_total` and the audit.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "unavailable",
            Self::Refused(_) => "refused",
            Self::Unreadable(_) => "unreadable",
        }
    }
}

/// Who exchanges, and where: the cluster's OIDC client.
pub struct ExchangeClient<'a> {
    pub token_endpoint: &'a str,
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
}

/// A token the cluster's IdP issued for the caller; its `Debug` is
/// `secrecy`'s, redacted.
#[derive(Clone, Debug)]
pub struct Exchanged {
    pub token: Arc<SecretString>,
    /// `expires_in`, when the IdP gave one.
    pub expires_in: Option<Duration>,
}

/// The form body of the exchange: the parameters of §6.2, the empty ones
/// omitted.
#[must_use]
pub fn form(settings: &TokenExchangeSettings, subject_token: &str) -> String {
    let mut form = form_urlencoded::Serializer::new(String::new());
    form.append_pair("grant_type", GRANT_TYPE)
        .append_pair("subject_token", subject_token)
        .append_pair("subject_token_type", ACCESS_TOKEN_TYPE)
        .append_pair("requested_token_type", ACCESS_TOKEN_TYPE);
    for (name, value) in [
        ("audience", &settings.audience),
        ("scope", &settings.scope),
        ("connector_id", &settings.connector_id),
    ] {
        if !value.is_empty() {
            form.append_pair(name, value);
        }
    }
    form.finish()
}

/// `client_secret_basic`: id and secret form-encoded, then base64
/// (RFC 6749 §2.3.1), so a `:` in either survives.
fn basic_authorization(client_id: &str, client_secret: &str) -> String {
    use base64::engine::general_purpose::STANDARD;
    let encode =
        |value: &str| form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>();
    format!(
        "Basic {}",
        STANDARD.encode(format!("{}:{}", encode(client_id), encode(client_secret)))
    )
}

/// The IdP's answer to the exchange, given its status and body.
///
/// # Errors
///
/// [`ExchangeError::Unavailable`] on a `5xx`, [`ExchangeError::Refused`] on
/// any other non-success, [`ExchangeError::Unreadable`] on a success without
/// a usable `access_token`.
pub fn parse_response(status: u16, body: &[u8]) -> Result<Exchanged, ExchangeError> {
    #[derive(serde::Deserialize)]
    struct Answer {
        access_token: Option<String>,
        expires_in: Option<u64>,
        error: Option<String>,
    }
    let answer = serde_json::from_slice::<Answer>(body).ok();
    if status >= 500 {
        return Err(ExchangeError::Unavailable(format!("status {status}")));
    }
    if !(200..300).contains(&status) {
        // The OAuth error code only (`invalid_grant`): never the description,
        // which an IdP may fill with the token's claims.
        let code = answer
            .and_then(|answer| answer.error)
            .filter(|code| code.len() <= 64 && code.bytes().all(|b| b.is_ascii_graphic()))
            .unwrap_or_else(|| "no error code".to_string());
        return Err(ExchangeError::Refused(format!("status {status}, {code}")));
    }
    let Some(answer) = answer else {
        return Err(ExchangeError::Unreadable(
            "not a JSON token response".to_string(),
        ));
    };
    match answer.access_token {
        Some(token) if !token.is_empty() => Ok(Exchanged {
            token: Arc::new(SecretString::from(token)),
            expires_in: answer.expires_in.map(Duration::from_secs),
        }),
        _ => Err(ExchangeError::Unreadable("no access_token".to_string())),
    }
}

/// One exchange with the cluster's IdP, uncached.
///
/// # Errors
///
/// See [`ExchangeError`]; a network failure or a timeout is
/// [`ExchangeError::Unavailable`].
pub async fn exchange(
    http: &reqwest::Client,
    client: &ExchangeClient<'_>,
    settings: &TokenExchangeSettings,
    subject_token: &str,
) -> Result<Exchanged, ExchangeError> {
    let mut request = http
        .post(client.token_endpoint)
        .timeout(EXCHANGE_TIMEOUT)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(reqwest::header::ACCEPT, "application/json");
    let mut body = form(settings, subject_token);
    match client.client_secret {
        Some(secret) => {
            let mut value = reqwest::header::HeaderValue::from_str(&basic_authorization(
                client.client_id,
                secret,
            ))
            .map_err(|_| ExchangeError::Refused("unusable client credentials".to_string()))?;
            value.set_sensitive(true);
            request = request.header(reqwest::header::AUTHORIZATION, value);
        }
        // A public client names itself in the form.
        None => {
            body = format!(
                "{body}&{}",
                form_urlencoded::Serializer::new(String::new())
                    .append_pair("client_id", client.client_id)
                    .finish()
            );
        }
    }
    let mut response = request
        .body(body)
        .send()
        .await
        .map_err(|e| ExchangeError::Unavailable(e.without_url().to_string()))?;
    let status = response.status().as_u16();
    let mut read = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ExchangeError::Unavailable(e.without_url().to_string()))?
    {
        if read.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ExchangeError::Unreadable(format!(
                "answer larger than {MAX_RESPONSE_BYTES} bytes"
            )));
        }
        read.extend_from_slice(&chunk);
    }
    parse_response(status, &read)
}

/// How long an exchanged token may be reused: the smallest of its own `exp`
/// minus 30 s, what is left of the caller's proxy token, and
/// `max_cache_seconds` (§4.2). Zero: not cached.
#[must_use]
pub fn cache_ttl(
    expires_in: Option<Duration>,
    source_ttl: Option<Duration>,
    max_cache_seconds: u32,
) -> Duration {
    let mut ttl = Duration::from_secs(u64::from(max_cache_seconds));
    if let Some(expires_in) = expires_in {
        ttl = ttl.min(expires_in.saturating_sub(EXPIRY_MARGIN));
    }
    if let Some(source_ttl) = source_ttl {
        ttl = ttl.min(source_ttl);
    }
    ttl
}

/// The cache key: never the raw token, and scoped to the cluster (its UID),
/// the IdP, the client and the audience (§5.2), so a user only ever gets the
/// token exchanged for their own (T4).
#[must_use]
pub fn cache_key(
    subject_token: &str,
    cluster_uid: &str,
    issuer_url: &str,
    client_id: &str,
    audience: &str,
) -> String {
    let mut hasher = Sha256::new();
    for part in [subject_token, cluster_uid, issuer_url, client_id, audience] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

/// Exchanged tokens, per replica, in memory only (decision 2). Its keys hold
/// the cluster's UID, so tests on distinct clusters never share an entry and
/// it stays on under `test-util`.
static EXCHANGED: LazyLock<TtlCache<String, Exchanged>> = LazyLock::new(TtlCache::default);

/// One lock per key in flight, so concurrent requests of one caller on one
/// cluster cost one exchange (§5.2).
static IN_FLIGHT: LazyLock<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

fn acquire(key: &str) -> Arc<AsyncMutex<()>> {
    let mut map = IN_FLIGHT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    map.entry(key.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

fn release(key: &str) {
    let mut map = IN_FLIGHT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if map
        .get(key)
        .is_some_and(|lock| Arc::strong_count(lock) == 1)
    {
        map.remove(key);
    }
}

/// Whether the token came from the cache or a real exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    CacheHit,
    Exchanged,
}

/// The exchanged token for `subject_token` on this cluster: from the cache,
/// or exchanged once behind the key's lock and cached for [`cache_ttl`].
///
/// # Errors
///
/// What [`exchange`] returns; failures are never cached.
pub async fn exchange_cached(
    http: &reqwest::Client,
    client: &ExchangeClient<'_>,
    settings: &TokenExchangeSettings,
    key: &str,
    subject_token: &str,
    source_ttl: Option<Duration>,
) -> Result<(Arc<SecretString>, Source), ExchangeError> {
    if let Some(hit) = EXCHANGED.get(&key.to_string()) {
        return Ok((hit.token, Source::CacheHit));
    }
    let lock = acquire(key);
    let outcome = async {
        let _guard = lock.lock().await;
        // Another request may have filled it while this one waited.
        if let Some(hit) = EXCHANGED.get(&key.to_string()) {
            return Ok((hit.token, Source::CacheHit));
        }
        let exchanged = exchange(http, client, settings, subject_token).await?;
        let ttl = cache_ttl(exchanged.expires_in, source_ttl, settings.max_cache_seconds);
        if !ttl.is_zero() {
            EXCHANGED.insert(key.to_string(), exchanged.clone(), ttl);
        }
        Ok((exchanged.token, Source::Exchanged))
    }
    .await;
    drop(lock);
    release(key);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret as _;

    fn settings() -> TokenExchangeSettings {
        TokenExchangeSettings::default()
    }

    fn pairs(body: &str) -> Vec<(String, String)> {
        form_urlencoded::parse(body.as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    #[test]
    fn the_form_omits_the_empty_parameters() {
        let body = form(&settings(), "tok&en");
        let pairs = pairs(&body);
        assert_eq!(
            pairs,
            [
                ("grant_type".to_string(), GRANT_TYPE.to_string()),
                ("subject_token".to_string(), "tok&en".to_string()),
                (
                    "subject_token_type".to_string(),
                    ACCESS_TOKEN_TYPE.to_string()
                ),
                (
                    "requested_token_type".to_string(),
                    ACCESS_TOKEN_TYPE.to_string()
                ),
            ]
        );
        let full = TokenExchangeSettings {
            audience: "kube-prod".to_string(),
            scope: "openid groups".to_string(),
            connector_id: "proxy".to_string(),
            ..settings()
        };
        let names: Vec<String> = super::tests::pairs(&form(&full, "t"))
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert!(names.ends_with(&[
            "audience".to_string(),
            "scope".to_string(),
            "connector_id".to_string()
        ]));
    }

    #[test]
    fn the_client_credentials_are_form_encoded_before_base64() {
        use base64::engine::general_purpose::STANDARD;
        let header = basic_authorization("kube:prod", "s3cr&t");
        let decoded = STANDARD
            .decode(header.trim_start_matches("Basic "))
            .unwrap();
        assert_eq!(decoded, b"kube%3Aprod:s3cr%26t");
    }

    #[test]
    fn the_answer_is_classified() {
        let ok = parse_response(200, br#"{"access_token":"x","expires_in":60}"#).unwrap();
        assert_eq!(ok.token.expose_secret(), "x");
        assert_eq!(ok.expires_in, Some(Duration::from_secs(60)));
        assert!(matches!(
            parse_response(503, b""),
            Err(ExchangeError::Unavailable(_))
        ));
        assert_eq!(
            parse_response(
                400,
                br#"{"error":"invalid_grant","error_description":"alice@corp"}"#
            )
            .err(),
            Some(ExchangeError::Refused(
                "status 400, invalid_grant".to_string()
            ))
        );
        assert!(matches!(
            parse_response(200, br#"{"token_type":"Bearer"}"#),
            Err(ExchangeError::Unreadable(_))
        ));
        assert!(matches!(
            parse_response(200, b"<html>"),
            Err(ExchangeError::Unreadable(_))
        ));
    }

    /// T9: never past `max_cache_seconds`, the token's `exp` or the caller's.
    #[test]
    fn the_cache_never_outlives_max_cache_seconds() {
        let s = Duration::from_secs;
        assert_eq!(cache_ttl(None, None, 300), s(300));
        assert_eq!(cache_ttl(Some(s(3600)), None, 300), s(300));
        assert_eq!(cache_ttl(Some(s(100)), None, 300), s(70));
        assert_eq!(cache_ttl(Some(s(10)), None, 300), s(0));
        assert_eq!(cache_ttl(Some(s(3600)), Some(s(42)), 300), s(42));
        assert_eq!(cache_ttl(Some(s(3600)), None, 0), s(0));
    }

    /// T4: two tokens, or one token on two clusters, never share a key.
    #[test]
    fn two_users_never_share_an_exchanged_token() {
        let key = |token, uid| cache_key(token, uid, "https://sso", "kube", "");
        assert_ne!(key("alice", "uid-1"), key("bob", "uid-1"));
        assert_ne!(key("alice", "uid-1"), key("alice", "uid-2"));
        // Length-prefixed: shifting bytes between parts changes the key.
        assert_ne!(
            cache_key("ab", "c", "i", "k", ""),
            cache_key("a", "bc", "i", "k", "")
        );
    }
}
