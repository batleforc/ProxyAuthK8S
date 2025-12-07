use std::collections::HashMap;

use reqwest::Url;
use serde::{Deserialize, Serialize};

pub mod cli_cluster_config;
pub mod cli_server_config;
pub mod error;

use cli_server_config::CliServerConfig;

use crate::cli_config::{cli_cluster_config::CliClusterConfig, error::CliConfigError};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CliConfig {
    pub default_server_name: String,
    pub servers: HashMap<String, CliServerConfig>,
}

impl Default for CliConfig {
    fn default() -> Self {
        CliConfig::new()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct UrlInfo {
    pub server_name: String,
    pub namespace: String,
    pub cluster_name: String,
}

impl CliConfig {
    pub fn new() -> Self {
        let server = CliServerConfig {
            url: "http://localhost:5437".to_string(),
            namespace: "default".to_string(),
            clusters: HashMap::new(),
        };
        CliConfig {
            default_server_name: "localhost-5437".to_string(),
            servers: vec![(server.url_to_name(), server)].into_iter().collect(),
        }
    }

    pub fn from_yaml(yaml_str: &str) -> Result<Self, CliConfigError> {
        match serde_yaml::from_str::<CliConfig>(yaml_str) {
            Ok(config) => Ok(config),
            Err(err) => Err(CliConfigError::YamlParseError(err.to_string())),
        }
    }

    pub fn to_yaml(&self) -> Result<String, CliConfigError> {
        match serde_yaml::to_string(self) {
            Ok(yaml_str) => Ok(yaml_str),
            Err(err) => Err(CliConfigError::YamlSerializeError(err.to_string())),
        }
    }

    pub fn get_cluster_config(
        &self,
        server_name: Option<String>,
        cluster_name: String,
        ns: Option<String>,
    ) -> Option<&CliClusterConfig> {
        let server_name = server_name.unwrap_or_else(|| self.default_server_name.clone());
        self.servers
            .get(&server_name)
            .and_then(|server| server.get_clusters_from_name_ns(cluster_name, ns))
    }

    pub fn proxy_url_to_tuple(url: &str) -> Result<UrlInfo, CliConfigError> {
        let parsed_url = match Url::parse(url) {
            Ok(u) => u,
            Err(err) => {
                return Err(CliConfigError::InvalidServerUrl(
                    url.to_string(),
                    err.to_string(),
                ))
            }
        };

        let host = match parsed_url.host_str() {
            Some(h) => h,
            None => {
                return Err(CliConfigError::InvalidServerUrl(
                    url.to_string(),
                    "No host found in URL".to_string(),
                ))
            }
        };
        let server_name = host.replace(".", "-").replace(":", "-");
        let ns = parsed_url
            .path_segments()
            .and_then(|mut segments| segments.nth(1))
            .map(|s| s.to_string());
        let cluster_name = parsed_url
            .path_segments()
            .and_then(|mut segments| segments.nth(2))
            .map(|s| s.to_string());
        if ns.is_none() || cluster_name.is_none() {
            return Err(CliConfigError::InvalidServerUrl(
                url.to_string(),
                "Namespace or cluster name not found in URL path".to_string(),
            ));
        }

        Ok(UrlInfo {
            server_name,
            namespace: ns.unwrap(),
            cluster_name: cluster_name.unwrap(),
        })
    }

    pub fn get_cluster_config_by_url(
        &self,
        cluster_url: String,
    ) -> Result<&CliClusterConfig, CliConfigError> {
        // Cluster url should look like "https://localhost:5437/clusters/default/local-sso"
        let url_info = Self::proxy_url_to_tuple(&cluster_url)?;

        self.servers
            .get(&url_info.server_name)
            .and_then(|server| {
                server.get_clusters_from_name_ns(url_info.cluster_name, Some(url_info.namespace))
            })
            .ok_or_else(|| CliConfigError::ServerNotFound(cluster_url))
    }
}
