use tracing::{info, warn};

use crate::{cli_config::cli_server_config::CliServerConfig, config::ConfigCommands, ctx::CliCtx};

impl ConfigCommands {
    pub fn handle_set_def(&self, ctx: &mut CliCtx) {
        if let ConfigCommands::SetDef {
            server_url,
            namespace,
            default_server,
        } = self
        {
            if let Some(default_server) = default_server {
                let default_server_name =
                    CliServerConfig::url_to_name_from_string(default_server.clone());
                // Check if the server exists in the config
                if !ctx.config.servers.contains_key(&default_server_name) {
                    warn!(
                        "Server name '{}' not found in config. Please login to add it first.",
                        default_server
                    );
                    return;
                }
                ctx.config.default_server_name = default_server_name;
                match ctx.config.write_to_file(ctx.config_path.clone()) {
                    Ok(_) => info!("Default server set successfully to: {}", default_server),
                    Err(e) => info!("Failed to set default server: {}", e),
                }
                return;
            }
            if server_url.is_none() || namespace.is_none() {
                info!("Both server_url and namespace must be provided together.");
                return;
            }
            let server_url = server_url.as_ref().unwrap();
            let namespace = namespace.as_ref().unwrap();
            let server_name = CliServerConfig::url_to_name_from_string(server_url.clone());
            if let Some(server_config) = ctx.config.servers.get_mut(&server_name) {
                server_config.namespace = namespace.clone();
                match ctx.config.write_to_file(ctx.config_path.clone()) {
                    Ok(_) => info!(
                        "Default namespace for server {} set successfully to: {}",
                        server_url, namespace
                    ),
                    Err(e) => info!("Failed to set default namespace: {}", e),
                }
            } else {
                warn!(
                    "Server URL not found in config: {}, please login to add it first.",
                    server_url
                );
            }
        }
    }
}
