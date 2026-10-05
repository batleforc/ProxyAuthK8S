use client_api::{
    apis::{api_clusters_api::get_all_visible_cluster, configuration::Configuration},
    models::GetAllVisibleClusterBody,
};
use std::{
    io::{self, Write},
    path::Path,
};
use tracing::{debug, error, info, warn};

pub mod get_token;
mod kubeconfig;
mod sso;

use crate::{
    cli_config::cli_server_config::{CliServerConfig, http_client, load_certificate_authority},
    ctx::CliCtx,
    error::ProxyAuthK8sError,
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
        if token.is_empty() { None } else { Some(token) }
    }

    pub async fn handle_login(
        &mut self,
        cluster_name: Option<String>,
        token: Option<String>,
        certificate_authority: Option<&Path>,
    ) -> Result<(), ProxyAuthK8sError> {
        let certificate_authority_data = match certificate_authority {
            Some(path) => match load_certificate_authority(path) {
                Ok(data) => Some(data),
                Err(e) => {
                    error!("{}", e);
                    return Err(e);
                }
            },
            None => None,
        };
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
            error!(
                "Error: No ProxyAuthK8S server URL provided and no existing configuration found. Please provide a server URL using the --server-url option or login to server first."
            );
            return Err(ProxyAuthK8sError::InvalidUsage(
                "no server URL provided and no existing configuration found".to_string(),
            ));
        }
        if let Some(cluster) = cluster_name {
            self.handle_login_clusters(cluster, token, certificate_authority_data)
                .await
        } else {
            self.handle_login_servers(token, certificate_authority_data)
                .await
        }
    }

    pub async fn handle_login_clusters(
        &mut self,
        cluster: String,
        token: Option<String>,
        certificate_authority_data: Option<String>,
    ) -> Result<(), ProxyAuthK8sError> {
        debug!("Logging in to cluster: {}", cluster);
        if let Some(data) = certificate_authority_data {
            // Saved on the server before any call, so the calls below trust it.
            self.save_certificate_authority(data)?;
        }
        // if server url is provided but not in config, return error
        let server_config = self.cluster_login_server_config()?;
        let server_name = CliServerConfig::url_to_name_from_string(server_config.url.clone());
        let namespace = self.effective_namespace(&server_config.namespace);
        let clusters = Self::fetch_visible_clusters(server_config).await?;
        let is_sso_enabled =
            Self::cluster_sso_enabled(&clusters, &cluster, &namespace, &server_name)?;
        let token =
            Self::resolve_cluster_token(token, is_sso_enabled, server_config, &namespace, &cluster)
                .await?;

        if let Some(tok) = token {
            info!("Using token for cluster authentication.");
            // The token is stored without being checked against the cluster: a
            // round-trip to `/api?timeout=32s` (what kubectl uses) would catch a
            // bad token here instead of on first use. Tracked on the roadmap.
            self.store_cluster_login(&server_name, &namespace, &cluster, tok)
        } else {
            error!("No token provided. Cluster login requires a token.");
            Err(ProxyAuthK8sError::InvalidUsage(
                "cluster login requires a token".to_string(),
            ))
        }
    }

    /// `--server-url`, or `None` when it was not given (use the default server).
    fn explicit_server_url(&self) -> Option<String> {
        if self.server_url.is_empty() {
            None
        } else {
            Some(self.server_url.clone())
        }
    }

    /// Config name of the server targeted by a cluster login: the one derived
    /// from `--server-url`, else the configured default.
    fn cluster_login_server_name(&self) -> String {
        if self.server_url.is_empty() {
            self.config.default_server_name.clone()
        } else {
            CliServerConfig::url_to_name_from_string(self.server_url.clone())
        }
    }

    /// Record `data` as the CA of the targeted server, which must already be
    /// configured.
    fn save_certificate_authority(&mut self, data: String) -> Result<(), ProxyAuthK8sError> {
        let server_name = self.cluster_login_server_name();
        match self.config.servers.get_mut(&server_name) {
            Some(server) => {
                server.certificate_authority_data = Some(data);
                Ok(())
            }
            None => {
                error!(
                    "Server '{}' not found in configuration, please login to server before login to cluster.",
                    server_name
                );
                Err(ProxyAuthK8sError::ServerNotFound(server_name))
            }
        }
    }

    /// The configured server a cluster login goes through.
    fn cluster_login_server_config(&self) -> Result<&CliServerConfig, ProxyAuthK8sError> {
        match self
            .config
            .get_server_config_by_url(self.explicit_server_url())
        {
            Ok(config) => Ok(config),
            Err(e) => {
                error!(
                    "Error retrieving server configuration, please login to server before login to cluster: {}",
                    e
                );
                Err(e.into())
            }
        }
    }

    /// `--namespace`, else the server's default namespace.
    fn effective_namespace(&self, server_namespace: &str) -> String {
        if self.namespace.is_empty() {
            server_namespace.to_string()
        } else {
            self.namespace.clone()
        }
    }

    /// The clusters the stored server token can see, logging why on failure.
    async fn fetch_visible_clusters(
        server_config: &CliServerConfig,
    ) -> Result<GetAllVisibleClusterBody, ProxyAuthK8sError> {
        match server_config.clusters_from_remote().await {
            Ok(clusters) => {
                // Full cluster topology (names, namespaces, SSO flags) should not
                // land in default-level logs.
                debug!(
                    count = clusters.clusters.len(),
                    "Successfully retrieved clusters"
                );
                Ok(clusters)
            }
            Err(e) => {
                error!("Failed to retrieve clusters, : {}", e);
                match &e {
                    ProxyAuthK8sError::Unauthenticated(_) => {
                        // The user is told to re-login rather than being pushed
                        // through the flow: doing it here would need the login
                        // command to be re-entrant, which it is not yet.
                        error!(
                            "Authentication failed: Invalid server token, please re-login to the server."
                        );
                    }
                    ProxyAuthK8sError::RemoteServerError(_) => {
                        error!("Server error occurred while retrieving clusters.");
                    }
                    _ => {
                        error!("An unknown error occurred while retrieving clusters.");
                    }
                }
                Err(e)
            }
        }
    }

    /// Whether `namespace/cluster` has SSO enabled, or `ClusterNotFound` when
    /// the server does not expose it.
    fn cluster_sso_enabled(
        clusters: &GetAllVisibleClusterBody,
        cluster: &str,
        namespace: &str,
        server_name: &str,
    ) -> Result<bool, ProxyAuthK8sError> {
        if let Some(cluster_config) = clusters
            .clusters
            .iter()
            .find(|c| c.name == cluster && c.namespace == namespace)
        {
            Ok(cluster_config.sso_enabled)
        } else {
            error!("Cluster {} not found on server.", cluster);
            Err(ProxyAuthK8sError::ClusterNotFound {
                server: server_name.to_string(),
                cluster: cluster.to_string(),
            })
        }
    }

    /// The cluster token: the given one, else an SSO browser login when the
    /// cluster supports it, else an interactive prompt.
    async fn resolve_cluster_token(
        token: Option<String>,
        is_sso_enabled: bool,
        server_config: &CliServerConfig,
        namespace: &str,
        cluster: &str,
    ) -> Result<Option<String>, ProxyAuthK8sError> {
        match token {
            Some(token) => Ok(Some(token)),
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
                match Self::sso_cluster_login(&base_config, namespace, cluster).await {
                    // A cluster login stores the id_token (see the front's
                    // ClusterCallbackView).
                    Ok(id_token) => Ok(Some(id_token)),
                    Err(e) => {
                        error!("{}", e);
                        Err(e)
                    }
                }
            }
            None => Ok(Self::prompt_for_token(
                "Cluster token not provided. Enter cluster token: ",
            )),
        }
    }

    /// Persist a cluster login: token in the keyring, CLI config file, and a
    /// kubeconfig context pointing kubectl at the cluster.
    fn store_cluster_login(
        &mut self,
        server_name: &str,
        namespace: &str,
        cluster: &str,
        tok: String,
    ) -> Result<(), ProxyAuthK8sError> {
        let Some(server) = self.config.servers.get_mut(server_name) else {
            // The server config was resolved just above; if it is gone now the
            // config is inconsistent — fail cleanly rather than panicking.
            error!(
                "Server '{}' is no longer present in the configuration; aborting.",
                server_name
            );
            return Err(ProxyAuthK8sError::ServerNotFound(server_name.to_string()));
        };
        // Persist the token to the keyring first; if that fails there is no
        // usable credential, so abort instead of reporting a false success.
        let server_url = server.url.clone();
        let certificate_authority_data = server.certificate_authority_data.clone();
        if let Err(e) = server.set_cluster_token(namespace.to_string(), cluster.to_string(), tok) {
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
        // Point kubectl at the cluster: the token alone is useless without a
        // context whose exec plugin hands it to kubectl.
        let names = match self.edit_kubeconfig(|kubeconfig| {
            kubeconfig::upsert_proxy_context(
                kubeconfig,
                &server_url,
                namespace,
                cluster,
                certificate_authority_data.as_deref(),
            )
        }) {
            Ok(names) => names,
            Err(e) => {
                error!(
                    "Token stored, but failed to write the kubeconfig at {}: {}",
                    self.kubeconfig_path.to_string_lossy(),
                    e
                );
                return Err(e);
            }
        };
        info!("Login to cluster {} successful.", cluster);
        info!(
            "Kubeconfig context '{}' written to {} and set as current context.",
            names.context,
            self.kubeconfig_path.to_string_lossy()
        );
        Ok(())
    }

    pub async fn handle_login_servers(
        &mut self,
        token: Option<String>,
        certificate_authority_data: Option<String>,
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
        let (server_url, server_name) = self.resolve_login_server()?;
        let certificate_authority_data =
            self.server_login_certificate_authority(certificate_authority_data, &server_name);
        let clusters = Self::fetch_visible_clusters_with_token(
            &tok,
            &server_url,
            certificate_authority_data.as_deref(),
        )
        .await?;
        debug!(
            count = clusters.clusters.len(),
            "Successfully retrieved clusters"
        );
        let server_config =
            self.record_server_login(server_name, server_url, certificate_authority_data);

        // Both the config write and the keyring store must succeed for the login
        // to have produced a usable, persisted credential.
        if let Err(e) = self.config.write_to_file(self.config_path.clone()) {
            error!("Failed to update config file: {}", e);
            return Err(e);
        }
        info!("Config file updated successfully.");
        if let Err(e) = server_config.set_server_token(tok) {
            error!("Failed to save token to keyring: {}", e);
            return Err(e);
        }
        info!("Token saved to keyring successfully.");
        Ok(())
    }

    /// Resolve the target server (url + name) once: either the explicit
    /// --server-url, or the configured default. Both the discovery call and
    /// the post-discovery config update reuse it.
    fn resolve_login_server(&self) -> Result<(String, String), ProxyAuthK8sError> {
        if self.server_url.is_empty() {
            let Some(def_server) = self.config.servers.get(&self.config.default_server_name) else {
                error!(
                    "Default server '{}' not found in configuration. Please login to a server first.",
                    self.config.default_server_name
                );
                return Err(ProxyAuthK8sError::ServerNotFound(
                    self.config.default_server_name.clone(),
                ));
            };
            Ok((
                def_server.url.clone(),
                self.config.default_server_name.clone(),
            ))
        } else {
            Ok((
                self.server_url.clone(),
                CliServerConfig::url_to_name_from_string(self.server_url.clone()),
            ))
        }
    }

    /// A CA given now wins; otherwise keep trusting the one saved by a
    /// previous login to this server.
    fn server_login_certificate_authority(
        &self,
        given: Option<String>,
        server_name: &str,
    ) -> Option<String> {
        given.or_else(|| {
            self.config
                .servers
                .get(server_name)
                .and_then(|server| server.certificate_authority_data.clone())
        })
    }

    /// Validate `token` against the server by listing its visible clusters.
    async fn fetch_visible_clusters_with_token(
        token: &str,
        server_url: &str,
        certificate_authority_data: Option<&str>,
    ) -> Result<GetAllVisibleClusterBody, ProxyAuthK8sError> {
        let client = match http_client(certificate_authority_data) {
            Ok(client) => client,
            Err(e) => {
                error!("{}", e);
                return Err(e);
            }
        };
        get_all_visible_cluster(&Configuration {
            bearer_access_token: Some(token.to_string()),
            base_path: server_url.to_string(),
            client,
            ..Default::default()
        })
        .await
        .map_err(|e| {
            // Reuse the client_api -> ProxyAuthK8sError From boundary rather
            // than re-matching the raw variants inline.
            let e = ProxyAuthK8sError::from(e);
            error!("Failed to retrieve clusters: {}", e);
            e
        })
    }

    /// Add (or update) the server in the in-memory config, making it the
    /// default when none is set yet. Returns the resulting server config.
    fn record_server_login(
        &mut self,
        server_name: String,
        server_url: String,
        certificate_authority_data: Option<String>,
    ) -> CliServerConfig {
        let server_name_clone = server_name.clone();
        let server_config = self
            .config
            .get_or_insert_server_config(server_name, server_url);
        server_config.certificate_authority_data = certificate_authority_data;
        let server_config_clone = server_config.clone();

        if self.config.default_server_name.is_empty() {
            self.config.default_server_name = server_name_clone;
        }
        server_config_clone
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use client_api::models::VisibleCluster;

    const URL: &str = "https://proxy.example.com";
    const NAME: &str = "proxy-example-com";

    /// A context with one configured server (`URL`), set as the default.
    fn ctx_with_server() -> CliCtx {
        let mut ctx = CliCtx::for_test();
        ctx.config
            .get_or_insert_server_config(NAME.to_string(), URL.to_string());
        ctx.config.default_server_name = NAME.to_string();
        ctx
    }

    #[test]
    fn explicit_server_url_is_none_when_not_given() {
        let mut ctx = CliCtx::for_test();
        assert_eq!(ctx.explicit_server_url(), None);
        ctx.server_url = URL.to_string();
        assert_eq!(ctx.explicit_server_url(), Some(URL.to_string()));
    }

    #[test]
    fn cluster_login_targets_the_given_server_else_the_default() {
        let mut ctx = CliCtx::for_test();
        ctx.config.default_server_name = "default-srv".to_string();
        assert_eq!(ctx.cluster_login_server_name(), "default-srv");
        ctx.server_url = "https://other:8443".to_string();
        assert_eq!(ctx.cluster_login_server_name(), "other-8443");
    }

    #[test]
    fn certificate_authority_is_saved_on_a_known_server_only() {
        let mut ctx = ctx_with_server();
        ctx.save_certificate_authority("Q0E=".to_string()).unwrap();
        assert_eq!(
            ctx.config.servers[NAME]
                .certificate_authority_data
                .as_deref(),
            Some("Q0E=")
        );

        ctx.server_url = "https://unknown".to_string();
        assert!(matches!(
            ctx.save_certificate_authority("Q0E=".to_string()),
            Err(ProxyAuthK8sError::ServerNotFound(name)) if name == "unknown"
        ));
    }

    #[test]
    fn cluster_login_server_config_requires_a_configured_server() {
        let mut ctx = ctx_with_server();
        assert_eq!(ctx.cluster_login_server_config().unwrap().url, URL);
        ctx.server_url = URL.to_string();
        assert_eq!(ctx.cluster_login_server_config().unwrap().url, URL);
        ctx.server_url = "https://unknown".to_string();
        assert!(matches!(
            ctx.cluster_login_server_config(),
            Err(ProxyAuthK8sError::ServerNotFound(_))
        ));
    }

    #[test]
    fn namespace_flag_overrides_the_server_default() {
        let mut ctx = CliCtx::for_test();
        assert_eq!(ctx.effective_namespace("default"), "default");
        ctx.namespace = "team-a".to_string();
        assert_eq!(ctx.effective_namespace("default"), "team-a");
    }

    #[test]
    fn sso_flag_is_read_from_the_matching_cluster() {
        let clusters = GetAllVisibleClusterBody::new(vec![
            VisibleCluster::new(true, "prod".into(), "team-a".into(), true),
            VisibleCluster::new(true, "prod".into(), "team-b".into(), false),
        ]);
        assert!(CliCtx::cluster_sso_enabled(&clusters, "prod", "team-a", NAME).unwrap());
        assert!(!CliCtx::cluster_sso_enabled(&clusters, "prod", "team-b", NAME).unwrap());
        assert!(matches!(
            CliCtx::cluster_sso_enabled(&clusters, "prod", "team-c", NAME),
            Err(ProxyAuthK8sError::ClusterNotFound { server, cluster })
                if server == NAME && cluster == "prod"
        ));
    }

    #[tokio::test]
    async fn a_given_cluster_token_is_used_as_is() {
        let server = CliServerConfig::new(URL.to_string());
        // SSO enabled or not, an explicit token short-circuits any login flow.
        for sso in [true, false] {
            let token = CliCtx::resolve_cluster_token(
                Some("tok".to_string()),
                sso,
                &server,
                "team-a",
                "prod",
            )
            .await
            .unwrap();
            assert_eq!(token.as_deref(), Some("tok"));
        }
    }

    #[test]
    fn server_login_uses_the_given_url_else_the_default_server() {
        let mut ctx = ctx_with_server();
        assert_eq!(
            ctx.resolve_login_server().unwrap(),
            (URL.to_string(), NAME.to_string())
        );
        ctx.server_url = "https://new.example:8443".to_string();
        assert_eq!(
            ctx.resolve_login_server().unwrap(),
            (
                "https://new.example:8443".to_string(),
                "new-example-8443".to_string()
            )
        );

        let mut empty = CliCtx::for_test();
        empty.config.default_server_name = "gone".to_string();
        assert!(matches!(
            empty.resolve_login_server(),
            Err(ProxyAuthK8sError::ServerNotFound(name)) if name == "gone"
        ));
    }

    #[test]
    fn server_login_ca_prefers_the_given_one_then_the_saved_one() {
        let mut ctx = ctx_with_server();
        assert_eq!(ctx.server_login_certificate_authority(None, NAME), None);
        ctx.config
            .servers
            .get_mut(NAME)
            .unwrap()
            .certificate_authority_data = Some("saved".to_string());
        assert_eq!(
            ctx.server_login_certificate_authority(None, NAME)
                .as_deref(),
            Some("saved")
        );
        assert_eq!(
            ctx.server_login_certificate_authority(Some("given".to_string()), NAME)
                .as_deref(),
            Some("given")
        );
        assert_eq!(ctx.server_login_certificate_authority(None, "other"), None);
    }

    #[test]
    fn recording_a_server_login_sets_the_default_only_once() {
        let mut ctx = CliCtx::for_test();
        let first = ctx.record_server_login(NAME.to_string(), URL.to_string(), Some("ca".into()));
        assert_eq!(first.url, URL);
        assert_eq!(first.certificate_authority_data.as_deref(), Some("ca"));
        assert_eq!(ctx.config.default_server_name, NAME);

        let second =
            ctx.record_server_login("other".to_string(), "https://other".to_string(), None);
        assert_eq!(second.url, "https://other");
        assert_eq!(ctx.config.default_server_name, NAME);
        assert_eq!(ctx.config.servers.len(), 2);

        // Re-login keeps the stored URL but replaces the CA.
        let again = ctx.record_server_login(NAME.to_string(), "https://ignored".to_string(), None);
        assert_eq!(again.url, URL);
        assert_eq!(again.certificate_authority_data, None);
    }
}
