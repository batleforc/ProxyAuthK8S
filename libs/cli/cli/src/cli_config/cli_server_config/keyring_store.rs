//! Bearer-token storage, backed by the OS keyring.
//!
//! Two kinds of entry are kept under the `proxyauthk8s` service: the server
//! token, keyed by the server URL, and the per-cluster tokens, keyed by
//! `<server url>::<ns>/<cluster>`.

use keyring_core::Entry;
use tracing::{debug, error};

use crate::{
    cli_config::{cli_cluster_config::CliClusterConfig, cli_server_config::CliServerConfig},
    error::ProxyAuthK8sError,
    keystore,
};

/// Keyring service every server and cluster token is stored under.
const KEYRING_SERVICE: &str = "proxyauthk8s";

impl CliServerConfig {
    pub fn server_token(&self) -> Result<String, ProxyAuthK8sError> {
        self.read_token(&self.url)
    }

    pub fn set_server_token(&self, token: String) -> Result<(), ProxyAuthK8sError> {
        self.write_token(&self.url, &token)
    }

    pub fn clear_server_token(&self) -> Result<(), ProxyAuthK8sError> {
        self.delete_token(&self.url)
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
        self.write_token(&self.cluster_keyring_user(&key), &token)
    }

    pub fn get_cluster_token(
        &self,
        ns: String,
        cluster: String,
    ) -> Result<String, ProxyAuthK8sError> {
        self.read_token(&self.cluster_keyring_user(&format!("{ns}/{cluster}")))
    }

    pub fn clear_cluster_token(
        &self,
        ns: String,
        cluster: String,
    ) -> Result<(), ProxyAuthK8sError> {
        self.delete_token(&self.cluster_keyring_user(&format!("{ns}/{cluster}")))
    }

    /// Keyring user under which the token of cluster `key` (`"<ns>/<cluster>"`)
    /// is stored. Changing this format would orphan every stored cluster token.
    fn cluster_keyring_user(&self, key: &str) -> String {
        format!("{}::{}", self.url, key)
    }

    /// Open the keyring entry for `user` under [`KEYRING_SERVICE`], mapping a
    /// failure to the caller's error variant (`err`).
    fn keyring_entry(
        &self,
        user: &str,
        err: fn(String) -> ProxyAuthK8sError,
    ) -> Result<Entry, ProxyAuthK8sError> {
        keystore::entry(KEYRING_SERVICE, user).map_err(|e| {
            debug!("Keyring entry creation error: {}", e);
            err(format!(
                "Failed to create keyring entry for server URL: {}",
                self.url
            ))
        })
    }

    fn read_token(&self, user: &str) -> Result<String, ProxyAuthK8sError> {
        let entry = self.keyring_entry(user, ProxyAuthK8sError::KeyringReadError)?;
        entry.get_password().map_err(|e| {
            debug!("Keyring read error: {}", e);
            ProxyAuthK8sError::KeyringReadError(format!(
                "Failed to read token from keyring for server URL: {}",
                self.url
            ))
        })
    }

    fn write_token(&self, user: &str, token: &str) -> Result<(), ProxyAuthK8sError> {
        let entry = self.keyring_entry(user, ProxyAuthK8sError::KeyringWriteError)?;
        entry.set_password(token).map_err(|e| {
            debug!("Keyring write error: {}", e);
            ProxyAuthK8sError::KeyringWriteError(format!(
                "Failed to write token to keyring for server URL: {}",
                self.url
            ))
        })
    }

