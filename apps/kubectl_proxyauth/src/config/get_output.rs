use comfy_table::Table;
use serde::{Deserialize, Serialize};
use std::vec;

use crate::{cli_config::cli_server_config::CliServerConfig, ctx::ContextFormat};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetOutput {
    pub is_default: bool,
    pub url: String,
    pub default_namespace: String,
    pub has_clusters: bool,
}

impl GetOutput {
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

    pub fn to_row(&self) -> Vec<String> {
        vec![
            self.is_default.to_string(),
            self.url.clone(),
            self.default_namespace.clone(),
            self.has_clusters.to_string(),
        ]
    }

    pub fn to_row_headers() -> Vec<String> {
        vec![
            "Is Default".to_string(),
            "Server URL".to_string(),
            "Default Namespace".to_string(),
            "Has Clusters".to_string(),
        ]
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VecGetOutput {
    pub api_version: String,
    pub kind: String,
    pub metadata: Option<serde_json::Value>,
    pub items: Vec<GetOutput>,
}

impl VecGetOutput {
    pub fn new(items: Vec<GetOutput>) -> Self {
        VecGetOutput {
            api_version: "v1".to_string(),
            kind: "List".to_string(),
            metadata: None,
            items,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn to_yaml(&self) -> String {
        serde_yaml::to_string(self).unwrap_or_default()
    }

    pub fn to_table(&self) -> String {
        let mut table = Table::new();
        table.load_preset(comfy_table::presets::NOTHING);
        table.set_header(GetOutput::to_row_headers());
        for item in &self.items {
            table.add_row(item.to_row());
        }
        table.to_string()
    }

    pub fn to_output(&self, format: ContextFormat) -> String {
        match format {
            ContextFormat::Json => self.to_json(),
            ContextFormat::Yaml => self.to_yaml(),
            ContextFormat::Table => self.to_table(),
        }
    }
}
