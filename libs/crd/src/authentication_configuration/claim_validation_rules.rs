use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct ClaimValidationRule {
    pub claim: String,
    pub required_value: String,

    pub expression: String,
    pub message: String,
}