    fn delete_token(&self, user: &str) -> Result<(), ProxyAuthK8sError> {
        let entry = self.keyring_entry(user, ProxyAuthK8sError::KeyringDeleteError)?;
        entry.delete_credential().map_err(|e| {
            debug!("Keyring delete error: {}", e);
            ProxyAuthK8sError::KeyringDeleteError(format!(
                "Failed to delete token from keyring for server URL: {}",
                self.url
            ))
        })
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
    fn cluster_keyring_user_keeps_the_stored_token_naming() {
        // Tokens already in users' keyrings are looked up under this exact
        // service/user pair; a format change would make them unreadable.
        assert_eq!(KEYRING_SERVICE, "proxyauthk8s");
        let config = CliServerConfig::new("https://localhost:5437".to_string());
        assert_eq!(
            config.cluster_keyring_user("team-a/prod"),
            "https://localhost:5437::team-a/prod"
        );
    }

    // The tests below run against the in-memory mock store `keystore` selects
    // under `cfg(test)`. Each uses its own server URL so they stay independent
    // when run in one process (`cargo test`).

    #[test]
    fn server_token_round_trips_through_the_keyring() {
        let config = CliServerConfig::new("https://tokens.example".to_string());
        assert!(matches!(
            config.server_token(),
            Err(ProxyAuthK8sError::KeyringReadError(_))
        ));

        config.set_server_token("tok".to_string()).unwrap();
        assert_eq!(config.server_token().unwrap(), "tok");
        // Stored under the documented service/user pair.
        let raw = keystore::entry(KEYRING_SERVICE, "https://tokens.example").unwrap();
        assert_eq!(raw.get_password().unwrap(), "tok");

        config.clear_server_token().unwrap();
        assert!(config.server_token().is_err());
        // Nothing left to delete.
        assert!(matches!(
            config.clear_server_token(),
            Err(ProxyAuthK8sError::KeyringDeleteError(_))
        ));
    }

    #[test]
    fn cluster_token_round_trips_and_marks_the_cluster_known() {
        let mut config = CliServerConfig::new("https://clusters.example".to_string());
        config
            .set_cluster_token("team-a".to_string(), "prod".to_string(), "ct".to_string())
            .unwrap();
        assert!(config.clusters["team-a/prod"].token_exist);
        assert_eq!(
            config
                .get_cluster_token("team-a".to_string(), "prod".to_string())
                .unwrap(),
            "ct"
        );
        let raw =
            keystore::entry(KEYRING_SERVICE, "https://clusters.example::team-a/prod").unwrap();
        assert_eq!(raw.get_password().unwrap(), "ct");

        config
            .clear_cluster_token("team-a".to_string(), "prod".to_string())
            .unwrap();
        assert!(matches!(
            config.get_cluster_token("team-a".to_string(), "prod".to_string()),
            Err(ProxyAuthK8sError::KeyringReadError(_))
        ));
    }

    #[test]
    fn keyring_failures_map_to_the_matching_error() {
        let config = CliServerConfig::new("https://failing.example".to_string());
        let raw = keystore::entry(KEYRING_SERVICE, "https://failing.example").unwrap();
        let mock: &keyring_core::mock::Cred = raw.as_any().downcast_ref().unwrap();
        mock.set_error(keyring_core::Error::NoStorageAccess(
            "locked".to_string().into(),
        ));
        assert!(matches!(
            config.set_server_token("tok".to_string()),
            Err(ProxyAuthK8sError::KeyringWriteError(_))
        ));
        // The injected error is one-shot: the next write goes through.
        config.set_server_token("tok".to_string()).unwrap();
    }

    #[test]
    fn clear_all_tokens_drops_the_server_and_every_cluster_token() {
        let mut config = CliServerConfig::new("https://all.example".to_string());
        config.set_server_token("st".to_string()).unwrap();
        for cluster in ["a", "b"] {
            config
                .set_cluster_token("ns".to_string(), cluster.to_string(), "ct".to_string())
                .unwrap();
        }
        // A cluster known in the config but without a stored token is only
        // logged, not fatal.
        config.clusters.insert(
            "ns/no-token".to_string(),
            CliClusterConfig { token_exist: true },
        );

        config.clear_all_tokens();

        assert!(config.server_token().is_err());
        for cluster in ["a", "b"] {
            assert!(
                config
                    .get_cluster_token("ns".to_string(), cluster.to_string())
                    .is_err()
            );
        }
        // Clearing again (nothing left) does not panic.
        config.clear_all_tokens();
    }
}
