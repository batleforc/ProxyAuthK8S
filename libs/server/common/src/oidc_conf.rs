use std::fmt::Debug;

use oauth2_reqwest::ReqwestClient;
use openidconnect::{
    core::{CoreClient, CoreProviderMetadata},
    ClientId, ClientSecret, EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl, RedirectUrl,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::oidc_error::OidcError;

pub type CoreClientFront = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

#[derive(Clone, Deserialize, Serialize)]
pub struct OidcConf {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub issuer_url: String,
    pub scopes: String,
    /// Audience the token must carry (its `aud` claim) to be accepted. Distinct
    /// from `client_id`: who the token was issued to versus what it is for.
    pub audience: String,
    /// Whether a token whose `aud` does not name this service is still accepted
    /// when its `azp`/`client_id` does. Off by default; see [`crate::token_audience`].
    #[serde(default)]
    pub accept_authorized_party: bool,
    pub redirect_url: Option<String>,
}

impl Debug for OidcConf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcConf")
            .field("client_id", &self.client_id)
            .field("client_secret", &"***REDACTED***")
            .field("issuer_url", &self.issuer_url)
            .field("scopes", &self.scopes)
            .field("audience", &self.audience)
            .field("accept_authorized_party", &self.accept_authorized_party)
            .field("redirect_url", &self.redirect_url)
            .finish()
    }
}

impl Default for OidcConf {
    fn default() -> Self {
        Self::new()
    }
}

impl OidcConf {
    #[must_use]
    pub fn new() -> Self {
        let client_id = std::env::var("OIDC_CLIENT_ID").unwrap_or("proxy-auth-k8s".to_string());
        let client_secret = std::env::var("OIDC_CLIENT_SECRET").ok();
        let issuer_url = std::env::var("OIDC_ISSUER_URL")
            .unwrap_or("https://authelia.k8s.localhost".to_string());
        let scopes = std::env::var("OIDC_SCOPES").unwrap_or("openid email profile".to_string());
        let audience = std::env::var("OIDC_AUDIENCE").unwrap_or("proxy-auth-k8s".to_string());
        let redirect_url = std::env::var("OIDC_REDIRECT_URL").ok();
        Self {
            client_id,
            client_secret,
            issuer_url,
            scopes,
            audience,
            // Secure default; per-cluster providers set this via the CRD field.
            accept_authorized_party: false,
            redirect_url,
        }
    }

    /// Build a plain `reqwest` client that never follows redirects.
    ///
    /// Redirects are disabled deliberately: an OIDC/OAuth exchange must talk to
    /// the exact endpoint it targeted, never a location the provider hands back.
    ///
    /// # Errors
    ///
    /// Returns [`OidcError::HttpClient`] if the HTTP client cannot be built.
    pub fn reqwest_client(&self) -> Result<reqwest::Client, OidcError> {
        Ok(reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .build()?)
    }

    /// Build the `openidconnect`-flavoured HTTP client (see [`Self::reqwest_client`]).
    ///
    /// # Errors
    ///
    /// Returns [`OidcError::HttpClient`] if the HTTP client cannot be built.
    pub fn oidc_reqwest_client(&self) -> Result<ReqwestClient, OidcError> {
        Ok(ReqwestClient::from(self.reqwest_client()?))
    }

    /// Discover the provider and build the OIDC client used for the front-end flow.
    ///
    /// # Errors
    ///
    /// Returns [`OidcError`] if the issuer URL is invalid, provider discovery
    /// fails, the redirect URL is invalid, or the HTTP client cannot be built.
    #[instrument(skip(self))]
    pub async fn oidc_core(&self) -> Result<CoreClientFront, OidcError> {
        let provider_metadata = CoreProviderMetadata::discover_async(
            IssuerUrl::new(self.issuer_url.clone())?,
            &self.oidc_reqwest_client()?,
        )
        .await?;
        let client_secret = self
            .client_secret
            .as_ref()
            .map(|secret| ClientSecret::new(secret.clone()));
        let mut core_client = CoreClient::from_provider_metadata(
            provider_metadata,
            ClientId::new(self.client_id.clone()),
            client_secret,
        );
        if let Some(redirect_url) = &self.redirect_url {
            core_client = core_client.set_redirect_uri(RedirectUrl::new(redirect_url.clone())?);
        }
        Ok(core_client)
    }

