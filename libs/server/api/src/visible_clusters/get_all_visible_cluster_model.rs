use crd::ProxyKubeApi;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Model representing a cluster visible to the user.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct VisibleCluster {
    pub enabled: bool,
    pub namespace: String,
    pub name: String,
    pub sso_enabled: bool,
    pub is_reachable: Option<bool>,
}

impl VisibleCluster {
    /// Create a new `VisibleCluster` instance
    #[must_use]
    pub fn new(
        enabled: bool,
        namespace: String,
        name: String,
        sso_enabled: bool,
        is_reachable: Option<bool>,
    ) -> Self {
        Self {
            enabled,
            namespace,
            name,
            sso_enabled,
            is_reachable,
        }
    }
}

impl From<ProxyKubeApi> for VisibleCluster {
    fn from(proxy: ProxyKubeApi) -> Self {
        Self {
            enabled: proxy.spec.enabled,
            namespace: proxy.metadata.namespace.unwrap_or_default(),
            name: proxy.metadata.name.unwrap_or_default(),
            sso_enabled: proxy
                .spec
                .auth_config
                .is_some_and(|auth_config| auth_config.oidc_provider.enabled),
            is_reachable: Some(proxy.status.unwrap_or_default().exposed),
        }
    }
}

/// Body of the response for the `get_all_visible_cluster` endpoint.
///
/// Contains a list of clusters visible to the user.
/// Will be empty if the user has no clusters visible to them.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct GetAllVisibleClusterBody {
    pub clusters: Vec<VisibleCluster>,
}
