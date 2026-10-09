//! Local validation of a bearer token against the configured `jwt` authenticators.
//!
//! Mirrors what the apiserver's structured authentication configuration does:
//! pick the authenticator whose issuer the token names, verify the signature
//! against that issuer's JWKS, check the registered claims, then apply
//! `claim_validation_rules`, `claim_mappings` and `user_validation_rules` in that
//! order.
//!
//! The order matters and is not an implementation detail: nothing about a token
//! is trusted until the signature is verified, claim rules run on verified
//! claims, and user rules run on the *mapped* identity — which is what makes a
//! rule like `!user.username.startsWith('system:')` able to stop an IdP from
//! minting a privileged name.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crd::authentication_configuration::{
    ClaimMappings, ClaimOrExpression, ClaimValidationRule, JWTAuthenticator,
    PrefixedClaimOrExpression,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use tokio::sync::RwLock;
use tracing::instrument;

use crate::cel_rules::{MappedUser, evaluate_bool, evaluate_string, evaluate_string_list};
use crate::error::JwtValidationError;
use crate::jwks::JwksCache;

/// The algorithms a token may be signed with.
///
/// Asymmetric only, and deliberately so. A JWKS publishes *public* keys: if an
/// HMAC algorithm were accepted, anyone who can read the (public) JWKS could
/// mint a token that verifies against it — the classic algorithm-confusion
/// forgery. `none` is likewise absent, and is not representable here anyway.
const ALLOWED_ALGORITHMS: &[Algorithm] = &[
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::ES256,
    Algorithm::ES384,
    Algorithm::EdDSA,
];

/// Validates bearer tokens against a set of [`JWTAuthenticator`]s.
///
/// Holds the JWKS cache and the per-issuer HTTP clients, so it is built once and
/// shared: constructing one per request would defeat both caches.
#[derive(Debug, Default)]
pub struct JwtValidator {
    jwks: JwksCache,
    clients: RwLock<HashMap<(String, Option<String>), reqwest::Client>>,
}

