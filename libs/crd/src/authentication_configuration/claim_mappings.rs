use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct ClaimMappings {
    pub username: Option<PrefixedClaimOrExpression>,
    pub groups: Option<PrefixedClaimOrExpression>,
    pub uid: Option<ClaimOrExpression>,
    pub extra: Vec<ExtraMapping>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct PrefixedClaimOrExpression {
    pub prefix: Option<String>,
    pub claim: String,
    pub expression: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct ClaimOrExpression {
    pub claim: Option<String>,
    pub expression: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct ExtraMapping {
    pub key: String,
    pub value_expression: String,
}
