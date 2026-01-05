use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct UserValidationRule {
    pub expression: String,
    pub message: String,
}
