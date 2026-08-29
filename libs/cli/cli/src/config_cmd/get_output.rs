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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_config::cli_cluster_config::CliClusterConfig;

    fn server_config() -> CliServerConfig {
        CliServerConfig::new("https://localhost:5437".to_string())
    }

    #[test]
    fn a_row_lines_up_with_the_headers() {
        let output = GetOutput::new_from_servers(server_config(), "localhost-5437".to_string());
        assert_eq!(
            GetOutput::headers(),
            vec![
                "Is Default",
                "Server URL",
                "Default Namespace",
                "Has Clusters"
            ]
        );
        assert_eq!(
            output.row(),
            vec!["true", "https://localhost:5437", "default", "false"]
        );
        assert_eq!(GetOutput::headers().len(), output.row().len());
    }

    #[test]
    fn is_default_compares_the_derived_name_not_the_raw_url() {
        // The default is stored under the name form ("localhost-5437"), so the
        // comparison has to go through url_to_name rather than the URL itself.
        let output = GetOutput::new_from_servers(server_config(), "localhost-5437".to_string());
        assert!(output.is_default);
        assert_eq!(output.url, "https://localhost:5437");

        let output =
            GetOutput::new_from_servers(server_config(), "https://localhost:5437".to_string());
        assert!(!output.is_default);

        let output = GetOutput::new_from_servers(server_config(), String::new());
        assert!(!output.is_default);
    }

    #[test]
    fn has_clusters_reports_whether_any_cluster_is_known() {
        let mut config = server_config();
        assert!(!GetOutput::new_from_servers(config.clone(), String::new()).has_clusters);

        config.clusters.insert(
            "default/prod".to_string(),
            CliClusterConfig { token_exist: true },
        );
        let output = GetOutput::new_from_servers(config, String::new());
        assert!(output.has_clusters);
        assert_eq!(output.row()[3], "true");
    }

    #[test]
    fn the_default_namespace_comes_from_the_server_config() {
        let mut config = server_config();
        config.namespace = "team-a".to_string();
        let output = GetOutput::new_from_servers(config, String::new());
        assert_eq!(output.default_namespace, "team-a");
        assert_eq!(output.row()[2], "team-a");
    }
}