impl JwtValidator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The HTTP client used to reach `authenticator`'s issuer.
    ///
    /// Cached because building a `reqwest::Client` (which sets up a TLS config
    /// and a connection pool) per request would be wasteful.
    ///
    /// The key includes the CA bundle, not just the issuer URL. One validator is
    /// shared by every `ProxyKubeApi` in the process, so two proxies may trust
    /// the same issuer while only one pins a `certificate_authority` — keyed on
    /// the URL alone, whichever request arrived first would decide, and the pin
    /// would silently do nothing. Including the bundle also means editing it on
    /// a live CR takes effect on the next request instead of at the next
    /// restart.
    async fn client_for(
        &self,
        authenticator: &JWTAuthenticator,
    ) -> Result<reqwest::Client, JwtValidationError> {
        let key = client_cache_key(authenticator);
        if let Some(client) = self.clients.read().await.get(&key) {
            return Ok(client.clone());
        }

        let mut builder = reqwest::Client::builder();
        if let Some(ca) = &authenticator.issuer.certificate_authority {
            let cert = reqwest::Certificate::from_pem(ca.as_bytes()).map_err(|e| {
                JwtValidationError::Configuration(format!(
                    "issuer.certificate_authority is not a valid PEM certificate: {e}"
                ))
            })?;
            builder = builder.add_root_certificate(cert);
        }
        let client = builder.build().map_err(|e| {
            JwtValidationError::Configuration(format!(
                "could not build the issuer HTTP client: {e}"
            ))
        })?;

        self.clients.write().await.insert(key, client.clone());
        Ok(client)
    }

    /// Validate `token` against `authenticators` and return the identity it maps to.
    #[instrument(skip_all, fields(issuer))]
    pub async fn validate(
        &self,
        token: &str,
        authenticators: &[JWTAuthenticator],
    ) -> Result<MappedUser, JwtValidationError> {
        let header = decode_header(token)
            .map_err(|e| JwtValidationError::Malformed(format!("unreadable header: {e}")))?;

        if !ALLOWED_ALGORITHMS.contains(&header.alg) {
            return Err(JwtValidationError::UnsupportedAlgorithm {
                alg: format!("{:?}", header.alg),
            });
        }

        // Read `iss` from the *unverified* payload purely to choose which
        // authenticator to try. Nothing is trusted from it: the issuer is
        // re-checked against that authenticator's configured URL as part of the
        // signature verification below, so a forged `iss` selects an
        // authenticator whose key then fails to verify the token.
        let unverified_issuer = unverified_issuer(token)?;
        tracing::Span::current().record("issuer", &unverified_issuer);

        let authenticator = authenticators
            .iter()
            .find(|candidate| candidate.issuer.url == unverified_issuer)
            .ok_or_else(|| JwtValidationError::UnknownIssuer {
                issuer: unverified_issuer.clone(),
            })?;

        let client = self.client_for(authenticator).await?;
        let jwk = self
            .jwks
            .key_for(
                &client,
                &authenticator.issuer.discovery_endpoint(),
                header.kid.as_deref(),
            )
            .await?;

        // Prefer the algorithm the key itself declares over the one the token
        // asks for: the header is attacker-controlled, the JWKS is not.
        let algorithm = match jwk.common.key_algorithm {
            Some(declared) => Algorithm::try_from(declared).map_err(|_| {
                JwtValidationError::UnsupportedAlgorithm {
                    alg: format!("{declared:?}"),
                }
            })?,
            None => header.alg,
        };
        if !ALLOWED_ALGORITHMS.contains(&algorithm) {
            return Err(JwtValidationError::UnsupportedAlgorithm {
                alg: format!("{algorithm:?}"),
            });
        }

        let decoding_key =
            DecodingKey::from_jwk(&jwk).map_err(|e| JwtValidationError::JwksInvalid {
                url: authenticator.issuer.url.clone(),
                reason: format!("key is not usable for verification: {e}"),
            })?;

        let mut validation = Validation::new(algorithm);
        validation.validate_exp = true;
        validation.validate_nbf = true;
        validation.set_issuer(&[&authenticator.issuer.url]);
        // `MatchAny` — the only policy the apiserver defines — is exactly
        // jsonwebtoken's "any of these audiences" semantics.
        validation.set_audience(&authenticator.issuer.audiences);

        let claims = decode::<serde_json::Value>(token, &decoding_key, &validation)
            .map_err(|e| JwtValidationError::Rejected(e.to_string()))?
            .claims;

        for rule in &authenticator.claim_validation_rules {
            apply_claim_rule(rule, &claims)?;
        }

        let user = map_user(&authenticator.claim_mappings, &claims)?;

        for rule in &authenticator.user_validation_rules {
            let user_value = serde_json::to_value(&user).map_err(|e| {
                JwtValidationError::ExpressionEvaluate {
                    expression: rule.expression.clone(),
                    reason: format!("could not bind user: {e}"),
                }
            })?;
            if !evaluate_bool(&rule.expression, "user", &user_value)? {
                return Err(JwtValidationError::UserRuleRejected {
                    message: rule.message.clone(),
                });
            }
        }

        Ok(user)
    }
}

/// Cache key for [`JwtValidator::client_for`]: the issuer URL *and* the CA
/// bundle that must be trusted to reach it.
fn client_cache_key(authenticator: &JWTAuthenticator) -> (String, Option<String>) {
    (
        authenticator.issuer.url.clone(),
        authenticator.issuer.certificate_authority.clone(),
    )
}

/// Read `iss` out of a token's payload without verifying anything.
fn unverified_issuer(token: &str) -> Result<String, JwtValidationError> {
    use base64::Engine;

    let payload = token.split('.').nth(1).ok_or_else(|| {
        JwtValidationError::Malformed("not three dot-separated parts".to_string())
    })?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|e| JwtValidationError::Malformed(format!("payload is not base64url: {e}")))?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|e| JwtValidationError::Malformed(format!("payload is not JSON: {e}")))?;
    claims
        .get("iss")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| JwtValidationError::Malformed("payload has no iss claim".to_string()))
}

