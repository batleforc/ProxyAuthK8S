use crate::cli_config::cli_cluster_config::CliClusterConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CliServerConfig {
    pub url: String,
    pub namespace: String,
    pub clusters: HashMap<String, CliClusterConfig>,
}

impl CliServerConfig {
    pub fn url_to_name(&self) -> String {
        let url = self.url.replace("https://", "").replace("http://", "");
        url.replace(".", "-").replace(":", "-")
    }

    pub fn url_to_name_from_string(url: String) -> String {
        let url = url.replace("https://", "").replace("http://", "");
        url.replace(".", "-").replace(":", "-")
    }

    pub fn get_clusters_from_name_ns(
        &self,
        name: String,
        ns: Option<String>,
    ) -> Option<&CliClusterConfig> {
        self.clusters.get(&format!(
            "{}/{}",
            ns.unwrap_or_else(|| self.namespace.clone()),
            name
        ))
    }
}
