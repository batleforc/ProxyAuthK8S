use tracing::{error, info, warn};

use crate::{cli_config::cli_server_config::CliServerConfig, ctx::CliCtx};

impl CliCtx {
    pub fn handle_clear_config(&mut self, all: bool, server_url: Option<&String>) {
        if all {
            match self.config.clear().write_to_file(self.config_path.clone()) {
                Ok(_) => info!("All configurations cleared successfully."),
                Err(e) => error!("Failed to clear configurations: {}", e),
            }
            return;
        }
        let Some(server_url) = server_url else {
            warn!("Please provide either --all to clear all configurations or --server_url to clear a specific server configuration.");
            return;
        };
        let server_name = CliServerConfig::url_to_name_from_string(server_url.clone());
        if self.config.default_server_name == server_name {
            self.config.default_server_name = String::new();
        }
        if let Some(server) = self.config.servers.get(&server_name) {
            server.clear_all_tokens();
        }
        if self.config.servers.remove(&server_name).is_some() {
            match self.config.write_to_file(self.config_path.clone()) {
                Ok(_) => info!(
                    "Configuration for server URL {} cleared successfully.",
                    server_url
                ),
                Err(e) => error!("Failed to clear configuration: {}", e),
            }
        } else {
            warn!("Server URL {} not found in configuration.", server_url);
        }
    }
}
