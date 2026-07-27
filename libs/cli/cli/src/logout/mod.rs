use tracing::{error, info};

use crate::{cli_config::cli_server_config::CliServerConfig, ctx::CliCtx};

impl CliCtx {
    /// Log out of a cluster (when `cluster_name` is given) or of the whole server
    /// (otherwise), removing the corresponding token(s) from the OS keyring and
    /// updating the config.
    pub fn handle_logout(&mut self, cluster_name: Option<String>) {
        if self.server_url.is_empty() && self.config.default_server_name.is_empty() {
            error!("Not logged in to any server; nothing to log out from.");
            return;
        }
        let server_name = if self.server_url.is_empty() {
            self.config.default_server_name.clone()
        } else {
            CliServerConfig::url_to_name_from_string(self.server_url.clone())
        };

        // Clone so the keyring operations do not hold a borrow on `self.config`
        // while we later mutate it.
        let Some(server_config) = self.config.servers.get(&server_name).cloned() else {
            error!(
                "Server '{}' not found in configuration; nothing to do.",
                server_name
            );
            return;
        };

        if let Some(cluster) = cluster_name {
            let namespace = if self.namespace.is_empty() {
                server_config.namespace.clone()
            } else {
                self.namespace.clone()
            };
            match server_config.clear_cluster_token(namespace.clone(), cluster.clone()) {
                Ok(()) => info!("Logged out of cluster '{}/{}'.", namespace, cluster),
                Err(e) => {
                    error!("Failed to remove cluster token from keyring: {}", e);
                    return;
                }
            }
            if let Some(server) = self.config.servers.get_mut(&server_name) {
                server.clusters.remove(&format!("{namespace}/{cluster}"));
            }
        } else {
            // Log out of the server: drop the server token and every cluster
            // token cached under it.
            server_config.clear_all_tokens();
            self.config.servers.remove(&server_name);
            if self.config.default_server_name == server_name {
                self.config.default_server_name = String::new();
            }
            info!("Logged out of server '{}'.", server_name);
        }

        if let Err(e) = self.config.write_to_file(self.config_path.clone()) {
            error!("Failed to update config file: {}", e);
        }
    }

    /// Clear every cached credential (all server and cluster tokens) from the
    /// keyring and reset the stored configuration.
    pub fn handle_cache_clear(&mut self) {
        self.config.clear();
        match self.config.write_to_file(self.config_path.clone()) {
            Ok(_) => info!("All cached tokens cleared."),
            Err(e) => error!("Failed to update config file after clearing cache: {}", e),
        }
    }
}
