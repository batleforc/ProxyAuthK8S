use crate::default::{default_disabled, default_empty_string};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct OidcProvider {
    #[serde(default = "default_disabled")]
    pub enabled: bool,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    #[serde(default = "default_empty_string")]
    pub extra_scope: String,
}
