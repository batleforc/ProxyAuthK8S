use authentication_configuration::AuthenticationConfiguration;
use kube::CustomResource;
use schemars::JsonSchema;
use security::SecurityConfiguration;
use serde::{Deserialize, Serialize};
use status::ProxyKubeApiStatus;

pub mod authentication_configuration;
pub mod default;
pub mod security;
pub mod status;

pub static FINALIZER: &str = "weebo.si.rs";

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
    pub cert: String,
    /// Main configuration for authentication
    pub auth_config: AuthenticationConfiguration,
    /// Security configuration
    pub security_config: SecurityConfiguration,
}
