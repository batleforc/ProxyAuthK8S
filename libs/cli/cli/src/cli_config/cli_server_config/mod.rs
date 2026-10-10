//! One configured `ProxyAuthK8S` server and the clusters seen through it.
//!
//! This module holds the shape and the pure URL/lookup helpers. The two sides
//! with an external dependency live next to it: `keyring_store.rs` for the OS
//! keyring that holds the bearer tokens, and `remote.rs` for the calls to the
//! server itself.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::cli_config::{browser::BrowserConfig, cli_cluster_config::CliClusterConfig};

mod keyring_store;
mod remote;

pub use remote::{http_client, load_certificate_authority};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CliServerConfig {
    pub url: String,
    pub namespace: String,
    pub clusters: HashMap<String, CliClusterConfig>,
    /// Base64 PEM bundle of the CA that signed the server's TLS certificate,
    /// for servers whose certificate the system does not trust (self-signed,
    /// internal CA). Same encoding as a kubeconfig `certificate-authority-data`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate_authority_data: Option<String>,
    /// Browser for the interactive SSO logins through this server, when it
    /// is not the system's default (`login --browser`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<BrowserConfig>,
}

impl CliServerConfig {
    #[must_use]
    pub fn new(server_url: String) -> Self {
        CliServerConfig {
            url: server_url,
            namespace: "default".to_string(),
            clusters: vec![].into_iter().collect(),
            certificate_authority_data: None,
            browser: None,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_authority_data_is_optional_in_the_config_file() {
        let config: CliServerConfig =
            serde_yaml_ng::from_str("url: https://a.b\nnamespace: default\nclusters: {}\n")
                .unwrap();
        assert_eq!(config.certificate_authority_data, None);
        let yaml = serde_yaml_ng::to_string(&config).unwrap();
        assert!(!yaml.contains("certificate_authority_data"));
    }

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

        assert!(
            config
                .get_clusters_from_ns_name(Some("team-a".to_string()), "prod".to_string())
                .is_some()
        );
        // Wrong namespace -> not found.
        assert!(
            config
                .get_clusters_from_ns_name(Some("team-b".to_string()), "prod".to_string())
                .is_none()
        );
        // None uses the default namespace, which has no "prod" entry here.
        assert!(
            config
                .get_clusters_from_ns_name(None, "prod".to_string())
                .is_none()
        );
    }
}
