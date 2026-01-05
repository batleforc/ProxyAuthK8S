use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct Issuer {
    pub url: String,
    #[serde(alias = "discoveryURL")]
    pub discovery_url: Option<String>,
    pub certificate_authority: Option<String>,
    pub audiences: Vec<String>,
    pub audience_match_policy: AudienceMatchPolicyType,
    pub egress_selector: EgressSelectorType,
}

#[derive(Serialize, Deserialize, Clone, Debug, Copy, JsonSchema)]
pub enum AudienceMatchPolicyType {
    #[serde(rename = "MatchAny")]
    MatchAny,
}

#[derive(Serialize, Deserialize, Clone, Debug, Copy, JsonSchema)]
pub enum EgressSelectorType {
    #[serde(rename = "controlplane")]
    ControlPlane,
    #[serde(rename = "cluster")]
    Cluster,
}
