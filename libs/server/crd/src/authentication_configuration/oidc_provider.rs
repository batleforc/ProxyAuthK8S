use crate::default::{default_disabled, default_empty_string};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An enabled provider that cannot be contacted is a cluster that never
/// authenticates anyone, so the apiserver refuses the CR outright.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [serde_json::json!({
    "rule": "!self.enabled || (self.issuer_url != '' && self.client_id != '')",
    "message": "an enabled OIDC provider requires a non-empty issuer_url and client_id",
})]))]
pub struct OidcProvider {
    #[serde(default = "default_disabled")]
    pub enabled: bool,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    #[serde(default = "default_empty_string")]
    pub extra_scope: String,
}
