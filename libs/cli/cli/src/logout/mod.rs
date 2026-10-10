use tracing::{error, info};

use crate::{
    cli_config::cli_server_config::CliServerConfig, ctx::CliCtx, error::ProxyAuthK8sError,
};

impl CliCtx {
    /// Log out of a cluster (when `cluster_name` is given) or of the whole server
    /// (otherwise), removing the corresponding token(s) from the OS keyring and
    /// updating the config.
    pub fn handle_logout(&mut self, cluster_name: Option<String>) -> Result<(), ProxyAuthK8sError> {
        if self.server_url.is_empty() && self.config.default_server_name.is_empty() {
            error!("Not logged in to any server; nothing to log out from.");
            return Err(ProxyAuthK8sError::InvalidUsage(
                "not logged in to any server".to_string(),
            ));
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
            return Err(ProxyAuthK8sError::ServerNotFound(server_name));
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
                    return Err(e);
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
            return Err(e);
        }
        Ok(())
    }

    /// Clear every cached credential (all server and cluster tokens) from the
    /// keyring and reset the stored configuration.
    pub fn handle_cache_clear(&mut self) -> Result<(), ProxyAuthK8sError> {
        self.config.clear();
        match self.config.write_to_file(self.config_path.clone()) {
            Ok(_) => {
                info!("All cached tokens cleared.");
                Ok(())
            }
            Err(e) => {
                error!("Failed to update config file after clearing cache: {}", e);
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_config::CliConfig;

    /// A context in `dir` logged in to `url` (the default server) and to its
    /// cluster `team-a/prod`. `url` is unique per test so keyring entries are
    /// not shared with another test of the same process.
    fn ctx_logged_in(dir: &std::path::Path, url: &str) -> CliCtx {
        let mut ctx = CliCtx::for_test_in(dir);
        let name = CliServerConfig::url_to_name_from_string(url.to_string());
        let server = ctx
            .config
            .get_or_insert_server_config(name.clone(), url.to_string());
        server.set_server_token("st".to_string()).unwrap();
        server
            .set_cluster_token("team-a".to_string(), "prod".to_string(), "ct".to_string())
            .unwrap();
        ctx.config.default_server_name = name;
        ctx
    }

    #[test]
    fn cluster_logout_drops_only_that_cluster() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_logged_in(dir.path(), "https://logout-cluster.example");
        ctx.namespace = "team-a".to_string();

        ctx.handle_logout(Some("prod".to_string())).unwrap();

        let server = &ctx.config.servers["logout-cluster-example"];
        assert!(server.clusters.is_empty());
        assert!(
            server
                .get_cluster_token("team-a".to_string(), "prod".to_string())
                .is_err()
        );
        assert_eq!(server.server_token().unwrap(), "st");
        let written = CliConfig::read_from_file(ctx.config_path.clone()).unwrap();
        assert!(
            written.servers["logout-cluster-example"]
                .clusters
                .is_empty()
        );

        // Already logged out: the keyring delete fails.
        assert!(matches!(
            ctx.handle_logout(Some("prod".to_string())),
            Err(ProxyAuthK8sError::KeyringDeleteError(_))
        ));
    }

    #[test]
    fn server_logout_drops_the_server_and_its_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_logged_in(dir.path(), "https://logout-server.example");
        let server = ctx.config.servers["logout-server-example"].clone();

        ctx.handle_logout(None).unwrap();

        assert!(ctx.config.servers.is_empty());
        assert!(ctx.config.default_server_name.is_empty());
        assert!(server.server_token().is_err());
        assert!(
            server
                .get_cluster_token("team-a".to_string(), "prod".to_string())
                .is_err()
        );
        let written = CliConfig::read_from_file(ctx.config_path.clone()).unwrap();
        assert!(written.servers.is_empty());
    }

    #[test]
    fn logout_needs_a_known_server_and_a_writable_config() {
        let mut ctx = CliCtx::for_test();
        assert!(matches!(
            ctx.handle_logout(None),
            Err(ProxyAuthK8sError::InvalidUsage(_))
        ));
        ctx.server_url = "https://unknown".to_string();
        assert!(matches!(
            ctx.handle_logout(None),
            Err(ProxyAuthK8sError::ServerNotFound(name)) if name == "unknown"
        ));

        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_logged_in(dir.path(), "https://logout-fail.example");
        ctx.config_path = dir.path().join("missing/config.yaml");
        assert!(matches!(
            ctx.handle_logout(None),
            Err(ProxyAuthK8sError::KubeconfigWriteError(_))
        ));
    }

    #[test]
    fn cache_clear_drops_everything() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_logged_in(dir.path(), "https://logout-cache.example");
        let server = ctx.config.servers["logout-cache-example"].clone();

        ctx.handle_cache_clear().unwrap();

        assert!(server.server_token().is_err());
        let written = CliConfig::read_from_file(ctx.config_path.clone()).unwrap();
        assert!(written.servers.is_empty());

        ctx.config_path = dir.path().join("missing/config.yaml");
        assert!(matches!(
            ctx.handle_cache_clear(),
            Err(ProxyAuthK8sError::KubeconfigWriteError(_))
        ));
    }
}
