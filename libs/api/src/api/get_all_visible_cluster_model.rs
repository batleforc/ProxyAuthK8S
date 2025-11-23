use crd::ProxyKubeApi;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Model representing a cluster visible to the user
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct VisibleCluster {
    pub namespace: String,
    pub name: String,
    pub sso_enabled: bool,
}

impl VisibleCluster {
    /// Create a new VisibleCluster instance
    pub fn new(namespace: String, name: String, sso_enabled: bool) -> Self {
        Self {
            namespace,
            name,
            sso_enabled,
        }
    }
}

impl From<ProxyKubeApi> for VisibleCluster {
    fn from(proxy: ProxyKubeApi) -> Self {
        Self {
            namespace: proxy.metadata.namespace.unwrap_or_default(),
            name: proxy.metadata.name.unwrap_or_default(),
            sso_enabled: proxy.spec.auth_config.is_some()
                && proxy.spec.auth_config.unwrap().oidc_provider.enabled,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct GetAllVisibleClusterBody {
    pub clusters: Vec<VisibleCluster>,
}
