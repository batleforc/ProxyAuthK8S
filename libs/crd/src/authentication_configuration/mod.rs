pub mod claim_mappings;
pub mod claim_validation_rules;
pub mod issuer;
pub mod user_validation_rule;

use claim_mappings::ClaimMappings;
use claim_validation_rules::ClaimValidationRule;
use issuer::Issuer;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use user_validation_rule::UserValidationRule;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct AuthenticationConfiguration {
    pub jwt: Vec<JWTAuthenticator>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct JWTAuthenticator {
    pub issuer: Issuer,
    pub claim_validation_rules: Vec<ClaimValidationRule>,
    pub claim_mappings: ClaimMappings,
    pub user_validation_rules: Vec<UserValidationRule>,
}
