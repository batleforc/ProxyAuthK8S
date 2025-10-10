use authentication_configuration::AuthenticationConfiguration;
use default::default_enabled;
use kube::{CustomResource, ResourceExt};
use schemars::JsonSchema;
use security::SecurityConfiguration;
use serde::{Deserialize, Serialize};
use status::ProxyKubeApiStatus;
pub mod authentication_configuration;
pub mod default;
pub mod security;
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
    /// Main configuration for authentication
    pub auth_config: AuthenticationConfiguration,
    /// Security configuration
    pub security_config: SecurityConfiguration,
    /// If the proxy exposition should be accessible via the Dashboard
    /// Default: false
    #[serde(default = "default_enabled")]
    pub expose_via_dashboard: bool,
    /// If the proxy exposition is accessible via the dashboard
    /// the oidc group that allow access to the dashboard, should be unique
    /// Default: to the resource namespace + resource name
    pub dashboard_group: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub enum CertSource {
    /// Use a cert from a secret
    Secret { name: String, key: String },
    /// Use a cert from a file path
    Cert(String),
}

impl ProxyKubeApi {
    pub fn to_identifier(&self) -> String {
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
}
