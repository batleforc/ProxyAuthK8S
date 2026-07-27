use tracing::{error, info, warn};

use crate::{cli_config::cli_server_config::CliServerConfig, ctx::CliCtx};

impl CliCtx {
    pub fn handle_set_def(
        &mut self,
        server_url: Option<&String>,
        namespace: Option<&String>,
        default_server: Option<&String>,
    ) {
        if let Some(default_server) = default_server {
            let default_server_name =
                CliServerConfig::url_to_name_from_string(default_server.clone());
            // Check if the server exists in the config
            if !self.config.servers.contains_key(&default_server_name) {
                warn!(
                    "Server name '{}' not found in config. Please login to add it first.",
                    default_server
                );
                return;
            }
            self.config.default_server_name = default_server_name;
            match self.config.write_to_file(self.config_path.clone()) {
                Ok(_) => info!("Default server set successfully to: {}", default_server),
                Err(e) => error!("Failed to set default server: {}", e),
            }
            return;
        }

        let (Some(server_url), Some(namespace)) = (server_url, namespace) else {
            warn!("Both server_url and namespace must be provided together.");
            return;
        };
        let server_name = CliServerConfig::url_to_name_from_string(server_url.clone());
        if let Some(server_config) = self.config.servers.get_mut(&server_name) {
            server_config.namespace = namespace.clone();
            match self.config.write_to_file(self.config_path.clone()) {
                Ok(_) => info!(
                    "Default namespace for server {} set successfully to: {}",
                    server_url, namespace
                ),
                Err(e) => error!("Failed to set default namespace: {}", e),
            }
        } else {
            warn!(
                "Server URL not found in config: {}, please login to add it first.",
                server_url
            );
        }
    }
}
