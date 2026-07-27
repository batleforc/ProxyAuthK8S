mod claim_mappings;
mod claim_validation_rules;
mod issuer;
mod jwt_authenticator;
mod oidc_provider;
mod user_validation_rule;
mod validate_against;

pub use claim_mappings::{
    ClaimMappings, ClaimOrExpression, ExtraMapping, PrefixedClaimOrExpression,
};
pub use claim_validation_rules::ClaimValidationRule;
pub use issuer::{AudienceMatchPolicyType, EgressSelectorType, Issuer};
pub use jwt_authenticator::JWTAuthenticator;
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
#[schemars(extend("x-kubernetes-validations" = [serde_json::json!({
    "rule": "self.validate_against != 'OidcProvider' || self.oidc_provider.enabled",
    "message": "validate_against is set to OidcProvider but the OIDC provider is not enabled",
})]))]
pub struct AuthenticationConfiguration {
    #[serde(default = "default_empty_array::<JWTAuthenticator>")]
    pub jwt: Vec<JWTAuthenticator>,
    pub oidc_provider: OidcProvider,

    /// Disable validation of the token against the configured JWT authenticators, OIDC provider or Kubernetes API
    /// If the `AuthenticationConfiguration` is not provided, does not validate the token against any of the configured JWT authenticators, OIDC provider or Kubernetes API
    /// Default : false
    #[serde(default = "default_disabled")]
    pub disable_validation: bool,
    /// Validate against the configured JWT authenticators, OIDC provider or Kubernetes API
    /// Default : `OidcProvider` if enabled, otherwise `JwtAuthenticators` if configured, otherwise Kubernetes
    #[serde(default = "default_validate_against")]
    pub validate_against: ValidateAgainst,
}

impl AuthenticationConfiguration {
    pub fn validate(&self) -> Result<(), String> {
        // Validate that if validate_against is OidcProvider, then the OIDC provider is enabled
        if let ValidateAgainst::OidcProvider = self.validate_against {
            if !self.oidc_provider.enabled {
                return Err(
                    "validate_against is set to OidcProvider but the OIDC provider is not enabled"
                        .to_string(),
                );
            }
        }
        Ok(())
    }
}