/// Apply one `claim_validation_rules` entry to the verified claims.
pub(crate) fn apply_claim_rule(
    rule: &ClaimValidationRule,
    claims: &serde_json::Value,
) -> Result<(), JwtValidationError> {
    // The two shapes are mutually exclusive (admission and `validate()` both say
    // so). Taking the `claim` branch when both are set would silently skip the
    // `expression` the operator wrote as a security control, so refuse instead —
    // a `Configuration` error is not the caller's fault and fails closed.
    if rule.claim.is_some() && rule.expression.is_some() {
        return Err(JwtValidationError::Configuration(
            "a claim validation rule sets both claim and expression".to_string(),
        ));
    }
    if let Some(claim) = &rule.claim {
        let value = claims.get(claim).and_then(serde_json::Value::as_str);
        let matches = match value {
            // An empty required_value means "the claim must merely be present",
            // which is how the apiserver reads it.
            Some(_) if rule.required_value.is_empty() => true,
            Some(value) => value == rule.required_value,
            None => false,
        };
        if !matches {
            return Err(JwtValidationError::ClaimRuleRejected {
                message: if rule.message.is_empty() {
                    format!("claim {claim} does not have the required value")
                } else {
                    rule.message.clone()
                },
            });
        }
        return Ok(());
    }

    let expression = rule.expression.as_ref().ok_or_else(|| {
        JwtValidationError::Configuration(
            "a claim validation rule has neither claim nor expression".to_string(),
        )
    })?;
    if evaluate_bool(expression, "claims", claims)? {
        Ok(())
    } else {
        Err(JwtValidationError::ClaimRuleRejected {
            message: rule.message.clone(),
        })
    }
}

/// Turn verified claims into the identity the proxy will authorize.
pub(crate) fn map_user(
    mappings: &ClaimMappings,
    claims: &serde_json::Value,
) -> Result<MappedUser, JwtValidationError> {
    let username_mapping = mappings.username.as_ref().ok_or_else(|| {
        JwtValidationError::Configuration("claim_mappings.username is required".to_string())
    })?;

    let username = map_prefixed_string(username_mapping, claims, "username")?;
    // An empty username is an anonymous identity, not a user: the proxy would go
    // on to authorize it, rate-limit every such caller under one empty subject
    // and forward `x-forwarded-user: ""` upstream. The apiserver refuses it for
    // the same reason.
    if username.is_empty() {
        return Err(JwtValidationError::Rejected(
            "the claim mappings produced an empty username".to_string(),
        ));
    }

    let mut user = MappedUser {
        username,
        ..MappedUser::default()
    };

    if let Some(groups) = &mappings.groups {
        user.groups = map_prefixed_string_list(groups, claims, "groups")?;
    }
    if let Some(uid) = &mappings.uid {
        user.uid = map_claim_or_expression(uid, claims, "uid")?;
    }

    let mut extra: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for mapping in &mappings.extra {
        let values = evaluate_string_list(&mapping.value_expression, "claims", claims)?;
        if !values.is_empty() {
            extra.insert(mapping.key.clone(), values);
        }
    }
    user.extra = extra;

    Ok(user)
}

fn map_prefixed_string(
    mapping: &PrefixedClaimOrExpression,
    claims: &serde_json::Value,
    field: &'static str,
) -> Result<String, JwtValidationError> {
    if let Some(claim) = &mapping.claim {
        let value = claims
            .get(claim)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| JwtValidationError::ClaimUnusable {
                claim: claim.clone(),
                want: "a string",
            })?;
        return Ok(format!(
            "{}{value}",
            mapping.prefix.as_deref().unwrap_or_default()
        ));
    }
    let expression = mapping.expression.as_ref().ok_or_else(|| {
        JwtValidationError::Configuration(format!(
            "claim_mappings.{field} has neither claim nor expression"
        ))
    })?;
    evaluate_string(expression, "claims", claims)
}

