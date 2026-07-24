//! Access-token audience validation.
//!
//! Calling `/userinfo` proves a bearer token is validly signed and active for
//! its issuer, but it does NOT restrict which OAuth client the token was minted
//! for. Without an audience check, any access token from the same issuer (for
//! example one issued for a different application on a shared IdP) would be
//! accepted and group-authorized here — a confused-deputy / audience-confusion
//! hole. This module enforces that the token's audience actually names this
//! service.
//!
//! Two mechanisms are used (see [`crate::oidc_conf::OidcConf::ensure_token_audience`]):
//!   1. RFC 7662 introspection, when the provider advertises an endpoint;
//!   2. reading the `aud`/`azp`/`client_id` claims directly from the token when
//!      it is a JWT — sound here because `/userinfo` already validated the
//!      signature, so the claims cannot have been tampered with.

use base64::Engine;
use serde::Deserialize;

/// How strictly to enforce audience validation, from `OIDC_AUDIENCE_VALIDATION`.
///
/// Defaults to [`AudienceValidationMode::Enforce`]. `warn` logs a mismatch but
/// lets the request through (useful to observe impact before enforcing), and
/// `off` disables the check entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudienceValidationMode {
    Enforce,
    Warn,
    Off,
}

impl AudienceValidationMode {
    pub fn from_env() -> Self {
        Self::from_str_value(&std::env::var("OIDC_AUDIENCE_VALIDATION").unwrap_or_default())
    }

    fn from_str_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "false" | "disabled" | "none" => Self::Off,
            "warn" | "warning" | "log" => Self::Warn,
            // Anything else (including empty / typos) fails closed to Enforce.
            _ => Self::Enforce,
        }
    }
}

/// Audiences carried by a token, gathered from `aud`, `azp` and `client_id`.
#[derive(Debug, Default, Clone)]
pub struct TokenAudiences {
    pub values: Vec<String>,
}

impl TokenAudiences {
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Whether the expected audience is present in the token's audiences.
    pub fn contains(&self, expected: &str) -> bool {
        self.values.iter().any(|value| value == expected)
    }
}

/// `aud` may be a single string or an array of strings; accept both shapes.
#[derive(Deserialize)]
#[serde(untagged)]
enum AudField {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
struct JwtAudClaims {
    aud: Option<AudField>,
    azp: Option<String>,
    client_id: Option<String>,
}

/// Parse the audiences from a JWT access token WITHOUT verifying its signature.
///
/// This is only sound to call once the token has already been validated (here,
/// by a successful `/userinfo` round-trip): the issuer signed the claims, so an
/// attacker cannot alter `aud` without invalidating that signature. Returns
/// `None` when the token is not a JWT (opaque) or carries no audience-like claim.
pub fn extract_jwt_audiences(token: &str) -> Option<TokenAudiences> {
    // A JWS/JWT is `header.payload.signature`; the payload is the middle part.
    let payload_b64 = token.split('.').nth(1)?;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let claims: JwtAudClaims = serde_json::from_slice(&payload).ok()?;

    let mut values = Vec::new();
    match claims.aud {
        Some(AudField::One(aud)) => values.push(aud),
        Some(AudField::Many(mut auds)) => values.append(&mut auds),
        None => {}
    }
    if let Some(azp) = claims.azp {
        values.push(azp);
    }
    if let Some(client_id) = claims.client_id {
        values.push(client_id);
    }

    if values.is_empty() {
        None
    } else {
        Some(TokenAudiences { values })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn jwt_with_payload(payload: &str) -> String {
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        format!(
            "{}.{}.{}",
            b64.encode(b"{\"alg\":\"RS256\"}"),
            b64.encode(payload.as_bytes()),
            b64.encode(b"sig")
        )
    }

    #[test]
    fn mode_parsing_fails_closed_to_enforce() {
        assert_eq!(
            AudienceValidationMode::from_str_value("off"),
            AudienceValidationMode::Off
        );
        assert_eq!(
            AudienceValidationMode::from_str_value("WARN"),
            AudienceValidationMode::Warn
        );
        assert_eq!(
            AudienceValidationMode::from_str_value(""),
            AudienceValidationMode::Enforce
        );
        assert_eq!(
            AudienceValidationMode::from_str_value("garbage"),
            AudienceValidationMode::Enforce
        );
    }

    #[test]
    fn extracts_single_string_aud() {
        let token = jwt_with_payload(r#"{"aud":"proxy-auth-k8s","sub":"alice"}"#);
        let auds = extract_jwt_audiences(&token).unwrap();
        assert!(auds.contains("proxy-auth-k8s"));
        assert!(!auds.contains("other-app"));
    }

    #[test]
    fn extracts_array_aud_and_azp() {
        let token = jwt_with_payload(r#"{"aud":["a","b"],"azp":"proxy-auth-k8s"}"#);
        let auds = extract_jwt_audiences(&token).unwrap();
        assert!(auds.contains("a"));
        assert!(auds.contains("b"));
        assert!(auds.contains("proxy-auth-k8s"));
    }

    #[test]
    fn falls_back_to_client_id_claim() {
        let token = jwt_with_payload(r#"{"client_id":"proxy-auth-k8s"}"#);
        let auds = extract_jwt_audiences(&token).unwrap();
        assert!(auds.contains("proxy-auth-k8s"));
    }

    #[test]
    fn opaque_or_audienceless_tokens_return_none() {
        // Not a JWT.
        assert!(extract_jwt_audiences("opaque-random-token").is_none());
        // JWT with no audience-like claim.
        let token = jwt_with_payload(r#"{"sub":"alice"}"#);
        assert!(extract_jwt_audiences(&token).is_none());
    }
}
