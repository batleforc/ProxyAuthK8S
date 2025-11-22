use openidconnect::{
    core::{CoreClient, CoreProviderMetadata},
    ClientId, EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl,
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
    pub issuer_url: String,
    pub scopes: String,
    pub audience: String,
}

impl OidcConf {
    pub fn new() -> Self {
        let client_id = std::env::var("OIDC_CLIENT_ID").unwrap_or("proxy-auth-k8s".to_string());
        let issuer_url = std::env::var("OIDC_ISSUER_URL")
            .unwrap_or("https://authelia.k8s.localhost".to_string());
        let scopes = std::env::var("OIDC_SCOPES").unwrap_or("openid email profile".to_string());
        let audience = std::env::var("OIDC_AUDIENCE").unwrap_or("proxy-auth-k8s".to_string());
        Self {
            client_id,
            issuer_url,
            scopes,
            audience,
        }
    }

    pub fn get_reqwest_client(&self) -> reqwest::Client {
        reqwest::ClientBuilder::new()
            // Following redirects opens the client up to SSRF vulnerabilities.
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
        Ok(CoreClient::from_provider_metadata(
            provider_metadata,
            ClientId::new(self.client_id.clone()),
            None,
        ))
    }
}
