use crate::default::default_empty_string;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A rule the token's claims must satisfy, mirroring the apiserver's
/// `claimValidationRules`.
///
/// Two mutually exclusive shapes, exactly as upstream:
///   - `claim` + `required_value`: the named claim must be a string equal to
///     `required_value`;
///   - `expression` (+ `message`): a CEL expression over `claims` that must
///     evaluate to `true`.
///
/// Both halves are optional in the schema because only one may be given; which
/// one was given is checked by [`ClaimValidationRule::validate`] and by the CEL
/// admission rule that mirrors it. Before this was modelled as an either/or, all
/// four fields were required — a rule could not be written at all without
/// supplying nonsense for the half you did not mean.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "has(self.claim) != has(self.expression)",
        "message": "exactly one of claim or expression must be set",
    }),
    // Guards on the *value*, not on `has()`: `required_value` carries a schema
    // default, and the apiserver applies structural defaults before evaluating
    // these rules — so `has(self.required_value)` is unconditionally true and a
    // `has()`-based rule here would reject every expression-form rule ever
    // written. Same reasoning applies to any other defaulted field.
    serde_json::json!({
        "rule": "self.required_value == '' || has(self.claim)",
        "message": "required_value may only be set together with claim",
    }),
    serde_json::json!({
        "rule": "!has(self.expression) || self.message != ''",
        "message": "an expression rule requires a message explaining what it rejects",
    }),
]))]
pub struct ClaimValidationRule {
    /// The claim whose value must equal `required_value`.
    #[schemars(length(max = 253))]
    pub claim: Option<String>,
    /// The value `claim` must hold. Absent means "the claim must merely be
    /// present", which is how the apiserver reads an empty `requiredValue`.
    #[serde(default = "default_empty_string")]
    #[schemars(length(max = 1024))]
    pub required_value: String,

    /// A CEL expression over `claims` that must evaluate to `true`.
    #[schemars(length(max = 4096))]
    pub expression: Option<String>,
    /// Shown when `expression` rejects a token. Required with `expression`: a
    /// rejection nobody can explain is a support ticket, not a security control.
    #[serde(default = "default_empty_string")]
    #[schemars(length(max = 1024))]
    pub message: String,
}

impl ClaimValidationRule {
    /// Mirrors the CEL admission rules above, for the paths that do not go
    /// through the apiserver (the controller's own `validate`, and unit tests).
    pub fn validate(&self) -> Result<(), String> {
        match (&self.claim, &self.expression) {
            (Some(_), Some(_)) => {
                Err("a claim validation rule cannot set both claim and expression".to_string())
            }
            (None, None) => {
                Err("a claim validation rule must set either claim or expression".to_string())
            }
            (None, Some(_)) if self.message.is_empty() => {
                Err("a claim validation rule using expression requires a message".to_string())
            }
            (None, Some(_)) if !self.required_value.is_empty() => {
                Err("required_value may only be set together with claim".to_string())
            }
            _ => Ok(()),
        }
    }
}
