mod claim_mappings;
mod claim_validation_rules;
mod issuer;
mod jwt_authenticator;
pub mod oidc_config_source;
mod oidc_provider;
mod user_validation_rule;
mod validate_against;

pub use claim_mappings::{
    ClaimMappings, ClaimOrExpression, ExtraMapping, PrefixedClaimOrExpression,
};
pub use claim_validation_rules::ClaimValidationRule;
pub use issuer::{AudienceMatchPolicyType, EgressSelectorType, Issuer};
pub use jwt_authenticator::JWTAuthenticator;
pub use oidc_config_source::{OidcConfigError, OidcConfigSource, OidcProviderOverrides};
pub use oidc_provider::OidcProvider;
pub use user_validation_rule::UserValidationRule;
pub use validate_against::ValidateAgainst;

use crate::default::{default_disabled, default_empty_array, default_validate_against};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Mirrors [`AuthenticationConfiguration::validate`] as a CEL admission rule, so
/// an invalid CR is refused by the apiserver instead of being accepted and then
/// failing in the resource status.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "self.validate_against != 'OidcProvider' || self.oidc_provider.enabled",
        "message": "validate_against is set to OidcProvider but the OIDC provider is not enabled",
    }),
    serde_json::json!({
        "rule": "self.validate_against != 'JwtAuthenticators' || size(self.jwt) > 0",
        "message": "validate_against is set to JwtAuthenticators but no jwt authenticator is configured",
    }),
    // The CEL mirror of the duplicate-issuer check in `validate` below. Without
    // it, the Rust check and admission drift apart: a duplicate would be
    // admitted and only surface later as a status error, which is exactly the
    // split this struct's doc comment says must not exist.
    //
    // `exists_one` over the list is the standard uniqueness idiom. It is
    // quadratic, so `jwt` carries a `maxItems` — the apiserver estimates a
    // rule's cost from the schema's bounds and rejects the CRD outright when an
    // unbounded list makes that estimate exceed the budget.
    serde_json::json!({
        "rule": "self.jwt.all(a, self.jwt.exists_one(b, b.issuer.url == a.issuer.url))",
        "message": "two jwt authenticators are configured for the same issuer; only the first would ever apply",
    }),
]))]
pub struct AuthenticationConfiguration {
    /// Bounded so the quadratic uniqueness rule above stays inside the
    /// apiserver's CEL cost budget — the estimator multiplies this bound by the
    /// bound on `issuer.url`, and an unbounded pair is rejected outright. Still
    /// far above any real deployment, which trusts a handful of issuers.
    #[serde(default = "default_empty_array::<JWTAuthenticator>")]
    #[schemars(length(max = 8))]
    pub jwt: Vec<JWTAuthenticator>,
    pub oidc_provider: OidcProvider,

    /// Disable validation of the token against the configured JWT authenticators, OIDC provider or Kubernetes API
    /// If the `AuthenticationConfiguration` is not provided, does not validate the token against any of the configured JWT authenticators, OIDC provider or Kubernetes API
    /// Default : false
    #[serde(default = "default_disabled")]
    pub disable_validation: bool,
    /// Validate against the configured JWT authenticators, OIDC provider or Kubernetes API
    /// Default : `Kubernetes` — the fail-closed choice, valid for every cluster.
    /// Selecting `OidcProvider` or `JwtAuthenticators` is always explicit, so a
    /// cluster never silently changes how it authenticates because a field was
    /// added elsewhere in the spec.
    #[serde(default = "default_validate_against")]
    pub validate_against: ValidateAgainst,
}

impl AuthenticationConfiguration {
    pub fn validate(&self) -> Result<(), String> {
        // Validate that if validate_against is OidcProvider, then the OIDC provider is enabled
        if let ValidateAgainst::OidcProvider = self.validate_against
            && !self.oidc_provider.enabled
        {
            return Err(
                "validate_against is set to OidcProvider but the OIDC provider is not enabled"
                    .to_string(),
            );
        }
        // Same shape for the local mode: selecting it without an authenticator
        // would validate every token against an empty list of trusted issuers.
        if let ValidateAgainst::JwtAuthenticators = self.validate_against
            && self.jwt.is_empty()
        {
            return Err(
                "validate_against is set to JwtAuthenticators but no jwt authenticator is configured"
                    .to_string(),
            );
        }
        // Authenticators are validated whenever they are present, not only when
        // they are selected: a rule that is wrong should be rejected when it is
        // written, not the day someone flips `validate_against`.
        //
        // Issuers must also be unique, as upstream requires: the validator picks
        // the *first* authenticator whose `issuer.url` matches the token, so a
        // second entry for the same issuer — a narrower audience, an extra claim
        // rule — would never run, and the operator would believe it did.
        let mut seen_issuers = std::collections::HashSet::new();
        for authenticator in &self.jwt {
            authenticator.validate()?;
            if !seen_issuers.insert(authenticator.issuer.url.as_str()) {
                return Err(format!(
                    "two jwt authenticators are configured for issuer {}; only the first would ever apply",
                    authenticator.issuer.url
                ));
            }
        }
        // Mirrors the OidcProvider CEL rule. `config_from` may supply both, in
        // which case only the runtime resolve can tell whether they are really
        // set — see `OidcProvider::resolve`.
        if self.oidc_provider.enabled
            && self.oidc_provider.config_from.is_none()
            && (self.oidc_provider.issuer_url.is_empty() || self.oidc_provider.client_id.is_empty())
        {
            return Err(
                "an enabled OIDC provider requires a non-empty issuer_url and client_id, or a \
                 config_from reference supplying them"
                    .to_string(),
            );
        }
        // Mirrors the OidcProvider CEL rule: the well-known discovery document
        // has nothing to expose without an enabled provider.
        if self.oidc_provider.expose_oauth_authorization_server && !self.oidc_provider.enabled {
            return Err(
                "expose_oauth_authorization_server requires the OIDC provider to be enabled"
                    .to_string(),
            );
        }
        Ok(())
    }
}