fn map_prefixed_string_list(
    mapping: &PrefixedClaimOrExpression,
    claims: &serde_json::Value,
    field: &'static str,
) -> Result<Vec<String>, JwtValidationError> {
    if let Some(claim) = &mapping.claim {
        let prefix = mapping.prefix.as_deref().unwrap_or_default();
        let Some(value) = claims.get(claim) else {
            // Groups are optional in a way a username is not: a token with no
            // group claim is a user in no groups, not a broken token.
            return Ok(Vec::new());
        };
        return match value {
            serde_json::Value::String(value) => Ok(vec![format!("{prefix}{value}")]),
            serde_json::Value::Array(values) => values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(|value| format!("{prefix}{value}"))
                        .ok_or_else(|| JwtValidationError::ClaimUnusable {
                            claim: claim.clone(),
                            want: "a string or list of strings",
                        })
                })
                .collect(),
            serde_json::Value::Null => Ok(Vec::new()),
            _ => Err(JwtValidationError::ClaimUnusable {
                claim: claim.clone(),
                want: "a string or list of strings",
            }),
        };
    }
    let expression = mapping.expression.as_ref().ok_or_else(|| {
        JwtValidationError::Configuration(format!(
            "claim_mappings.{field} has neither claim nor expression"
        ))
    })?;
    evaluate_string_list(expression, "claims", claims)
}

fn map_claim_or_expression(
    mapping: &ClaimOrExpression,
    claims: &serde_json::Value,
    field: &'static str,
) -> Result<String, JwtValidationError> {
    if let Some(claim) = &mapping.claim {
        return claims
            .get(claim)
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| JwtValidationError::ClaimUnusable {
                claim: claim.clone(),
                want: "a string",
            });
    }
    let expression = mapping.expression.as_ref().ok_or_else(|| {
        JwtValidationError::Configuration(format!(
            "claim_mappings.{field} has neither claim nor expression"
        ))
    })?;
    evaluate_string(expression, "claims", claims)
}

/// `Arc` so the validator can be shared by every request handler.
pub type SharedJwtValidator = Arc<JwtValidator>;

#[cfg(test)]
mod tests {
    use super::{ALLOWED_ALGORITHMS, apply_claim_rule, map_user, unverified_issuer};
    use crate::error::JwtValidationError;
    use crd::authentication_configuration::{
        ClaimMappings, ClaimOrExpression, ClaimValidationRule, ExtraMapping,
        PrefixedClaimOrExpression,
    };
    use jsonwebtoken::Algorithm;
    use serde_json::json;

    fn claims() -> serde_json::Value {
        json!({
            "iss": "https://issuer.example.com",
            "sub": "1234",
            "email": "alice@example.com",
            "hd": "example.com",
            "groups": ["dev", "ops"],
            "team": "platform",
            "uid_claim": "u-1234",
        })
    }

    fn claim_rule(claim: &str, required_value: &str) -> ClaimValidationRule {
        ClaimValidationRule {
            claim: Some(claim.to_string()),
            required_value: required_value.to_string(),
            expression: None,
            message: String::new(),
        }
    }

    fn expression_rule(expression: &str, message: &str) -> ClaimValidationRule {
        ClaimValidationRule {
            claim: None,
            required_value: String::new(),
            expression: Some(expression.to_string()),
            message: message.to_string(),
        }
    }

    fn claim_mapping(claim: &str, prefix: Option<&str>) -> PrefixedClaimOrExpression {
        PrefixedClaimOrExpression {
            prefix: prefix.map(ToString::to_string),
            claim: Some(claim.to_string()),
            expression: None,
        }
    }

    fn expression_mapping(expression: &str) -> PrefixedClaimOrExpression {
        PrefixedClaimOrExpression {
            prefix: None,
            claim: None,
            expression: Some(expression.to_string()),
        }
    }

    fn mappings(username: PrefixedClaimOrExpression) -> ClaimMappings {
        ClaimMappings {
            username: Some(username),
            ..ClaimMappings::default()
        }
    }

    /// A JWKS publishes public keys, so accepting an HMAC algorithm would let
    /// anyone who can read it mint a token that verifies — the classic
    /// algorithm-confusion forgery.
    #[test]
    fn no_symmetric_algorithm_is_ever_accepted() {
        for symmetric in [Algorithm::HS256, Algorithm::HS384, Algorithm::HS512] {
            assert!(
                !ALLOWED_ALGORITHMS.contains(&symmetric),
                "{symmetric:?} must not be accepted"
            );
        }
        assert!(ALLOWED_ALGORITHMS.contains(&Algorithm::RS256));
        assert!(ALLOWED_ALGORITHMS.contains(&Algorithm::ES256));
    }

