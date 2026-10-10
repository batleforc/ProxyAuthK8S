use super::claim_mappings::ClaimMappings;
use super::claim_validation_rules::ClaimValidationRule;
use super::issuer::Issuer;
use super::user_validation_rule::UserValidationRule;
use crate::default::default_empty_array;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One trusted JWT issuer and the rules applied to the tokens it mints,
/// mirroring one entry of the apiserver's structured authentication
/// configuration `jwt:` list.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct JWTAuthenticator {
    pub issuer: Issuer,
    /// Rules the raw claims must satisfy before any mapping happens.
    #[serde(default = "default_empty_array::<ClaimValidationRule>")]
    pub claim_validation_rules: Vec<ClaimValidationRule>,
    /// How the claims become a username, groups, uid and extra.
    pub claim_mappings: ClaimMappings,
    /// Rules the *mapped* user must satisfy — e.g. refusing a `system:` prefix.
    #[serde(default = "default_empty_array::<UserValidationRule>")]
    pub user_validation_rules: Vec<UserValidationRule>,
}

impl JWTAuthenticator {
    pub fn validate(&self) -> Result<(), String> {
        self.issuer.validate()?;
        for rule in &self.claim_validation_rules {
            rule.validate()?;
        }
        self.claim_mappings.validate()?;
        for rule in &self.user_validation_rules {
            rule.validate()?;
        }
        Ok(())
    }
}
