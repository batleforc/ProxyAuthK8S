pub mod claim_mappings;
pub mod claim_validation_rules;
pub mod issuer;
pub mod user_validation_rule;
use crate::default::default_disabled;

use claim_mappings::ClaimMappings;
use claim_validation_rules::ClaimValidationRule;
use issuer::Issuer;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use user_validation_rule::UserValidationRule;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct AuthenticationConfiguration {
    pub jwt: Vec<JWTAuthenticator>,
    pub oidc_provider: OidcProvider,
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
    pub extra_scope: String,
}
