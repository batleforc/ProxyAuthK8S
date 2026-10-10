use tracing::{error, info, warn};

use crate::{
    cli_config::cli_server_config::CliServerConfig, ctx::CliCtx, error::ProxyAuthK8sError,
};

impl CliCtx {
    pub fn handle_set_def(
        &mut self,
        server_url: Option<&String>,
        namespace: Option<&String>,
        default_server: Option<&String>,
    ) -> Result<(), ProxyAuthK8sError> {
        if let Some(default_server) = default_server {
            let default_server_name =
                CliServerConfig::url_to_name_from_string(default_server.clone());
            // Check if the server exists in the config
            if !self.config.servers.contains_key(&default_server_name) {
                warn!(
                    "Server name '{}' not found in config. Please login to add it first.",
                    default_server
                );
                return Err(ProxyAuthK8sError::ServerNotFound(default_server.clone()));
            }
            self.config.default_server_name = default_server_name;
            return match self.config.write_to_file(self.config_path.clone()) {
                Ok(_) => {
                    info!("Default server set successfully to: {}", default_server);
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to set default server: {}", e);
                    Err(e)
                }
            };
        }

        let (Some(server_url), Some(namespace)) = (server_url, namespace) else {
            warn!("Both server_url and namespace must be provided together.");
            return Err(ProxyAuthK8sError::InvalidUsage(
                "both server_url and namespace must be provided together".to_string(),
            ));
        };
        let server_name = CliServerConfig::url_to_name_from_string(server_url.clone());
        if let Some(server_config) = self.config.servers.get_mut(&server_name) {
            server_config.namespace = namespace.clone();
            match self.config.write_to_file(self.config_path.clone()) {
                Ok(_) => {
                    info!(
                        "Default namespace for server {} set successfully to: {}",
                        server_url, namespace
                    );
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to set default namespace: {}", e);
                    Err(e)
                }
            }
        } else {
            warn!(
                "Server URL not found in config: {}, please login to add it first.",
                server_url
            );
            Err(ProxyAuthK8sError::ServerNotFound(server_name))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_config::CliConfig;

    const URL_A: &str = "https://setdef-a.example";
    const URL_B: &str = "https://setdef-b.example:8443";

    fn ctx_with_two_servers(dir: &std::path::Path) -> CliCtx {
        let mut ctx = CliCtx::for_test_in(dir);
        for url in [URL_A, URL_B] {
            ctx.config.get_or_insert_server_config(
                CliServerConfig::url_to_name_from_string(url.to_string()),
                url.to_string(),
            );
        }
        ctx.config.default_server_name = "setdef-a-example".to_string();
        ctx
    }

    fn written(ctx: &CliCtx) -> CliConfig {
        CliConfig::read_from_file(ctx.config_path.clone()).unwrap()
    }

    #[test]
    fn default_server_is_switched_and_persisted_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_with_two_servers(dir.path());

        ctx.handle_set_def(None, None, Some(&URL_B.to_string()))
            .unwrap();

        assert_eq!(written(&ctx).default_server_name, "setdef-b-example-8443");
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
    fn default_server_must_be_known() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_with_two_servers(dir.path());

        assert!(matches!(
            ctx.handle_set_def(None, None, Some(&"https://unknown".to_string())),
            Err(ProxyAuthK8sError::ServerNotFound(url)) if url == "https://unknown"
        ));
        assert_eq!(ctx.config.default_server_name, "setdef-a-example");
        assert!(!ctx.config_path.exists());
    }

    #[test]
    fn default_namespace_is_set_on_the_given_server() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_with_two_servers(dir.path());

        ctx.handle_set_def(Some(&URL_B.to_string()), Some(&"team-b".to_string()), None)
            .unwrap();

        let config = written(&ctx);
        assert_eq!(config.servers["setdef-b-example-8443"].namespace, "team-b");
        // The other server and the default are untouched.
        assert_eq!(config.servers["setdef-a-example"].namespace, "default");
        assert_eq!(config.default_server_name, "setdef-a-example");
    }

    #[test]
    fn namespace_needs_both_flags_and_a_known_server() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_with_two_servers(dir.path());

        assert!(matches!(
            ctx.handle_set_def(Some(&URL_A.to_string()), None, None),
            Err(ProxyAuthK8sError::InvalidUsage(_))
        ));
        assert!(matches!(
            ctx.handle_set_def(None, Some(&"ns".to_string()), None),
            Err(ProxyAuthK8sError::InvalidUsage(_))
        ));
        assert!(matches!(
            ctx.handle_set_def(
                Some(&"https://unknown".to_string()),
                Some(&"ns".to_string()),
                None
            ),
            Err(ProxyAuthK8sError::ServerNotFound(name)) if name == "unknown"
        ));
        assert!(!ctx.config_path.exists());
    }

    #[test]
    fn set_def_reports_a_config_write_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx_with_two_servers(dir.path());
        ctx.config_path = dir.path().join("missing/config.yaml");

        assert!(matches!(
            ctx.handle_set_def(None, None, Some(&URL_B.to_string())),
            Err(ProxyAuthK8sError::KubeconfigWriteError(_))
        ));
        assert!(matches!(
            ctx.handle_set_def(Some(&URL_A.to_string()), Some(&"ns".to_string()), None),
            Err(ProxyAuthK8sError::KubeconfigWriteError(_))
        ));
    }
}
