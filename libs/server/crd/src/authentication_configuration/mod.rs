pub mod claim_mappings;
pub mod claim_validation_rules;
pub mod issuer;
pub mod user_validation_rule;
use crate::default::{
    default_disabled, default_empty_array, default_empty_string, default_validate_against,
};

use claim_mappings::ClaimMappings;
use claim_validation_rules::ClaimValidationRule;
use issuer::Issuer;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use user_validation_rule::UserValidationRule;

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

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct JWTAuthenticator {
    pub issuer: Issuer,
    pub claim_validation_rules: Vec<ClaimValidationRule>,
    pub claim_mappings: ClaimMappings,
    pub user_validation_rules: Vec<UserValidationRule>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct OidcProvider {
    #[serde(default = "default_disabled")]
    pub enabled: bool,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    #[serde(default = "default_empty_string")]
    pub extra_scope: String,
}

/// Validate the authentication token against either:
/// - the JWT authenticators, by validating the token signature and claims according to the configured rules
/// - the OIDC provider, by validating the token by calling the provider's userinfo endpoint and validating the response according to the configured rules
/// - the kubernetes API, by validating the token by calling the SelfSubjectAccessReview API
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub enum ValidateAgainst {
    JwtAuthenticators,
    OidcProvider,
    Kubernetes,
}
