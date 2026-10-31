use crate::cli_config::cli_server_config::CliServerConfig;
use crate::ctx::CliCtx;
use crate::error::ProxyAuthK8sError;
use tracing::{debug, error};

//https://kubernetes.io/docs/reference/access-authn-authz/authentication/#input-and-output-formats
impl CliCtx {
    /// The token kubectl sends to `namespace/cluster`: the server token,
    /// renewed when about to expire, for a cluster recorded in token exchange
    /// by `login` (RFC 0010); else the cluster token from the keyring.
    async fn cluster_exec_token(
        server_config: &CliServerConfig,
        namespace: &str,
        cluster_name: &str,
    ) -> Result<String, ProxyAuthK8sError> {
        let token_exchange = server_config
            .get_clusters_from_ns_name(Some(namespace.to_string()), cluster_name.to_string())
            .is_some_and(|cluster| cluster.token_exchange);
        if token_exchange {
            return server_config.fresh_server_token().await.inspect_err(|e| {
                error!(
                    "Failed to retrieve the server token for cluster '{}/{}': {}",
                    namespace, cluster_name, e
                );
            });
        }
        server_config
            .get_cluster_token(namespace.to_string(), cluster_name.to_string())
            .inspect_err(|e| {
                error!(
                    "Failed to retrieve token for cluster '{}/{}': {}. Please login using 'login --cluster-name {}'.",
                    namespace, cluster_name, e, cluster_name
                );
            })
    }