    /// Enforce that `token` was actually minted for this service's audience.
    ///
    /// MUST be called only after the token has otherwise been validated (e.g. a
    /// successful `/userinfo` call): the JWT fallback trusts the token's claims,
    /// which is only safe once the signature is known good.
    ///
    /// Order (per the operator's chosen policy): try RFC 7662 introspection when
    /// the provider advertises an endpoint, then fall back to reading the JWT
    /// `aud`/`azp`/`client_id` claims. When the audience cannot be determined at
    /// all, the configured [`AudienceValidationMode`] decides (enforce → reject).
    ///
    /// # Errors
    ///
    /// Returns [`OidcError::AudienceValidation`] when the audience does not match
    /// and the mode is `Enforce` (or when it cannot be determined and the mode
    /// fails closed).
    #[instrument(skip(self, token))]
    pub async fn ensure_token_audience(&self, token: &str) -> Result<(), OidcError> {
        use crate::token_audience::{extract_jwt_audiences, AudienceValidationMode};

        let mode = AudienceValidationMode::from_env();
        if mode == AudienceValidationMode::Off {
            return Ok(());
        }

        // 1. Introspection first, when the provider exposes an endpoint.
        if let Some(endpoint) = self.discover_introspection_endpoint().await {
            match self.introspect_audiences(&endpoint, token).await {
                Ok(Some(auds)) => return self.decide_audience(&auds, mode, "introspection"),
                Ok(None) => {
                    return Self::reject_or_warn(mode, "introspection reported the token inactive");
                }
                Err(()) => {
                    tracing::warn!(
                        "token introspection failed; falling back to JWT audience check"
                    );
                }
            }
        }

        // 2. JWT claims fallback (safe post-userinfo, see method doc).
        if let Some(auds) = extract_jwt_audiences(token) {
            return self.decide_audience(&auds, mode, "jwt");
        }

        // 3. Neither mechanism could determine the audience.
        Self::reject_or_warn(
            mode,
            "token audience could not be determined (opaque token and no usable introspection endpoint)",
        )
    }

    fn decide_audience(
        &self,
        auds: &crate::token_audience::TokenAudiences,
        mode: crate::token_audience::AudienceValidationMode,
        source: &str,
    ) -> Result<(), OidcError> {
        if auds.matches(&self.audience, self.accept_authorized_party) {
            tracing::debug!(source, "token audience accepted");
            Ok(())
        } else {
            Self::reject_or_warn(
                mode,
                &format!(
                    "token audiences {:?} (authorized party {:?}) do not include the expected \
                     audience {:?} (via {})",
                    auds.values, auds.authorized_party, self.audience, source
                ),
            )
        }
    }

    fn reject_or_warn(
        mode: crate::token_audience::AudienceValidationMode,
        reason: &str,
    ) -> Result<(), OidcError> {
        use crate::token_audience::AudienceValidationMode;
        match mode {
            AudienceValidationMode::Enforce => {
                tracing::warn!("rejecting token: {}", reason);
                Err(OidcError::AudienceValidation(reason.to_string()))
            }
            AudienceValidationMode::Warn => {
                tracing::warn!("audience validation (warn mode, allowing): {}", reason);
                Ok(())
            }
            AudienceValidationMode::Off => Ok(()),
        }
    }

    /// Fetch the provider's `introspection_endpoint` from its discovery document,
    /// or `None` when the provider does not advertise one.
    async fn discover_introspection_endpoint(&self) -> Option<String> {
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.issuer_url.trim_end_matches('/')
        );
        let response = self.reqwest_client().ok()?.get(url).send().await.ok()?;
        let metadata: serde_json::Value = response.json().await.ok()?;
        metadata
            .get("introspection_endpoint")?
            .as_str()
            .map(std::string::ToString::to_string)
    }

    /// RFC 7662 token introspection.
    ///
    /// Returns `Ok(Some(audiences))` when the token is active, `Ok(None)` when
    /// the provider reports it inactive, and `Err(())` on a transport/parse
    /// error (so the caller can fall back to the JWT check).
    async fn introspect_audiences(
        &self,
        endpoint: &str,
        token: &str,
    ) -> Result<Option<crate::token_audience::TokenAudiences>, ()> {
        let mut request = self.reqwest_client().map_err(|_| ())?.post(endpoint);
        let mut body = openidconnect::url::form_urlencoded::Serializer::new(String::new());
        body.append_pair("token", token);
        body.append_pair("token_type_hint", "access_token");
        // RFC 7662 requires the caller to authenticate: HTTP Basic with the
        // client secret when we have one, otherwise send the client_id in the
        // body for a public client.
        if let Some(secret) = &self.client_secret {
            request = request.basic_auth(&self.client_id, Some(secret));
        } else {
            body.append_pair("client_id", &self.client_id);
        }
        let body = body.finish();

        let response = request
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|_| ())?;
        let body: serde_json::Value = response.json().await.map_err(|_| ())?;

        if !body
            .get("active")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(None);
        }

        let mut values = Vec::new();
        match body.get("aud") {
            Some(serde_json::Value::String(aud)) => values.push(aud.clone()),
            Some(serde_json::Value::Array(auds)) => {
                for aud in auds {
                    if let Some(aud) = aud.as_str() {
                        values.push(aud.to_string());
                    }
                }
            }
            _ => {}
        }
        // `client_id` (and `azp`) name the client the token was issued to, not
        // the resource; keep them out of `aud` and only consult them when azp
        // acceptance is explicitly enabled.
        let mut authorized_party = Vec::new();
        if let Some(azp) = body.get("azp").and_then(|v| v.as_str()) {
            authorized_party.push(azp.to_string());
        }
        if let Some(client_id) = body.get("client_id").and_then(|v| v.as_str()) {
            authorized_party.push(client_id.to_string());
        }
        Ok(Some(crate::token_audience::TokenAudiences {
            values,
            authorized_party,
        }))
    }
}
