use crate::default::{default_empty_array, default_empty_string};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How the token's claims become a Kubernetes user, mirroring the apiserver's
/// `claimMappings`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "has(self.username)",
        "message": "claim_mappings.username is required: without it a validated token maps to no user",
    }),
]))]
pub struct ClaimMappings {
    /// Required — a token that validates but maps to no username authenticates
    /// nobody, so the apiserver requires it too.
    pub username: Option<PrefixedClaimOrExpression>,
    pub groups: Option<PrefixedClaimOrExpression>,
    pub uid: Option<ClaimOrExpression>,
    #[serde(default = "default_empty_array::<ExtraMapping>")]
    pub extra: Vec<ExtraMapping>,
}

/// Either a claim (with an optional prefix) or a CEL expression, never both.
///
/// `prefix` exists to keep an external identity provider from minting a
/// username that collides with a local one; it applies only to the `claim`
/// form, since an `expression` can prepend whatever it likes itself.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "has(self.claim) != has(self.expression)",
        "message": "exactly one of claim or expression must be set",
    }),
    // Both directions, as upstream requires: a prefix is meaningless beside an
    // expression (which builds whatever name it likes), and a claim without an
    // explicit prefix — even an empty one — is a collision decision nobody
    // made. An unprefixed external username can collide with a local one.
    serde_json::json!({
        "rule": "!has(self.prefix) || has(self.claim)",
        "message": "prefix may only be set together with claim",
    }),
    serde_json::json!({
        "rule": "!has(self.claim) || has(self.prefix)",
        "message": "prefix is required with claim (use \"\" to opt out of prefixing deliberately)",
    }),
]))]
pub struct PrefixedClaimOrExpression {
    /// Prepended to the claim's value. Only valid with `claim`.
    #[schemars(length(max = 253))]
    pub prefix: Option<String>,
    #[schemars(length(max = 253))]
    pub claim: Option<String>,
    #[schemars(length(max = 4096))]
    pub expression: Option<String>,
}

/// Either a claim or a CEL expression, never both.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "has(self.claim) != has(self.expression)",
        "message": "exactly one of claim or expression must be set",
    }),
]))]
pub struct ClaimOrExpression {
    #[schemars(length(max = 253))]
    pub claim: Option<String>,
    #[schemars(length(max = 4096))]
    pub expression: Option<String>,
}

/// One entry of the user's `extra` map, keyed by a domain-prefixed name.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct ExtraMapping {
    pub key: String,
    /// A CEL expression over `claims` producing the value(s) for `key`.
    #[serde(default = "default_empty_string")]
    pub value_expression: String,
}

impl PrefixedClaimOrExpression {
    pub fn validate(&self, field: &str) -> Result<(), String> {
        match (&self.claim, &self.expression) {
            (Some(_), Some(_)) => Err(format!(
                "claim_mappings.{field} cannot set both claim and expression"
            )),
            (None, None) => Err(format!(
                "claim_mappings.{field} must set either claim or expression"
            )),
            (None, Some(_)) if self.prefix.is_some() => Err(format!(
                "claim_mappings.{field}.prefix may only be set together with claim"
            )),
            (Some(_), None) if self.prefix.is_none() => Err(format!(
                "claim_mappings.{field}.prefix is required with claim \
                 (use \"\" to opt out of prefixing deliberately)"
            )),
            _ => Ok(()),
        }
    }
}

impl ClaimOrExpression {
    pub fn validate(&self, field: &str) -> Result<(), String> {
        match (&self.claim, &self.expression) {
            (Some(_), Some(_)) => Err(format!(
                "claim_mappings.{field} cannot set both claim and expression"
            )),
            (None, None) => Err(format!(
                "claim_mappings.{field} must set either claim or expression"
            )),
            _ => Ok(()),
        }
    }
}

impl ClaimMappings {
    pub fn validate(&self) -> Result<(), String> {
        let Some(username) = &self.username else {
            return Err(
                "claim_mappings.username is required: without it a validated token maps to no user"
                    .to_string(),
            );
        };
        username.validate("username")?;
        if let Some(groups) = &self.groups {
            groups.validate("groups")?;
        }
        if let Some(uid) = &self.uid {
            uid.validate("uid")?;
        }
        for extra in &self.extra {
            if extra.key.is_empty() {
                return Err("claim_mappings.extra entries require a key".to_string());
            }
            if extra.value_expression.is_empty() {
                return Err(format!(
                    "claim_mappings.extra[{}] requires a value_expression",
                    extra.key
                ));
            }
        }
        Ok(())
    }
}
