use client_api::apis::{api_clusters_api::get_all_visible_cluster, configuration::Configuration};
use tracing::{debug, error, info};

use crate::{cli_config::cli_server_config::CliServerConfig, ctx::CliCtx};

impl CliCtx {
    pub async fn handle_login(&mut self, cluster_name: Option<String>, token: Option<String>) {
        // if server_url is not provided and none exist in config, return error
        if self.server_url.is_empty() && self.config.default_server_name.is_empty() {
            error!("Error: No ProxyAuthK8S server URL provided and no existing configuration found. Please provide a server URL using the --server-url option or login to an application first.");
            return;
        }
        if let Some(cluster) = cluster_name {
            debug!("Logging in to cluster: {}", cluster);
            if let Some(tok) = token {
                info!("Using provided token for authentication. {}", tok);
                // Use the token for authentication
            } else {
                info!("No token provided, this workflow is not yet implemented.");
                // Proceed without a token
            }
        } else {
            debug!("Logging in to ProxyAuthK8S server.");
            if let Some(tok) = token {
                info!("Using provided token for authentication. {}", tok);
                // Use the token for authentication
                // Try to get cluster info from server using the token
                let output = get_all_visible_cluster(&Configuration {
                    bearer_access_token: Some(tok.clone()),
                    base_path: if !self.server_url.is_empty() {
                        self.server_url.clone()
                    } else {
                        let def_server = self
                            .config
                            .servers
                            .get(&self.config.default_server_name)
                            .unwrap();
                        def_server.url.clone()
                    },
                    ..Default::default()
                })
                .await;

                match output {
                    Ok(clusters) => {
                        info!("Successfully retrieved clusters: {:?}", clusters);
                        // Get servers from config or insert if not existing
                        let (server_url, server_name) = if !self.server_url.is_empty() {
                            (
                                self.server_url.clone(),
                                CliServerConfig::url_to_name_from_string(self.server_url.clone()),
                            )
                        } else {
                            let def_server = self
                                .config
                                .servers
                                .get(&self.config.default_server_name.clone())
                                .unwrap();
                            (
                                def_server.url.clone(),
                                self.config.default_server_name.clone(),
                            )
                        };
                        let server_config = self
                            .config
                            .get_or_insert_server_config(server_name, server_url);
                        let server_config_clone = server_config.clone();
                        match self.config.write_to_file(self.config_path.clone()) {
                            Ok(_) => info!("Config file updated successfully."),
                            Err(e) => error!("Failed to update config file: {}", e),
                        };
                        match server_config_clone.set_server_token(tok.clone()) {
                            Ok(_) => info!("Token saved to keyring successfully."),
                            Err(e) => error!("Failed to save token to keyring: {}", e),
                        }
                    }
                    Err(e) => {
                        error!("Failed to retrieve clusters, : {}", e);
                        match e {
                            client_api::apis::Error::ResponseError(resp_content) => {
                                match resp_content.entity {
                                    Some(client_api::apis::api_clusters_api::GetAllVisibleClusterError::Status401()) => {
                                        error!("Authentication failed: Invalid token provided.");
                                    }
                                    Some(client_api::apis::api_clusters_api::GetAllVisibleClusterError::Status500()) => {
                                        error!("Server error occurred while retrieving clusters.");
                                    }
                                    _ => {
                                        error!("An unknown error occurred while retrieving clusters.");
                                    }
                                }
                            }
                            client_api::apis::Error::Serde(req_err) => {
                                error!("Invalid response from server: {}", req_err);
                            }
                            _ => {
                                error!("An unexpected error occurred: {}", e);
                            }
                        }
                    }
                }
            } else {
                info!("No token provided, this workflow is not yet implemented.");
                // Proceed without a token
            }
        }
    }
}
