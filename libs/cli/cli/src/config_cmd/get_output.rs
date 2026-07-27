use serde::{Deserialize, Serialize};
use std::vec;

use crate::{cli_config::cli_server_config::CliServerConfig, output::TableRow};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetOutput {
    pub is_default: bool,
    pub url: String,
    pub default_namespace: String,
    pub has_clusters: bool,
}

impl GetOutput {
    #[must_use]
    pub fn new_from_servers(
        cli_server_config: CliServerConfig,
        default_server_name: String,
    ) -> GetOutput {
        GetOutput {
            is_default: cli_server_config.url_to_name() == default_server_name,
            url: cli_server_config.url,
            default_namespace: cli_server_config.namespace,
            has_clusters: !cli_server_config.clusters.is_empty(),
        }
    }
}

impl TableRow for GetOutput {
    fn headers() -> Vec<String> {
        vec![
            "Is Default".to_string(),
            "Server URL".to_string(),
            "Default Namespace".to_string(),
            "Has Clusters".to_string(),
        ]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.is_default.to_string(),
            self.url.clone(),
            self.default_namespace.clone(),
            self.has_clusters.to_string(),
        ]
    }
}