    /// Returns `Err` on any failure so the caller can propagate a non-zero
    /// exit code. `kubectl` treats a zero exit from an exec-credential plugin as
    /// "credentials produced"; exiting 0 on failure would make it retry/fail with
    /// a confusing error instead of surfacing our message.
    pub async fn handle_get_token(
        &mut self,
        cluster_name: Option<String>,
    ) -> Result<(), ProxyAuthK8sError> {
        debug!("Handling get token for cluster: {:?}", cluster_name);

        // if server_url is not provided and none exist in config, return error
        if self.server_url.is_empty() && self.config.default_server_name.is_empty() {
            error!(
                "Error: No ProxyAuthK8S server URL provided and no existing configuration found. Please provide a server URL using the --server-url option or login to server first."
            );
            return Err(ProxyAuthK8sError::InvalidUsage(
                "No ProxyAuthK8S server URL provided and no existing configuration found"
                    .to_owned(),
            ));
        }

        // KUBERNETES_EXEC_INFO is always set by kubectl when invoking an exec plugin
        let exec_ctx = if let Ok(info) = std::env::var("KUBERNETES_EXEC_INFO") {
            info
        } else {
            error!("KUBERNETES_EXEC_INFO environment variable is not set. Cannot retrieve ctx.");
            return Err(ProxyAuthK8sError::ExecCredential(
                "KUBERNETES_EXEC_INFO environment variable is not set".to_owned(),
            ));
        };
        debug!("KUBERNETES_EXEC_INFO: {}", exec_ctx);

        let exec_info: serde_json::Value = match serde_json::from_str(&exec_ctx) {
            Ok(info) => info,
            Err(e) => {
                error!("Failed to parse KUBERNETES_EXEC_INFO as JSON: {}", e);
                return Err(ProxyAuthK8sError::ExecCredential(format!(
                    "failed to parse KUBERNETES_EXEC_INFO as JSON: {e}"
                )));
            }
        };

        // Use the apiVersion from exec info for the ExecCredential response
        let api_version = exec_info
            .get("apiVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("client.authentication.k8s.io/v1beta1");
        debug!("API Version from exec info: {}", api_version);

        // Determine cluster_name and a candidate namespace from exec info:
        // - If cluster_name is provided as CLI arg, use it directly.
        // - Otherwise, extract both from spec.cluster.server in KUBERNETES_EXEC_INFO
        //   (URL format: https://<host>/clusters/<namespace>/<cluster-name>)
        let (cluster_name, exec_namespace) = if let Some(name) = cluster_name {
            debug!("Cluster name provided as argument: {}", name);
            let exec_ns = exec_info
                .get("spec")
                .and_then(|s| s.get("cluster"))
                .and_then(|s| s.get("server"))
                .and_then(|v| v.as_str())
                .and_then(|url| {
                    crate::cli_config::CliConfig::proxy_url_to_tuple(url)
                        .ok()
                        .map(|info| info.namespace)
                });
            (name, exec_ns)
        } else {
            debug!(
                "No cluster name provided, extracting from KUBERNETES_EXEC_INFO spec.cluster.server"
            );
            let cluster_server_url = if let Some(url) = exec_info
                .get("spec")
                .and_then(|s| s.get("cluster"))
                .and_then(|s| s.get("server"))
                .and_then(|v| v.as_str())
            {
                url
            } else {
                error!(
                    "Cluster name not provided and spec.cluster.server not found in KUBERNETES_EXEC_INFO. Please provide the cluster name as argument."
                );
                return Err(ProxyAuthK8sError::ExecCredential(
                    "cluster name not provided and spec.cluster.server not found in KUBERNETES_EXEC_INFO"
                        .to_owned(),
                ));
            };
            match crate::cli_config::CliConfig::proxy_url_to_tuple(cluster_server_url) {
                Ok(url_info) => {
                    debug!(
                        "Extracted cluster '{}' in namespace '{}' from server URL",
                        url_info.cluster_name, url_info.namespace
                    );
                    (url_info.cluster_name, Some(url_info.namespace))
                }
                Err(e) => {
                    error!(
                        "Failed to extract cluster name from server URL '{}': {}. Please provide the cluster name as argument.",
                        cluster_server_url, e
                    );
                    return Err(e.into());
                }
            }
        };

        // Resolve server config (via --server-url arg or default)
        let server_config = match self.config.get_server_config_by_url(
            if self.server_url.is_empty() {
                None
            } else {
                Some(self.server_url.clone())
            },
        ) {
            Ok(config) => config,
            Err(e) => {
                error!(
                    "Error retrieving server configuration: {}. Please login to the server first.",
                    e
                );
                return Err(e.into());
            }
        };

        // Determine namespace: CLI arg > exec info > server default
        let namespace = if !self.namespace.is_empty() {
            self.namespace.clone()
        } else if let Some(ns) = exec_namespace {
            ns
        } else {
            server_config.namespace.clone()
        };

        debug!(
            "Retrieving token for cluster '{}/{}' from server '{}'",
            namespace, cluster_name, server_config.url
        );

        let token = Self::cluster_exec_token(server_config, &namespace, &cluster_name).await?;

        debug!(
            "Successfully retrieved token for cluster '{}/{}'.",
            namespace, cluster_name
        );
        // get v1 or v1beta1 from api_version string (default to v1beta1 if not found)
        let api_version = if api_version.contains("v1beta1") {
            "client.authentication.k8s.io/v1beta1"
        } else {
            "client.authentication.k8s.io/v1"
        };

        // Output ExecCredential JSON to stdout (kubectl exec plugin protocol)
        let exec_credential = serde_json::json!({
            "apiVersion": api_version,
            "kind": "ExecCredential",
            "status": {
                "token": token
            }
        });

        match serde_json::to_string(&exec_credential) {
            Ok(output) => {
                println!("{output}");
                Ok(())
            }
            Err(e) => {
                error!("Failed to serialize ExecCredential response: {}", e);
                Err(ProxyAuthK8sError::ExecCredential(format!(
                    "failed to serialize ExecCredential response: {e}"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_config::cli_cluster_config::CliClusterConfig;

    #[tokio::test]
    async fn an_exchange_cluster_gets_the_server_token_never_a_cluster_token() {
        let mut server = CliServerConfig::new("http://127.0.0.1:9/get-token-exchange".to_string());
        server.set_server_token("srv-tok".to_string()).unwrap();
        // A stale cluster token is there, and must not be the one handed out.
        server
            .set_cluster_token("team-a".to_string(), "x".to_string(), "cl-tok".to_string())
            .unwrap();
        server
            .clusters
            .insert("team-a/x".to_string(), CliClusterConfig::token_exchange());

        let token = CliCtx::cluster_exec_token(&server, "team-a", "x")
            .await
            .unwrap();
        assert_eq!(token, "srv-tok");
    }

    #[tokio::test]
    async fn a_cluster_without_the_flag_keeps_its_own_token() {
        let mut server = CliServerConfig::new("http://127.0.0.1:9/get-token-plain".to_string());
        server.set_server_token("srv-tok".to_string()).unwrap();
        server
            .set_cluster_token("team-a".to_string(), "x".to_string(), "cl-tok".to_string())
            .unwrap();

        let token = CliCtx::cluster_exec_token(&server, "team-a", "x")
            .await
            .unwrap();
        assert_eq!(token, "cl-tok");
        assert!(matches!(
            CliCtx::cluster_exec_token(&server, "team-a", "missing").await,
            Err(ProxyAuthK8sError::KeyringReadError(_))
        ));
    }
}
