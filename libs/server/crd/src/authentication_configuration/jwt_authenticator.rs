use super::claim_mappings::ClaimMappings;
use super::claim_validation_rules::ClaimValidationRule;
use super::issuer::Issuer;
use super::user_validation_rule::UserValidationRule;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct JWTAuthenticator {
    pub issuer: Issuer,
    pub claim_validation_rules: Vec<ClaimValidationRule>,
    pub claim_mappings: ClaimMappings,
    pub user_validation_rules: Vec<UserValidationRule>,
}
