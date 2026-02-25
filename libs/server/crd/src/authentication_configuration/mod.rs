pub mod claim_mappings;
pub mod claim_validation_rules;
pub mod issuer;
pub mod jwt_authenticator;
pub mod oidc_provider;
pub mod user_validation_rule;
pub mod validate_against;

use crate::{
    authentication_configuration::{
        jwt_authenticator::JWTAuthenticator, oidc_provider::OidcProvider,
        validate_against::ValidateAgainst,
    },
    default::{default_empty_array, default_validate_against},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct AuthenticationConfiguration {
    #[serde(default = "default_empty_array::<JWTAuthenticator>")]
    pub jwt: Vec<JWTAuthenticator>,
    pub oidc_provider: OidcProvider,
    /// Validate against the configured JWT authenticators, OIDC provider or Kubernetes API
    /// Default : OidcProvider if enabled, otherwise JwtAuthenticators if configured, otherwise Kubernetes
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
        // TODO : validate that if validate_against is JwtAuthenticators, then at least one JWT authenticator is configured and enabled
        Ok(())
    }
}
