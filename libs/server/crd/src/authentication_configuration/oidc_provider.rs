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
    /// Audience the access token must carry (its `aud` claim) to be accepted.
    ///
    /// When empty, `client_id` is used — correct for providers that put the
    /// client id in `aud`. Set it explicitly when the provider mints tokens for a
    /// distinct resource audience, so `client_id` (who the token is for) and the
    /// expected audience (what the token is for) are configured independently.
    #[serde(default = "default_empty_string")]
    pub audience: String,
    /// Accept a token whose `aud` does not name this service but whose
    /// `azp`/`client_id` does.
    ///
    /// Off by default: `azp`/`client_id` identify the client the token was issued
    /// to, not the resource, so accepting them weakens audience validation.
    /// Enable only for providers (e.g. Keycloak) that mint self-audience tokens
    /// carrying the client only in `azp`.
    #[serde(default = "default_disabled")]
    pub accept_authorized_party: bool,
}
