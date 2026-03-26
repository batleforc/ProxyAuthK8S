use comfy_table::Table;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::{
    ctx::{CliCtx, ContextFormat},
    error::ProxyAuthK8sError,
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetClusterOutput {
    pub name: String,
    pub namespace: String,
    pub enabled: bool,
    pub is_reachable: Option<bool>,
    pub sso_enabled: bool,
}

impl GetClusterOutput {
    pub fn to_row(&self) -> Vec<String> {
        vec![
            self.name.clone(),
            self.namespace.clone(),
            self.enabled.to_string(),
            self.is_reachable
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            self.sso_enabled.to_string(),
        ]
    }

    pub fn to_row_headers() -> Vec<String> {
        vec![
            "NAME".to_string(),
            "NAMESPACE".to_string(),
            "ENABLED".to_string(),
            "REACHABLE".to_string(),
            "SSO".to_string(),
        ]
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VecGetClusterOutput {
    pub api_version: String,
    pub kind: String,
    pub metadata: Option<serde_json::Value>,
    pub items: Vec<GetClusterOutput>,
}

impl VecGetClusterOutput {
    pub fn new(items: Vec<GetClusterOutput>) -> Self {
        VecGetClusterOutput {
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
        table.set_header(GetClusterOutput::to_row_headers());
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

impl CliCtx {
    pub async fn handle_get_clusters(&mut self, cluster_name: Option<String>) {
        let server_config =
            match self
                .config
                .get_server_config_by_url(if self.server_url.is_empty() {
                    None
                } else {
                    Some(self.server_url.clone())
                }) {
                Ok(config) => config,
                Err(e) => {
                    error!(
                        "Error retrieving server configuration, please login to server first: {}",
                        e
                    );
                    return;
                }
            };

        let namespace_filter = if self.namespace.is_empty() {
            None
        } else {
            Some(self.namespace.clone())
        };

        match server_config.get_clusters_from_remote().await {
            Ok(clusters) => {
                let mut outputs: Vec<GetClusterOutput> = clusters
                    .clusters
                    .iter()
                    .filter(|cluster| {
                        let namespace_matches = namespace_filter
                            .as_ref()
                            .is_none_or(|namespace| cluster.namespace == *namespace);
                        let name_matches = cluster_name
                            .as_ref()
                            .is_none_or(|name| cluster.name == *name);
                        namespace_matches && name_matches
                    })
                    .map(|cluster| GetClusterOutput {
                        name: cluster.name.clone(),
                        namespace: cluster.namespace.clone(),
                        enabled: cluster.enabled,
                        is_reachable: cluster.is_reachable.flatten(),
                        sso_enabled: cluster.sso_enabled,
                    })
                    .collect();

                outputs.sort_by(|a, b| {
                    a.namespace
                        .cmp(&b.namespace)
                        .then_with(|| a.name.cmp(&b.name))
                });

                let output = VecGetClusterOutput::new(outputs);
                println!("{}", output.to_output(self.format.clone()));
            }
            Err(e) => {
                error!("Failed to retrieve clusters: {}", e);
                match e {
                    ProxyAuthK8sError::Unauthenticated(_) => {
                        info!("Authentication failed: invalid or missing server token. Please run login first.");
                    }
                    ProxyAuthK8sError::RemoteServerError(_) => {
                        info!("Server error occurred while retrieving clusters.");
                    }
                    _ => {
                        info!("An unexpected error occurred while retrieving clusters.");
                    }
                }
            }
        }
    }
}
