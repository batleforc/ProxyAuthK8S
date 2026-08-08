use crate::default::{default_disabled, default_empty_string};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An enabled provider that cannot be contacted is a cluster that never
/// authenticates anyone, so the apiserver refuses the CR outright.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "!self.enabled || (self.issuer_url != '' && self.client_id != '')",
        "message": "an enabled OIDC provider requires a non-empty issuer_url and client_id",
    }),
    serde_json::json!({
        "rule": "!self.expose_oauth_authorization_server || self.enabled",
        "message": "expose_oauth_authorization_server requires the OIDC provider to be enabled",
    }),
]))]
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
    /// Expose a mediated OAuth 2.0 Authorization Server (RFC 6749 + PKCE) for
    /// this cluster, advertised at `/.well-known/oauth-authorization-server`
    /// (RFC 8414), like OpenShift does for its own OAuth server.
    ///
    /// The proxy itself becomes the authorization server an external caller
    /// talks to — `issuer`, `authorization_endpoint`, `token_endpoint`, and
    /// `jwks_uri` are all this cluster's own proxy URLs — mediating the whole
    /// exchange with the configured `issuer_url` server-side. A caller (e.g.
    /// `oc login`, or any RFC-8414-aware tool) never needs to be registered
    /// with — or even learn the hostname of — the upstream provider, and no
    /// companion `oauth2-proxy` is needed to serve this document.
    ///
    /// Off by default and unauthenticated by nature (discovery, and the
    /// authorization/token endpoints it advertises, must be reachable before
    /// a caller has a token): enabling it lets anonymous callers learn that
    /// this cluster exists and start a login against it. `redirect_uri` at
    /// `/oauth/authorize` is restricted to loopback addresses
    /// (`http://localhost`/`http://127.0.0.1`, RFC 8252) since callers are
    /// not pre-registered — accepting an arbitrary `redirect_uri` would be an
    /// open redirect. Requires `enabled: true`.
    #[serde(default = "default_disabled")]
    pub expose_oauth_authorization_server: bool,
}
