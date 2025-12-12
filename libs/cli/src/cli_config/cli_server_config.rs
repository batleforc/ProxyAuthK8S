use crate::{cli_config::cli_cluster_config::CliClusterConfig, error::ProxyAuthK8sError};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CliServerConfig {
    pub url: String,
    pub namespace: String,
    pub clusters: HashMap<String, CliClusterConfig>,
}

impl CliServerConfig {
    pub fn new(server_url: String) -> Self {
        CliServerConfig {
            url: server_url,
            namespace: "default".to_string(),
            clusters: vec![].into_iter().collect(),
        }
    }
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

    pub fn get_server_token(&self) -> Result<String, ProxyAuthK8sError> {
        let entry = match Entry::new("proxyauthk8s", &self.url) {
            Ok(entry) => entry,
            Err(err) => {
                debug!("Keyring entry creation error: {}", err);
                return Err(ProxyAuthK8sError::KeyringReadError(format!(
                    "Failed to create keyring entry for server URL: {}",
                    &self.url
                )));
            }
        };
        match entry.get_password() {
            Ok(token) => Ok(token),
            Err(err) => {
                debug!("Keyring read error: {}", err);
                Err(ProxyAuthK8sError::KeyringReadError(format!(
                    "Failed to read token from keyring for server URL: {}",
                    &self.url
                )))
            }
        }
    }

    pub fn set_server_token(&self, token: String) -> Result<(), ProxyAuthK8sError> {
        let entry = match Entry::new("proxyauthk8s", &self.url) {
            Ok(entry) => entry,
            Err(err) => {
                debug!("Keyring entry creation error: {}", err);
                return Err(ProxyAuthK8sError::KeyringWriteError(format!(
                    "Failed to create keyring entry for server URL: {}",
                    &self.url
                )));
            }
        };
        match entry.set_password(&token) {
            Ok(_) => Ok(()),
            Err(err) => {
                debug!("Keyring write error: {}", err);
                Err(ProxyAuthK8sError::KeyringWriteError(format!(
                    "Failed to write token to keyring for server URL: {}",
                    &self.url
                )))
            }
        }
    }
}
