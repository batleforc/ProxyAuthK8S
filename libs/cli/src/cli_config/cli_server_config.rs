use crate::{cli_config::cli_cluster_config::CliClusterConfig, error::ProxyAuthK8sError};
use client_api::{
    apis::{api_clusters_api::get_all_visible_cluster, configuration::Configuration},
    models::GetAllVisibleClusterBody,
};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, error};

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

    pub fn get_cluster_url_from_ns_name(&self, ns: Option<String>, name: String) -> Option<String> {
        let ns = ns.unwrap_or_else(|| self.namespace.clone());
        format!("{}/{}/{}", self.url, ns, name).into()
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

    pub fn clear_server_token(&self) -> Result<(), ProxyAuthK8sError> {
        let entry = match Entry::new("proxyauthk8s", &self.url) {
            Ok(entry) => entry,
            Err(err) => {
                debug!("Keyring entry creation error: {}", err);
                return Err(ProxyAuthK8sError::KeyringDeleteError(format!(
                    "Failed to create keyring entry for server URL: {}",
                    &self.url
                )));
            }
        };
        match entry.delete_credential() {
            Ok(_) => Ok(()),
            Err(err) => {
                debug!("Keyring delete error: {}", err);
                Err(ProxyAuthK8sError::KeyringDeleteError(format!(
                    "Failed to delete token from keyring for server URL: {}",
                    &self.url
                )))
            }
        }
    }

    pub fn get_base_configuration(&self) -> Result<Configuration, ProxyAuthK8sError> {
        let token = self.get_server_token()?;
        Ok(Configuration {
            base_path: self.url.clone(),
            bearer_access_token: Some(token),
            ..Default::default()
        })
    }

    pub async fn get_clusters_from_remote(
        &self,
    ) -> Result<GetAllVisibleClusterBody, ProxyAuthK8sError> {
        get_all_visible_cluster(&self.get_base_configuration()?)
            .await
            .map_err(|e| {
                debug!("Error fetching clusters from remote: {:?}", e);
                e.into()
            })
    }

    pub fn set_cluster_token(
        &mut self,
        ns: String,
        cluster: String,
        token: String,
    ) -> Result<(), ProxyAuthK8sError> {
        let key = format!("{}/{}", ns, cluster);
        let _ = self
            .clusters
            .entry(key.clone())
            .or_insert(CliClusterConfig { token_exist: true });
        let entry = match Entry::new("proxyauthk8s", &format!("{}::{}", &self.url, &key)) {
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

    pub fn get_cluster_token(
        &self,
        ns: String,
        cluster: String,
    ) -> Result<String, ProxyAuthK8sError> {
        let key = format!("{}/{}", ns, cluster);
        let entry = match Entry::new("proxyauthk8s", &format!("{}::{}", &self.url, &key)) {
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

    pub fn clear_cluster_token(
        &self,
        ns: String,
        cluster: String,
    ) -> Result<(), ProxyAuthK8sError> {
        let key = format!("{}/{}", ns, cluster);
        let entry = match Entry::new("proxyauthk8s", &format!("{}::{}", &self.url, &key)) {
            Ok(entry) => entry,
            Err(err) => {
                debug!("Keyring entry creation error: {}", err);
                return Err(ProxyAuthK8sError::KeyringDeleteError(format!(
                    "Failed to create keyring entry for server URL: {}",
                    &self.url
                )));
            }
        };
        match entry.delete_credential() {
            Ok(_) => Ok(()),
            Err(err) => {
                debug!("Keyring delete error: {}", err);
                Err(ProxyAuthK8sError::KeyringDeleteError(format!(
                    "Failed to delete token from keyring for server URL: {}",
                    &self.url
                )))
            }
        }
    }

    pub fn clear_all_tokens(&self) {
        for cluster in self.clusters.keys() {
            let val: Vec<&str> = cluster.split("/").collect();
            let ns = val.get(0).unwrap_or(&"").to_string();
            let cluster = val.get(1).unwrap_or(&"").to_string();
            if let Err(err) = self.clear_cluster_token(ns, cluster.clone()) {
                error!("Error clearing token for cluster {}: {}", cluster, err);
            }
        }
        if let Err(err) = self.clear_server_token() {
            error!("Error clearing server token: {}", err);
        }
    }
}
