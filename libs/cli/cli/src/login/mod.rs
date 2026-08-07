use client_api::apis::{api_clusters_api::get_all_visible_cluster, configuration::Configuration};
use std::io::{self, Write};
use tracing::{debug, error, info, warn};

pub mod get_token;
mod sso;

use crate::{
    cli_config::cli_server_config::CliServerConfig, ctx::CliCtx, error::ProxyAuthK8sError,
};

impl CliCtx {
    fn prompt_for_token(prompt: &str) -> Option<String> {
        print!("{prompt}");
        if io::stdout().flush().is_err() {
            return None;
        }
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return None;
        }
        let token = input.trim().to_string();
        if token.is_empty() {
            None
        } else {
            Some(token)
        }
    }

    pub async fn handle_login(
        &mut self,
        cluster_name: Option<String>,
        token: Option<String>,
    ) -> Result<(), ProxyAuthK8sError> {
        if token.is_some() {
            // A token on the command line lands in `ps`/`/proc/<pid>/cmdline` and
            // the shell history. Prefer the interactive prompt (omit `--token`).
            warn!(
                "passing --token on the command line exposes it to other local users \
                 (ps, shell history); prefer the interactive prompt"
            );
        }
        // if server_url is not provided and none exist in config, return error
        if self.server_url.is_empty() && self.config.default_server_name.is_empty() {
            error!("Error: No ProxyAuthK8S server URL provided and no existing configuration found. Please provide a server URL using the --server-url option or login to server first.");
            return Err(ProxyAuthK8sError::InvalidUsage(
                "no server URL provided and no existing configuration found".to_string(),
            ));
        }
        if let Some(cluster) = cluster_name {
            self.handle_login_clusters(cluster, token).await
        } else {
            self.handle_login_servers(token).await
        }
    }

    pub async fn handle_login_clusters(
        &mut self,
        cluster: String,
        token: Option<String>,
    ) -> Result<(), ProxyAuthK8sError> {
        debug!("Logging in to cluster: {}", cluster);
        // if server url is provided but not in config, return error
        let server_config = match self.config.get_server_config_by_url(
            if self.server_url.is_empty() {
                None
            } else {
                Some(self.server_url.clone())
            },
        ) {
            Ok(config) => config,
            Err(e) => {
                error!("Error retrieving server configuration, please login to server before login to cluster: {}", e);
                return Err(e.into());
            }
        };
        let server_name = CliServerConfig::url_to_name_from_string(server_config.url.clone());
        let namespace = if self.namespace.is_empty() {
            server_config.namespace.clone()
        } else {
            self.namespace.clone()
        };
        let clusters = match server_config.clusters_from_remote().await {
            Ok(clusters) => {
                // Full cluster topology (names, namespaces, SSO flags) should not
                // land in default-level logs.
                debug!(
                    count = clusters.clusters.len(),
                    "Successfully retrieved clusters"
                );
                clusters
            }
            Err(e) => {
                error!("Failed to retrieve clusters, : {}", e);
                match &e {
                    ProxyAuthK8sError::Unauthenticated(_) => {
                        // The user is told to re-login rather than being pushed
                        // through the flow: doing it here would need the login
                        // command to be re-entrant, which it is not yet.
                        error!("Authentication failed: Invalid server token, please re-login to the server.");
                    }
                    ProxyAuthK8sError::RemoteServerError(_) => {
                        error!("Server error occurred while retrieving clusters.");
                    }
                    _ => {
                        error!("An unknown error occurred while retrieving clusters.");
                    }
                }
                return Err(e);
            }
        };

        let target_cluster = clusters
            .clusters
            .iter()
            .find(|c| c.name == cluster && c.namespace == namespace);

        let is_sso_enabled = if let Some(cluster_config) = target_cluster {
            cluster_config.sso_enabled
        } else {
            error!("Cluster {} not found on server.", cluster);
            return Err(ProxyAuthK8sError::ClusterNotFound {
                server: server_name.clone(),
                cluster: cluster.clone(),
            });
        };

        let token = match token {
            Some(token) => Some(token),
            None if is_sso_enabled => {
                info!(
                    "Cluster '{}' has SSO enabled; starting interactive browser login.",
                    cluster
                );
                // `/auth/login` is authenticated: reuse the stored server token.
                let base_config = match server_config.base_configuration() {
                    Ok(config) => config,
                    Err(e) => {
                        error!(
                            "Cannot start SSO login (are you logged in to the server?): {}",
                            e
                        );
                        return Err(e);
                    }
                };
                match Self::sso_cluster_login(&base_config, &namespace, &cluster).await {
                    // A cluster login stores the id_token (see the front's
                    // ClusterCallbackView).
                    Ok(id_token) => Some(id_token),
                    Err(e) => {
                        error!("{}", e);
                        return Err(e);
                    }
                }
            }
            None => Self::prompt_for_token("Cluster token not provided. Enter cluster token: "),
        };

        if let Some(tok) = token {
            info!("Using token for cluster authentication.");
            // The token is stored without being checked against the cluster: a
            // round-trip to `/api?timeout=32s` (what kubectl uses) would catch a
            // bad token here instead of on first use. Tracked on the roadmap.

            let Some(server) = self.config.servers.get_mut(&server_name) else {
                // The server config was resolved just above; if it is gone now the
                // config is inconsistent — fail cleanly rather than panicking.
                error!(
                    "Server '{}' is no longer present in the configuration; aborting.",
                    server_name
                );
                return Err(ProxyAuthK8sError::ServerNotFound(server_name.clone()));
            };
            // Persist the token to the keyring first; if that fails there is no
            // usable credential, so abort instead of reporting a false success.
            if let Err(e) = server.set_cluster_token(namespace, cluster.clone(), tok.clone()) {
                error!("Failed to store cluster token: {}", e);
                return Err(e);
            }
            match self.config.write_to_file(self.config_path.clone()) {
                Ok(_) => info!("Config file updated successfully."),
                Err(e) => {
                    error!("Failed to update config file: {}", e);
                    return Err(e);
                }
            }
            info!("Login to cluster {} successful.", cluster);
            Ok(())
        } else {
            error!("No token provided. Cluster login requires a token.");
            Err(ProxyAuthK8sError::InvalidUsage(
                "cluster login requires a token".to_string(),
            ))
        }
    }

    pub async fn handle_login_servers(
        &mut self,
        token: Option<String>,
    ) -> Result<(), ProxyAuthK8sError> {
        debug!("Logging in to ProxyAuthK8S server.");
        let token = token
            .or_else(|| Self::prompt_for_token("Server token not provided. Enter server token: "));

        let Some(tok) = token else {
            error!("No token provided. Server login requires a token.");
            return Err(ProxyAuthK8sError::InvalidUsage(
                "server login requires a token".to_string(),
            ));
        };
        info!("Using token for server authentication.");
        // Resolve the target server (url + name) once: either the explicit
        // --server-url, or the configured default. Both the discovery call
        // and the post-discovery config update reuse it.
        let (server_url, server_name) = if self.server_url.is_empty() {
            let Some(def_server) = self.config.servers.get(&self.config.default_server_name) else {
                error!(
                    "Default server '{}' not found in configuration. Please login to a server first.",
                    self.config.default_server_name
                );
                return Err(ProxyAuthK8sError::ServerNotFound(
                    self.config.default_server_name.clone(),
                ));
            };
            (
                def_server.url.clone(),
                self.config.default_server_name.clone(),
            )
        } else {
            (
                self.server_url.clone(),
                CliServerConfig::url_to_name_from_string(self.server_url.clone()),
            )
        };
        let output = get_all_visible_cluster(&Configuration {
            bearer_access_token: Some(tok.clone()),
            base_path: server_url.clone(),
            ..Default::default()
        })
        .await;

        let clusters = match output {
            Ok(clusters) => clusters,
            Err(e) => {
                // Reuse the client_api -> ProxyAuthK8sError From boundary rather
                // than re-matching the raw variants inline.
                let e = ProxyAuthK8sError::from(e);
                error!("Failed to retrieve clusters: {}", e);
                return Err(e);
            }
        };
        debug!(
            count = clusters.clusters.len(),
            "Successfully retrieved clusters"
        );
        let server_name_clone = server_name.clone();
        let server_config = self
            .config
            .get_or_insert_server_config(server_name, server_url);
        let server_config_clone = server_config.clone();

        if self.config.default_server_name.is_empty() {
            self.config.default_server_name = server_name_clone;
        }

        // Both the config write and the keyring store must succeed for the login
        // to have produced a usable, persisted credential.
        if let Err(e) = self.config.write_to_file(self.config_path.clone()) {
            error!("Failed to update config file: {}", e);
            return Err(e);
        }
        info!("Config file updated successfully.");
        if let Err(e) = server_config_clone.set_server_token(tok.clone()) {
            error!("Failed to save token to keyring: {}", e);
            return Err(e);
        }
        info!("Token saved to keyring successfully.");
        Ok(())
    }
}
