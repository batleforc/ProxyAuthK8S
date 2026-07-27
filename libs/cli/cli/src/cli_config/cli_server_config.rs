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
    #[must_use]
    pub fn new(server_url: String) -> Self {
        CliServerConfig {
            url: server_url,
            namespace: "default".to_string(),
            clusters: vec![].into_iter().collect(),
        }
    }
    #[must_use]
    pub fn url_to_name(&self) -> String {
        let url = self.url.replace("https://", "").replace("http://", "");
        url.replace(['.', ':'], "-")
    }

    #[must_use]
    pub fn url_to_name_from_string(url: String) -> String {
        let url = url.replace("https://", "").replace("http://", "");
        url.replace(['.', ':'], "-")
    }

    #[must_use]
    pub fn get_cluster_url_from_ns_name(&self, ns: Option<String>, name: String) -> Option<String> {
        let ns = ns.unwrap_or_else(|| self.namespace.clone());
        format!("{}/{}/{}", self.url, ns, name).into()
    }

    #[must_use]
    pub fn get_clusters_from_ns_name(
        &self,
        ns: Option<String>,
        name: String,
    ) -> Option<&CliClusterConfig> {
        self.clusters.get(&format!(
            "{}/{}",
            ns.unwrap_or_else(|| self.namespace.clone()),
            name
        ))
    }

    pub fn server_token(&self) -> Result<String, ProxyAuthK8sError> {
        let entry = match Entry::new("proxyauthk8s", &self.url) {
            Ok(entry) => entry,
            Err(err) => {
                debug!("Keyring entry creation error: {}", err);
                return Err(ProxyAuthK8sError::KeyringReadError(format!(
                    "Failed to create keyring entry for server URL: {}",
                    self.url
                )));
            }
        };
        match entry.get_password() {
            Ok(token) => Ok(token),
            Err(err) => {
                debug!("Keyring read error: {}", err);
                Err(ProxyAuthK8sError::KeyringReadError(format!(
                    "Failed to read token from keyring for server URL: {}",
                    self.url
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
                    self.url
                )));
            }
        };
        match entry.set_password(&token) {
            Ok(()) => Ok(()),
            Err(err) => {
                debug!("Keyring write error: {}", err);
                Err(ProxyAuthK8sError::KeyringWriteError(format!(
                    "Failed to write token to keyring for server URL: {}",
                    self.url
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
                    self.url
                )));
            }
        };
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(err) => {
                debug!("Keyring delete error: {}", err);
                Err(ProxyAuthK8sError::KeyringDeleteError(format!(
                    "Failed to delete token from keyring for server URL: {}",
                    self.url
                )))
            }
        }
    }

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

    pub fn set_cluster_token(
        &mut self,
        ns: String,
        cluster: String,
        token: String,
    ) -> Result<(), ProxyAuthK8sError> {
        let key = format!("{ns}/{cluster}");
        let _ = self
            .clusters
            .entry(key.clone())
            .or_insert(CliClusterConfig { token_exist: true });
        let entry = match Entry::new("proxyauthk8s", &format!("{}::{}", self.url, key)) {
            Ok(entry) => entry,
            Err(err) => {
                debug!("Keyring entry creation error: {}", err);
                return Err(ProxyAuthK8sError::KeyringWriteError(format!(
                    "Failed to create keyring entry for server URL: {}",
                    self.url
                )));
            }
        };
        match entry.set_password(&token) {
            Ok(()) => Ok(()),
            Err(err) => {
                debug!("Keyring write error: {}", err);
                Err(ProxyAuthK8sError::KeyringWriteError(format!(
                    "Failed to write token to keyring for server URL: {}",
                    self.url
                )))
            }
        }
    }

    pub fn get_cluster_token(
        &self,
        ns: String,
        cluster: String,
    ) -> Result<String, ProxyAuthK8sError> {
        let key = format!("{ns}/{cluster}");
        let entry = match Entry::new("proxyauthk8s", &format!("{}::{}", self.url, key)) {
            Ok(entry) => entry,
            Err(err) => {
                debug!("Keyring entry creation error: {}", err);
                return Err(ProxyAuthK8sError::KeyringReadError(format!(
                    "Failed to create keyring entry for server URL: {}",
                    self.url
                )));
            }
        };
        match entry.get_password() {
            Ok(token) => Ok(token),
            Err(err) => {
                debug!("Keyring read error: {}", err);
                Err(ProxyAuthK8sError::KeyringReadError(format!(
                    "Failed to read token from keyring for server URL: {}",
                    self.url
                )))
            }
        }
    }

    pub fn clear_cluster_token(
        &self,
        ns: String,
        cluster: String,
    ) -> Result<(), ProxyAuthK8sError> {
        let key = format!("{ns}/{cluster}");
        let entry = match Entry::new("proxyauthk8s", &format!("{}::{}", self.url, key)) {
            Ok(entry) => entry,
            Err(err) => {
                debug!("Keyring entry creation error: {}", err);
                return Err(ProxyAuthK8sError::KeyringDeleteError(format!(
                    "Failed to create keyring entry for server URL: {}",
                    self.url
                )));
            }
        };
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(err) => {
                debug!("Keyring delete error: {}", err);
                Err(ProxyAuthK8sError::KeyringDeleteError(format!(
                    "Failed to delete token from keyring for server URL: {}",
                    self.url
                )))
            }
        }
    }

    pub fn clear_all_tokens(&self) {
        for cluster in self.clusters.keys() {
            let val: Vec<&str> = cluster.split('/').collect();
            let ns = val.first().unwrap_or(&"").to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_to_name_strips_scheme_and_encodes_separators() {
        assert_eq!(
            CliServerConfig::url_to_name_from_string("https://localhost:5437".to_string()),
            "localhost-5437"
        );
        assert_eq!(
            CliServerConfig::url_to_name_from_string("http://proxy.example.com".to_string()),
            "proxy-example-com"
        );
        // Instance method agrees with the associated one.
        let config = CliServerConfig::new("https://a.b:1".to_string());
        assert_eq!(config.url_to_name(), "a-b-1");
    }

    #[test]
    fn cluster_url_uses_the_given_namespace_then_falls_back_to_the_default() {
        let config = CliServerConfig::new("https://localhost:5437".to_string());
        // Explicit namespace wins.
        assert_eq!(
            config.get_cluster_url_from_ns_name(Some("team-a".to_string()), "prod".to_string()),
            Some("https://localhost:5437/team-a/prod".to_string())
        );
        // None falls back to the server's default namespace ("default").
        assert_eq!(
            config.get_cluster_url_from_ns_name(None, "prod".to_string()),
            Some("https://localhost:5437/default/prod".to_string())
        );
    }

    #[test]
    fn clusters_are_looked_up_by_ns_and_name() {
        let mut config = CliServerConfig::new("https://localhost:5437".to_string());
        config.clusters.insert(
            "team-a/prod".to_string(),
            CliClusterConfig { token_exist: true },
        );

        assert!(config
            .get_clusters_from_ns_name(Some("team-a".to_string()), "prod".to_string())
            .is_some());
        // Wrong namespace -> not found.
        assert!(config
            .get_clusters_from_ns_name(Some("team-b".to_string()), "prod".to_string())
            .is_none());
        // None uses the default namespace, which has no "prod" entry here.
        assert!(config
            .get_clusters_from_ns_name(None, "prod".to_string())
            .is_none());
    }
}
