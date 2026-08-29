//! Calls to the `ProxyAuthK8S` server through the generated `client_api`.

use client_api::{
    apis::{api_clusters_api::get_all_visible_cluster, configuration::Configuration},
    models::GetAllVisibleClusterBody,
};
use tracing::debug;

use crate::{cli_config::cli_server_config::CliServerConfig, error::ProxyAuthK8sError};

impl CliServerConfig {
    pub fn base_configuration(&self) -> Result<Configuration, ProxyAuthK8sError> {
        let token = self.server_token()?;
        Ok(Configuration {
            base_path: self.url.clone(),
            bearer_access_token: Some(token),
            ..Default::default()
        })
    }

    pub async fn clusters_from_remote(
        &self,
    ) -> Result<GetAllVisibleClusterBody, ProxyAuthK8sError> {
        get_all_visible_cluster(&self.base_configuration()?)
            .await
            .map_err(|e| {
                debug!("Error fetching clusters from remote: {:?}", e);
                e.into()
            })
    }
}