    #[test]
    fn a_claim_and_required_value_rule_matches_exactly() {
        assert!(apply_claim_rule(&claim_rule("hd", "example.com"), &claims()).is_ok());

        let err = apply_claim_rule(&claim_rule("hd", "other.com"), &claims()).unwrap_err();
        assert!(
            matches!(err, JwtValidationError::ClaimRuleRejected { .. }),
            "unexpected error: {err}"
        );
    }

    /// An empty `required_value` means "the claim must merely be present", which
    /// is how the apiserver reads it.
    #[test]
    fn an_empty_required_value_only_demands_the_claim_is_present() {
        assert!(apply_claim_rule(&claim_rule("hd", ""), &claims()).is_ok());
        assert!(apply_claim_rule(&claim_rule("absent", ""), &claims()).is_err());
    }

    /// A missing claim must reject, never pass by comparing against nothing.
    #[test]
    fn a_missing_claim_rejects_rather_than_matching() {
        let err = apply_claim_rule(&claim_rule("absent", "whatever"), &claims()).unwrap_err();
        assert!(
            matches!(err, JwtValidationError::ClaimRuleRejected { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn an_expression_rule_reports_its_configured_message() {
        assert!(
            apply_claim_rule(
                &expression_rule("claims.hd == 'example.com'", "wrong domain"),
                &claims()
            )
            .is_ok()
        );

        let err = apply_claim_rule(
            &expression_rule("claims.hd == 'other.com'", "wrong domain"),
            &claims(),
        )
        .unwrap_err();
        assert!(
            matches!(err, JwtValidationError::ClaimRuleRejected { ref message } if message == "wrong domain"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_username_is_mapped_from_a_claim_with_its_prefix() {
        let user = map_user(&mappings(claim_mapping("sub", Some("oidc:"))), &claims()).unwrap();
        assert_eq!(user.username, "oidc:1234");

        let user = map_user(&mappings(claim_mapping("sub", None)), &claims()).unwrap();
        assert_eq!(user.username, "1234");
    }

    #[test]
    fn a_username_is_mapped_from_an_expression() {
        let user = map_user(
            &mappings(expression_mapping("claims.email + ':external'")),
            &claims(),
        )
        .unwrap();
        assert_eq!(user.username, "alice@example.com:external");
    }

    /// The prefix is what keeps an external IdP from minting a username that
    /// collides with a local one, so it must reach every group, not just the
    /// first.
    #[test]
    fn every_group_gets_the_prefix() {
        let mut config = mappings(claim_mapping("sub", None));
        config.groups = Some(claim_mapping("groups", Some("oidc:")));

        let user = map_user(&config, &claims()).unwrap();
        assert_eq!(user.groups, vec!["oidc:dev", "oidc:ops"]);
    }

    /// A group claim holding a bare string is a user in one group — the shape
    /// several providers emit.
    #[test]
    fn a_single_string_group_claim_becomes_one_group() {
        let mut config = mappings(claim_mapping("sub", None));
        config.groups = Some(claim_mapping("team", None));

        assert_eq!(
            map_user(&config, &claims()).unwrap().groups,
            vec!["platform"]
        );
    }

    /// Groups are optional in a way a username is not: no group claim is a user
    /// in no groups, not a broken token.
    #[test]
    fn an_absent_group_claim_maps_to_no_groups() {
        let mut config = mappings(claim_mapping("sub", None));
        config.groups = Some(claim_mapping("absent", Some("oidc:")));

        assert!(map_user(&config, &claims()).unwrap().groups.is_empty());
    }

    #[test]
    fn a_missing_username_claim_is_an_error_rather_than_an_empty_user() {
        let err = map_user(&mappings(claim_mapping("absent", None)), &claims()).unwrap_err();
        assert!(
            matches!(err, JwtValidationError::ClaimUnusable { .. }),
            "unexpected error: {err}"
        );
    }

    /// An empty username is an anonymous identity the proxy would then
    /// authorize, rate-limit under one shared empty subject and forward
    /// upstream as `x-forwarded-user: ""`.
    #[test]
    fn an_empty_mapped_username_is_refused() {
        let claims = json!({ "iss": "https://issuer.example.com", "sub": "" });
        let err = map_user(&mappings(claim_mapping("sub", None)), &claims).unwrap_err();
        assert!(
            matches!(err, JwtValidationError::Rejected(_)),
            "unexpected error: {err}"
        );

        let err = map_user(&mappings(expression_mapping("''")), &claims).unwrap_err();
        assert!(
            matches!(err, JwtValidationError::Rejected(_)),
            "unexpected error: {err}"
        );
    }

    /// Taking the `claim` branch would silently skip an `expression` the
    /// operator wrote as a security control, so a rule carrying both must fail
    /// closed instead.
    #[test]
    fn a_rule_setting_both_shapes_fails_closed() {
        let rule = ClaimValidationRule {
            claim: Some("hd".to_string()),
            required_value: "example.com".to_string(),
            expression: Some("claims.hd == 'never-matches'".to_string()),
            message: "nope".to_string(),
        };
        let err = apply_claim_rule(&rule, &claims()).unwrap_err();
        assert!(
            matches!(err, JwtValidationError::Configuration(_)),
            "unexpected error: {err}"
        );
        assert!(!err.is_caller_fault());
    }

    #[test]
    fn uid_and_extra_are_mapped() {
        let mut config = mappings(claim_mapping("sub", None));
        config.uid = Some(ClaimOrExpression {
            claim: Some("uid_claim".to_string()),
            expression: None,
        });
        config.extra = vec![ExtraMapping {
            key: "example.org/team".to_string(),
            value_expression: "claims.team".to_string(),
        }];

        let user = map_user(&config, &claims()).unwrap();
        assert_eq!(user.uid, "u-1234");
        assert_eq!(
            user.extra.get("example.org/team"),
            Some(&vec!["platform".to_string()])
        );
    }

    /// One validator serves every `ProxyKubeApi` in the process. Keyed on the
    /// issuer URL alone, two proxies trusting the same issuer with different CA
    /// pinning would share whichever client was built first — and the pin would
    /// silently not apply.
    #[test]
    fn the_client_cache_key_separates_different_ca_bundles() {
        use super::client_cache_key;
        use crd::authentication_configuration::{ClaimMappings, Issuer, JWTAuthenticator};

        let authenticator = |ca: Option<&str>| JWTAuthenticator {
            issuer: Issuer {
                url: "https://issuer.example.com".to_string(),
                discovery_url: None,
                certificate_authority: ca.map(ToString::to_string),
                audiences: vec!["proxyauthk8s".to_string()],
                audience_match_policy:
                    crd::authentication_configuration::AudienceMatchPolicyType::MatchAny,
                egress_selector:
                    crd::authentication_configuration::EgressSelectorType::ControlPlane,
            },
            claim_validation_rules: Vec::new(),
            claim_mappings: ClaimMappings::default(),
            user_validation_rules: Vec::new(),
        };

        let unpinned = client_cache_key(&authenticator(None));
        let pinned = client_cache_key(&authenticator(Some("-----BEGIN CERTIFICATE-----")));
        let other_pin = client_cache_key(&authenticator(Some("-----BEGIN OTHER-----")));

        assert_ne!(unpinned, pinned);
        assert_ne!(pinned, other_pin);
        assert_eq!(
            pinned,
            client_cache_key(&authenticator(Some("-----BEGIN CERTIFICATE-----")))
        );
    }

    #[test]
    fn the_unverified_issuer_is_read_from_the_payload() {
        // header.payload.signature, payload being {"iss":"https://issuer.example.com"}
        let token = "eyJhbGciOiJSUzI1NiJ9.eyJpc3MiOiJodHRwczovL2lzc3Vlci5leGFtcGxlLmNvbSJ9.sig"; // gitleaks:allow
        assert_eq!(
            unverified_issuer(token).unwrap(),
            "https://issuer.example.com"
        );
    }

    #[test]
    fn a_token_without_an_issuer_is_malformed() {
        // payload is {"sub":"1234"}
        let token = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiIxMjM0In0.sig"; // gitleaks:allow
        assert!(matches!(
            unverified_issuer(token).unwrap_err(),
            JwtValidationError::Malformed(_)
        ));
        assert!(matches!(
            unverified_issuer("not-a-jwt").unwrap_err(),
            JwtValidationError::Malformed(_)
        ));
    }
}
