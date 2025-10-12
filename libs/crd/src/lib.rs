use std::sync::Arc;

use authentication_configuration::AuthenticationConfiguration;
use certificate::CertSource;
use common::State;
use default::default_enabled;
use kube::{CustomResource, ResourceExt};
use schemars::JsonSchema;
use security::SecurityConfiguration;
use serde::{Deserialize, Serialize};
use service::Service;
use status::ProxyKubeApiStatus;

pub mod authentication_configuration;
pub mod certificate;
pub mod default;
pub mod security;
pub mod service;
pub mod status;

pub static PROXY_KUBE_FINALIZER: &str = "weebo.si.rs";

#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "weebo.si.rs",
    version = "v1",
    kind = "ProxyKubeApi",
    plural = "proxykubeapis",
    namespaced
)]
#[kube(status = "ProxyKubeApiStatus")]
pub struct ProxyKubeApiSpec {
    /// Certificate for the Kubernetes API
    pub cert: CertSource,
    /// Service to expose the proxy
    pub service: Service,
    /// Main configuration for authentication
    pub auth_config: Option<AuthenticationConfiguration>,
    /// Security configuration
    pub security_config: Option<SecurityConfiguration>,
    /// If the proxy exposition should be accessible via the Dashboard
    /// Default: false
    #[serde(default = "default_enabled")]
    pub expose_via_dashboard: bool,
    /// If the proxy exposition is accessible via the dashboard
    /// the oidc group that allow access to the dashboard, should be unique
    /// Default: to the resource namespace + resource name
    pub dashboard_group: Option<String>,
}

impl ProxyKubeApi {
    pub fn to_identifier(&self) -> String {
        format!(
            "proxyk8sauth:{}/{}",
            self.namespace().unwrap_or_default(),
            self.name_any()
        )
    }
    pub fn to_path(&self) -> String {
        format!(
            "{}/{}",
            self.namespace().unwrap_or_default(),
            self.name_any()
        )
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    pub fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }
    /// Check if the service is reachable
    pub async fn is_reachable(&self, ctx: Arc<State>) -> Result<bool, String> {
        let ip = match self
            .spec
            .service
            .url_to_call(ctx.client.clone(), self.namespace().unwrap_or_default())
            .await
        {
            Ok(url) => url,
            Err(_) => return Ok(false),
        };
        let mut reqwest_client = reqwest::ClientBuilder::new();
        reqwest_client = match &self
            .spec
            .cert
            .get_cert(ctx.client.clone(), &self.namespace().unwrap_or_default())
            .await
        {
            Ok(Some(cert)) => match reqwest::Certificate::from_pem(cert.as_bytes()) {
                Ok(c) => reqwest_client.add_root_certificate(c),
                Err(err) => return Err(err.to_string()),
            },
            Ok(None) => reqwest_client.danger_accept_invalid_certs(true),
            Err(err) => {
                return Err(err.to_string());
            }
        };
        reqwest_client = reqwest_client.use_rustls_tls();
        let client = match reqwest_client.build() {
            Ok(c) => c,
            Err(err) => return Err(err.to_string()),
        };
        match client.get(&ip).send().await {
            Ok(resp) => {
                if resp.status().as_u16() >= 400 && resp.status().as_u16() < 500 {
                    // client error, the service is reachable but the request is not authorized
                    return Ok(true);
                }
                if resp.status().is_success() {
                    return Ok(true);
                }
                return Ok(false);
            }
            Err(err) => {
                match err.status() {
                    Some(status) => {
                        if status.as_u16() >= 400 && status.as_u16() < 500 {
                            // client error, the service is reachable but the request is not authorized
                            return Ok(true);
                        }
                        if status.is_success() {
                            return Ok(true);
                        }
                        return Ok(false);
                    }
                    None => {}
                }
                return Err(err.to_string());
            }
        }
    }
}
