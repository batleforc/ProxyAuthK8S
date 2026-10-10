use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

pub mod browser;
pub mod cli_cluster_config;
pub mod cli_server_config;
pub mod error;

use cli_server_config::CliServerConfig;

use crate::{
    cli_config::{cli_cluster_config::CliClusterConfig, error::CliConfigError},
    error::ProxyAuthK8sError,
};

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
    #[must_use]
    pub fn new() -> Self {
        CliConfig {
            default_server_name: String::new(),
            servers: vec![].into_iter().collect(),
        }
    }

    pub fn clear(&mut self) -> &Self {
        // Clear all credentials and server configs
        for server in self.servers.values() {
            server.clear_all_tokens();
        }
        self.servers = HashMap::new();
        self.default_server_name = String::new();
        self
    }

    pub fn read_from_file(path: PathBuf) -> Result<Self, ProxyAuthK8sError> {
        let content = std::fs::read_to_string(path.clone()).map_err(|e| {
            ProxyAuthK8sError::KubeconfigReadError(format!(
                "Failed to read CLI config file at {}: {}",
                path.to_string_lossy(),
                e
            ))
        })?;
        Ok(Self::from_yaml(&content)?)
    }

    pub fn write_to_file(&self, path: PathBuf) -> Result<&Self, ProxyAuthK8sError> {
        let yaml_str = self.to_yaml().map_err(|e| {
            ProxyAuthK8sError::KubeconfigWriteError(format!(
                "Failed to serialize CLI config to YAML: {e}"
            ))
        })?;
        crate::helper::secure_write(path.clone(), &yaml_str).map_err(|e| {
            ProxyAuthK8sError::KubeconfigWriteError(format!(
                "Failed to write CLI config file at {}: {}",
                path.to_string_lossy(),
                e
            ))
        })?;
        Ok(self)
    }

    pub fn from_yaml(yaml_str: &str) -> Result<Self, CliConfigError> {
        match serde_yaml_ng::from_str::<CliConfig>(yaml_str) {
            Ok(config) => Ok(config),
            Err(err) => Err(CliConfigError::YamlParseError(err.to_string())),
        }
    }

    pub fn to_yaml(&self) -> Result<String, CliConfigError> {
        match serde_yaml_ng::to_string(self) {
            Ok(yaml_str) => Ok(yaml_str),
            Err(err) => Err(CliConfigError::YamlSerializeError(err.to_string())),
        }
    }

    pub fn get_or_insert_server_config(
        &mut self,
        server_name: String,
        server_url: String,
    ) -> &mut CliServerConfig {
        self.servers
            .entry(server_name.clone())
            .or_insert_with(|| CliServerConfig::new(server_url))
    }

    pub fn proxy_url_to_tuple(url: &str) -> Result<UrlInfo, CliConfigError> {
        let parsed_url = match Url::parse(url) {
            Ok(u) => u,
            Err(err) => {
                return Err(CliConfigError::InvalidServerUrl(
                    url.to_string(),
                    err.to_string(),
                ));
            }
        };

        let Some(host) = parsed_url.host_str() else {
            return Err(CliConfigError::InvalidServerUrl(
                url.to_string(),
                "No host found in URL".to_string(),
            ));
        };
        // Derive the server name exactly like logins do
        // (`CliServerConfig::url_to_name`), INCLUDING the port — otherwise a
        // cluster-by-URL lookup keyed on `localhost` would never match a server
        // stored under `localhost-5437`. `host_str()` drops the port, so rebuild
        // the origin before running it through the shared name function.
        let origin = match parsed_url.port() {
            Some(port) => format!("{}://{host}:{port}", parsed_url.scheme()),
            None => format!("{}://{host}", parsed_url.scheme()),
        };
        let server_name = CliServerConfig::url_to_name_from_string(origin);
        let (Some(namespace), Some(cluster_name)) = (
            parsed_url
                .path_segments()
                .and_then(|mut segments| segments.nth(1))
                .map(std::string::ToString::to_string),
            parsed_url
                .path_segments()
                .and_then(|mut segments| segments.nth(2))
                .map(std::string::ToString::to_string),
        ) else {
            return Err(CliConfigError::InvalidServerUrl(
                url.to_string(),
                "Namespace or cluster name not found in URL path".to_string(),
            ));
        };

        Ok(UrlInfo {
            server_name,
            namespace,
            cluster_name,
        })
    }

    pub fn get_cluster_config_by_url(
        &self,
        cluster_url: String,
    ) -> Result<&CliClusterConfig, CliConfigError> {
        // Cluster url should look like "https://localhost:5437/clusters/default/local-sso"
        let url_info = Self::proxy_url_to_tuple(&cluster_url)?;

        // Distinguish "no such server" from "server exists but no such cluster"
        // so the caller can tell a wrong host from a wrong cluster.
        let server = self
            .servers
            .get(&url_info.server_name)
            .ok_or_else(|| CliConfigError::ServerNotFound(url_info.server_name.clone()))?;
        server
            .get_clusters_from_ns_name(Some(url_info.namespace), url_info.cluster_name.clone())
            .ok_or(CliConfigError::ClusterNotFound {
                server: url_info.server_name,
                cluster: url_info.cluster_name,
            })
    }

    pub fn get_server_config_by_url(
        &self,
        server_url: Option<String>,
    ) -> Result<&CliServerConfig, CliConfigError> {
        let server_name = match server_url {
            Some(url) => CliServerConfig::url_to_name_from_string(url),
            None => self.default_server_name.clone(),
        };
        self.servers
            .get(&server_name)
            .ok_or(CliConfigError::ServerNotFound(server_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_url_to_tuple_extracts_server_ns_and_cluster() {
        let info = CliConfig::proxy_url_to_tuple("https://proxy.example.com/clusters/team-a/prod")
            .expect("a well-formed proxy URL parses");
        assert_eq!(info.server_name, "proxy-example-com");
        assert_eq!(info.namespace, "team-a");
        assert_eq!(info.cluster_name, "prod");
    }

    #[test]
    fn proxy_url_to_tuple_rejects_malformed_urls() {
        // Not a URL at all.
        assert!(CliConfig::proxy_url_to_tuple("not a url").is_err());
        // Missing the namespace/cluster path segments.
        assert!(CliConfig::proxy_url_to_tuple("https://localhost:5437").is_err());
    }

    /// A config holding one server (`localhost-5437`) with one cluster
    /// (`team-a/prod`), the shape every lookup below is exercised against.
    fn config_with_one_cluster() -> CliConfig {
        let mut config = CliConfig::new();
        let server = config.get_or_insert_server_config(
            "localhost-5437".to_string(),
            "https://localhost:5437".to_string(),
        );
        server.clusters.insert(
            "team-a/prod".to_string(),
            CliClusterConfig { token_exist: true },
        );
        config.default_server_name = "localhost-5437".to_string();
        config
    }

    #[test]
    fn a_new_config_is_empty_and_matches_the_default() {
        let config = CliConfig::new();
        assert!(config.default_server_name.is_empty());
        assert!(config.servers.is_empty());
        let default = CliConfig::default();
        assert!(default.default_server_name.is_empty());
        assert!(default.servers.is_empty());
    }

    #[test]
    fn get_or_insert_server_config_inserts_once_then_hands_back_the_same_entry() {
        let mut config = CliConfig::new();
        let server = config.get_or_insert_server_config(
            "localhost-5437".to_string(),
            "https://localhost:5437".to_string(),
        );
        assert_eq!(server.url, "https://localhost:5437");
        assert_eq!(server.namespace, "default");
        server.namespace = "team-a".to_string();

        // A second call with a different URL must not overwrite what is stored.
        let server = config.get_or_insert_server_config(
            "localhost-5437".to_string(),
            "https://elsewhere:1234".to_string(),
        );
        assert_eq!(server.url, "https://localhost:5437");
        assert_eq!(server.namespace, "team-a");
        assert_eq!(config.servers.len(), 1);
    }

    #[test]
    fn a_config_round_trips_through_yaml() {
        let config = config_with_one_cluster();
        let yaml = config.to_yaml().expect("the config serializes");
        let parsed = CliConfig::from_yaml(&yaml).expect("the config parses back");

        assert_eq!(parsed.default_server_name, "localhost-5437");
        let server = parsed
            .servers
            .get("localhost-5437")
            .expect("the server survives the round trip");
        assert_eq!(server.url, "https://localhost:5437");
        assert!(
            server
                .get_clusters_from_ns_name(Some("team-a".to_string()), "prod".to_string())
                .is_some()
        );
    }

    #[test]
    fn from_yaml_reports_a_parse_error_rather_than_panicking() {
        let err = CliConfig::from_yaml("default_server_name: [not, a, string]")
            .expect_err("a mistyped field is rejected");
        assert!(matches!(err, CliConfigError::YamlParseError(_)));
    }

    #[test]
    fn get_server_config_by_url_falls_back_to_the_default_server() {
        let config = config_with_one_cluster();

        // An explicit URL is resolved through url_to_name.
        assert_eq!(
            config
                .get_server_config_by_url(Some("https://localhost:5437".to_string()))
                .expect("the server is configured")
                .url,
            "https://localhost:5437"
        );
        // None means "whatever the default server is".
        assert_eq!(
            config
                .get_server_config_by_url(None)
                .expect("the default server is configured")
                .url,
            "https://localhost:5437"
        );
    }

    #[test]
    fn get_server_config_by_url_names_the_server_it_could_not_find() {
        let config = config_with_one_cluster();
        assert!(matches!(
            config.get_server_config_by_url(Some("https://other.example.com".to_string())),
            Err(CliConfigError::ServerNotFound(name)) if name == "other-example-com"
        ));

        // With no default set, None resolves to the empty name and also fails.
        let empty = CliConfig::new();
        assert!(matches!(
            empty.get_server_config_by_url(None),
            Err(CliConfigError::ServerNotFound(name)) if name.is_empty()
        ));
    }

    #[test]
    fn get_cluster_config_by_url_resolves_a_full_proxy_url() {
        let config = config_with_one_cluster();
        let cluster = config
            .get_cluster_config_by_url("https://localhost:5437/clusters/team-a/prod".to_string())
            .expect("the cluster is configured");
        assert!(cluster.token_exist);
    }

    #[test]
    fn get_cluster_config_by_url_tells_a_wrong_host_from_a_wrong_cluster() {
        let config = config_with_one_cluster();

        // Unknown host -> ServerNotFound.
        assert!(matches!(
            config.get_cluster_config_by_url(
                "https://other.example.com/clusters/team-a/prod".to_string()
            ),
            Err(CliConfigError::ServerNotFound(name)) if name == "other-example-com"
        ));
        // Known host, unknown cluster -> ClusterNotFound, naming both.
        assert!(matches!(
            config
                .get_cluster_config_by_url("https://localhost:5437/clusters/team-a/dev".to_string()),
            Err(CliConfigError::ClusterNotFound { server, cluster })
                if server == "localhost-5437" && cluster == "dev"
        ));
        // A URL that is not a proxy URL at all fails before either lookup.
        assert!(matches!(
            config.get_cluster_config_by_url("https://localhost:5437".to_string()),
            Err(CliConfigError::InvalidServerUrl(_, _))
        ));
    }

    #[test]
    fn clear_resets_the_servers_and_the_default_name() {
        // Only exercised on a config with no server: clearing one would reach
        // for the OS keyring, which is not available to the test suite.
        let mut config = CliConfig::new();
        config.default_server_name = "localhost-5437".to_string();

        let cleared = config.clear();
        assert!(cleared.servers.is_empty());
        assert!(cleared.default_server_name.is_empty());
    }

    #[test]
    fn proxy_url_to_tuple_server_name_keeps_the_port_and_matches_url_to_name() {
        // Regression: the server name must include the port so a cluster-by-URL
        // lookup matches the key a server login stores (url_to_name keeps it).
        let info = CliConfig::proxy_url_to_tuple("https://localhost:5437/clusters/default/local")
            .expect("a well-formed proxy URL parses");
        assert_eq!(info.server_name, "localhost-5437");
        assert_eq!(
            info.server_name,
            CliServerConfig::url_to_name_from_string("https://localhost:5437".to_string())
        );
    }
}
