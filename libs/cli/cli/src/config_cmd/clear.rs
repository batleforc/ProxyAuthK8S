use tracing::{error, info, warn};

use crate::{
    cli_config::cli_server_config::CliServerConfig, ctx::CliCtx, error::ProxyAuthK8sError,
};

impl CliCtx {
    pub fn handle_clear_config(
        &mut self,
        all: bool,
        server_url: Option<&String>,
    ) -> Result<(), ProxyAuthK8sError> {
        if all {
            return match self.config.clear().write_to_file(self.config_path.clone()) {
                Ok(_) => {
                    info!("All configurations cleared successfully.");
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to clear configurations: {}", e);
                    Err(e)
                }
            };
        }
        let Some(server_url) = server_url else {
            warn!(
                "Please provide either --all to clear all configurations or --server_url to clear a specific server configuration."
            );
            return Err(ProxyAuthK8sError::InvalidUsage(
                "provide either --all or --server_url".to_string(),
            ));
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
                Ok(_) => {
                    info!(
                        "Configuration for server URL {} cleared successfully.",
                        server_url
                    );
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to clear configuration: {}", e);
                    Err(e)
                }
            }
        } else {
            warn!("Server URL {} not found in configuration.", server_url);
            Err(ProxyAuthK8sError::ServerNotFound(server_name))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_config::CliConfig;

    const URL_B: &str = "https://clear-b.example";

    /// A context in `dir` with servers `url_a` (the default, with a stored
    /// server and cluster token) and B. `url_a` is unique per test so its
    /// keyring entries are not shared with another test of the same process.
    fn ctx_with_two_servers(dir: &std::path::Path, url_a: &str) -> CliCtx {
        let mut ctx = CliCtx::for_test_in(dir);
        let name_a = CliServerConfig::url_to_name_from_string(url_a.to_string());
        for url in [url_a, URL_B] {
            ctx.config.get_or_insert_server_config(
                CliServerConfig::url_to_name_from_string(url.to_string()),
                url.to_string(),
            );
        }
        ctx.config.default_server_name.clone_from(&name_a);
        let server = ctx.config.servers.get_mut(&name_a).unwrap();
        server.set_server_token("server-tok".to_string()).unwrap();
        server
            .set_cluster_token("ns".to_string(), "c".to_string(), "cluster-tok".to_string())
            .unwrap();
        ctx
    }

    fn written(ctx: &CliCtx) -> CliConfig {
        CliConfig::read_from_file(ctx.config_path.clone()).unwrap()
    }

    #[test]
    fn clear_all_drops_every_server_and_token() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_with_two_servers(dir.path(), "https://clear-all.example");
        let server_a = ctx.config.servers["clear-all-example"].clone();

        ctx.handle_clear_config(true, None).unwrap();

        let config = written(&ctx);
        assert!(config.servers.is_empty());
        assert!(config.default_server_name.is_empty());
        assert!(server_a.server_token().is_err());
        assert!(
            server_a
                .get_cluster_token("ns".to_string(), "c".to_string())
                .is_err()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&ctx.config_path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn clear_one_server_removes_it_and_resets_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_with_two_servers(dir.path(), "https://clear-one.example");
        let server_a = ctx.config.servers["clear-one-example"].clone();

        ctx.handle_clear_config(false, Some(&"https://clear-one.example".to_string()))
            .unwrap();

        let config = written(&ctx);
        assert_eq!(config.servers.len(), 1);
        assert!(config.servers.contains_key("clear-b-example"));
        assert!(config.default_server_name.is_empty());
        assert!(server_a.server_token().is_err());
    }

    #[test]
    fn clear_another_server_keeps_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_with_two_servers(dir.path(), "https://clear-keep.example");

        ctx.handle_clear_config(false, Some(&URL_B.to_string()))
            .unwrap();

        let config = written(&ctx);
        assert_eq!(config.default_server_name, "clear-keep-example");
        assert!(!config.servers.contains_key("clear-b-example"));
    }

    #[test]
    fn clear_rejects_missing_flags_and_unknown_servers() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_with_two_servers(dir.path(), "https://clear-reject.example");

        assert!(matches!(
            ctx.handle_clear_config(false, None),
            Err(ProxyAuthK8sError::InvalidUsage(_))
        ));
        assert!(matches!(
            ctx.handle_clear_config(false, Some(&"https://unknown".to_string())),
            Err(ProxyAuthK8sError::ServerNotFound(name)) if name == "unknown"
        ));
        // Nothing was written.
        assert!(!ctx.config_path.exists());
    }

    #[test]
    fn clear_reports_a_config_write_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_with_two_servers(dir.path(), "https://clear-fail.example");
        ctx.config_path = dir.path().join("missing/config.yaml");

        assert!(matches!(
            ctx.handle_clear_config(false, Some(&URL_B.to_string())),
            Err(ProxyAuthK8sError::KubeconfigWriteError(_))
        ));
        assert!(matches!(
            ctx.handle_clear_config(true, None),
            Err(ProxyAuthK8sError::KubeconfigWriteError(_))
        ));
    }
}
