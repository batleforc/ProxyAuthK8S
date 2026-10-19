use crate::default::default_empty_string;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A rule the *mapped* user must satisfy, mirroring the apiserver's
/// `userValidationRules`.
///
/// Runs after [`super::ClaimMappings`], over a `user` variable — which is what
/// makes it the right place to refuse a username an external IdP should never be
/// able to mint, such as anything under `system:`.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "self.expression != '' && self.message != ''",
        "message": "a user validation rule requires both an expression and a message",
    }),
]))]
pub struct UserValidationRule {
    /// A CEL expression over `user` that must evaluate to `true`.
    #[serde(default = "default_empty_string")]
    #[schemars(length(max = 4096))]
    pub expression: String,
    /// Shown when the rule rejects a user.
    #[serde(default = "default_empty_string")]
    #[schemars(length(max = 1024))]
    pub message: String,
}

impl UserValidationRule {
    pub fn validate(&self) -> Result<(), String> {
        if self.expression.is_empty() {
            return Err("a user validation rule requires an expression".to_string());
        }
        if self.message.is_empty() {
            return Err("a user validation rule requires a message".to_string());
        }
        Ok(())
    }
}
