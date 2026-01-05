use openidconnect::{
    core::{CoreClient, CoreProviderMetadata},
    ClientId, ClientSecret, EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl, RedirectUrl,
};
use serde::{Deserialize, Serialize};

use crate::oidc_error::OidcError;

pub type CoreClientFront = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OidcConf {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub issuer_url: String,
    pub scopes: String,
    pub audience: String,
    pub redirect_url: Option<String>,
}

impl Default for OidcConf {
    fn default() -> Self {
        Self::new()
    }
}

impl OidcConf {
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
            redirect_url,
        }
    }

    pub fn get_reqwest_client(&self) -> reqwest::Client {
        reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("Client should build")
    }

    pub async fn get_oidc_core(&self) -> Result<CoreClientFront, OidcError> {
        let provider_metadata = CoreProviderMetadata::discover_async(
            IssuerUrl::new(self.issuer_url.clone())?,
            &self.get_reqwest_client(),
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
}
