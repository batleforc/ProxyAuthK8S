//! Bearer-token storage, backed by the OS keyring.
//!
//! Two kinds of entry are kept under the `proxyauthk8s` service: the server
//! token, keyed by the server URL, and the per-cluster tokens, keyed by
//! `<server url>::<ns>/<cluster>`.

use keyring::Entry;
use tracing::{debug, error};

use crate::{
    cli_config::{cli_cluster_config::CliClusterConfig, cli_server_config::CliServerConfig},
    error::ProxyAuthK8sError,
};

impl CliServerConfig {
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
