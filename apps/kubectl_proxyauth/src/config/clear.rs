use tracing::{error, info, warn};

use crate::config::ConfigCommands;

impl ConfigCommands {
    pub fn handle_clear(&self, ctx: &mut crate::ctx::CliCtx) {
        if let ConfigCommands::Clear { all, server_url } = self {
            if *all {
                match ctx.config.clear().write_to_file(ctx.config_path.clone()) {
                    Ok(_) => info!("All configurations cleared successfully."),
                    Err(e) => error!("Failed to clear configurations: {}", e),
                }
                return;
            } else if let Some(server_url) = server_url {
                let server_name =
                    crate::cli_config::cli_server_config::CliServerConfig::url_to_name_from_string(
                        server_url.clone(),
                    );
                if ctx.config.default_server_name == server_name {
                    ctx.config.default_server_name = "".to_string();
                }
                if ctx.config.servers.remove(&server_name).is_some() {
                    match ctx.config.write_to_file(ctx.config_path.clone()) {
                        Ok(_) => info!(
                            "Configuration for server URL {} cleared successfully.",
                            server_url
                        ),
                        Err(e) => error!("Failed to clear configuration: {}", e),
                    }
                } else {
                    warn!("Server URL {} not found in configuration.", server_url);
                }
                return;
            } else {
                warn!("Please provide either --all to clear all configurations or --server_url to clear a specific server configuration.");
            }
        }
    }
}
