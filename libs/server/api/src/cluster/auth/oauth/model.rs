//! Redis-backed state for the mediated OAuth Authorization Server flow.
//!
//! Two short-lived, single-use records:
//!
//! - [`PendingAuthorization`]: written by `/oauth/authorize`, keyed by the
//!   correlation id sent to the upstream provider as its own `state`. Holds
//!   everything needed to resume once the upstream provider calls back.
//! - [`IssuedCode`]: written by `/oauth/callback` once the upstream exchange
//!   succeeds, keyed by a proxy-minted authorization code handed to the
//!   external client. Holds the upstream tokens `/oauth/token` will release
//!   once the external client proves possession of the PKCE verifier.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Redis key prefix for [`PendingAuthorization`] records.
pub const PENDING_PREFIX: &str = "oauth_as_pending";
/// Redis key prefix for [`IssuedCode`] records.
pub const CODE_PREFIX: &str = "oauth_as_code";

/// How long a caller has to complete the upstream login before `/oauth/authorize`'s
/// state expires.
pub const PENDING_TTL_SECONDS: u64 = 300;
/// How long the external client has to redeem the proxy-minted code at
/// `/oauth/token`. Short: the code is single-use and the redirect to the
/// client is immediate.
pub const CODE_TTL_SECONDS: u64 = 60;

#[derive(Serialize, Deserialize)]
pub struct PendingAuthorization {
    /// The external client's own `redirect_uri`, already validated as loopback-only.
    pub client_redirect_uri: String,
    /// The external client's opaque `state`, echoed back verbatim on redirect.
    pub client_state: Option<String>,
    /// The external client's PKCE `code_challenge` (always S256).
    pub client_code_challenge: String,
    /// Nonce sent to the upstream provider, verified against its ID token.
    pub upstream_nonce: String,
    /// PKCE verifier for the proxy's own (server-side) exchange with the upstream provider.
    pub upstream_pkce_verifier: String,
}

#[derive(Serialize, Deserialize)]
pub struct IssuedCode {
    /// Must match the `redirect_uri` the external client presents at `/oauth/token`
    /// (RFC 6749 §4.1.3): binds the code to the request that requested it.
    pub redirect_uri: String,
    pub code_challenge: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: String,
    pub scope: String,
    pub expires_in: Option<i64>,
}

/// A loopback-only `redirect_uri`, per RFC 8252 (OAuth 2.0 for Native Apps).
///
/// The proxy does not register external clients, so it cannot pin a
/// `redirect_uri` to a known client the way a normal OAuth server would; an
/// attacker-supplied `redirect_uri` on a non-loopback host would turn this
/// endpoint into an open redirect that leaks a real user's authorization code
/// (PKCE does not help here: the attacker crafts the whole `/oauth/authorize`
/// URL, including its own `code_challenge`). Restricting to loopback confines
/// that attack to a caller who can already bind a listener on the victim's
/// own machine — a fundamentally different, already-compromised threat model.
#[must_use]
pub fn parse_loopback_redirect_uri(raw: &str) -> Option<Url> {
    let url = Url::parse(raw).ok()?;
    if url.scheme() != "http" {
        return None;
    }
    // `host_str` keeps the brackets of an IPv6 literal, so `::1` reads `[::1]`.
    if !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) {
        return None;
    }
    if url.fragment().is_some() {
        return None;
    }
    Some(url)
}

/// Whether the `redirect_uri` sent to `/oauth/token` is the one the code was
/// issued for (RFC 6749 §4.1.3).
///
/// The issued URI was stored in its normalised form (`Url::to_string`), so the
/// presented one is normalised the same way before comparing: otherwise
/// `http://localhost:8000` (stored as `http://localhost:8000/`), an explicit
/// `:80` or different escaping would be refused although it is the same URI.
#[must_use]
pub fn redirect_uri_matches(presented: &str, issued: &str) -> bool {
    parse_loopback_redirect_uri(presented).is_some_and(|url| url.as_str() == issued)
}

/// A PKCE `code_challenge`/`code_verifier` value's charset, per RFC 7636 §4.1
/// (`unreserved` characters) and length bounds.
#[must_use]
pub fn is_valid_pkce_value(value: &str) -> bool {
    (43..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// Verify a PKCE `code_verifier` against a stored `S256` `code_challenge`
/// (RFC 7636 §4.6): `code_challenge == BASE64URL-NOPAD(SHA256(code_verifier))`.
#[must_use]
pub fn verify_pkce_s256(verifier: &str, challenge: &str) -> bool {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest) == challenge
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_loopback_redirect_uri() {
        assert!(parse_loopback_redirect_uri("http://localhost:8000/callback").is_some());
        assert!(parse_loopback_redirect_uri("http://127.0.0.1:9999/").is_some());
        assert!(parse_loopback_redirect_uri("http://[::1]:9999/").is_some());
    }

    #[test]
    fn redirect_uri_is_compared_in_its_normalised_form() {
        let stored = parse_loopback_redirect_uri("http://localhost:8000")
            .unwrap()
            .to_string();
        assert!(redirect_uri_matches("http://localhost:8000", &stored));
        assert!(redirect_uri_matches("http://localhost:8000/", &stored));
        let stored = parse_loopback_redirect_uri("http://localhost/cb")
            .unwrap()
            .to_string();
        assert!(redirect_uri_matches("http://localhost:80/cb", &stored));
        assert!(!redirect_uri_matches("http://localhost:8001/cb", &stored));
        assert!(!redirect_uri_matches("not a url", &stored));
    }

    #[test]
    fn rejects_a_non_loopback_redirect_uri() {
        assert!(parse_loopback_redirect_uri("https://evil.example.com/steal").is_none());
        assert!(parse_loopback_redirect_uri("http://localhost.evil.com/").is_none());
        assert!(parse_loopback_redirect_uri("http://localhost:8000/callback#frag").is_none());
        assert!(parse_loopback_redirect_uri("not a url").is_none());
    }

    #[test]
    fn pkce_value_length_and_charset_are_enforced() {
        assert!(is_valid_pkce_value(&"a".repeat(43)));
        assert!(is_valid_pkce_value(&"a".repeat(128)));
        assert!(!is_valid_pkce_value(&"a".repeat(42)));
        assert!(!is_valid_pkce_value(&"a".repeat(129)));
        assert!(!is_valid_pkce_value(&format!("{}!", "a".repeat(42))));
    }

    #[test]
    fn pkce_s256_matches_the_rfc7636_appendix_b_vector() {
        // https://www.rfc-editor.org/rfc/rfc7636#appendix-B
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(verify_pkce_s256(verifier, challenge));
        assert!(!verify_pkce_s256(
            "wrong-verifier-wrong-verifier-wrong-verifi",
            challenge
        ));
    }
}
