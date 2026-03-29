use crate::ctx::CliCtx;
use tracing::{debug, error};
//https://kubernetes.io/docs/reference/access-authn-authz/authentication/#input-and-output-formats
impl CliCtx {
    pub async fn handle_get_token(&mut self, cluster_name: Option<String>) {
        debug!("Handling get token for cluster: {:?}", cluster_name);

        // if server_url is not provided and none exist in config, return error
        if self.server_url.is_empty() && self.config.default_server_name.is_empty() {
            error!("Error: No ProxyAuthK8S server URL provided and no existing configuration found. Please provide a server URL using the --server-url option or login to server first.");
            return;
        }

        // KUBERNETES_EXEC_INFO is always set by kubectl when invoking an exec plugin
        let exec_ctx = match std::env::var("KUBERNETES_EXEC_INFO") {
            Ok(info) => info,
            Err(_) => {
                error!(
                    "KUBERNETES_EXEC_INFO environment variable is not set. Cannot retrieve ctx."
                );
                return;
            }
        };
        debug!("KUBERNETES_EXEC_INFO: {}", exec_ctx);

        let exec_info: serde_json::Value = match serde_json::from_str(&exec_ctx) {
            Ok(info) => info,
            Err(e) => {
                error!("Failed to parse KUBERNETES_EXEC_INFO as JSON: {}", e);
                return;
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
            debug!("No cluster name provided, extracting from KUBERNETES_EXEC_INFO spec.cluster.server");
            let cluster_server_url = match exec_info
                .get("spec")
                .and_then(|s| s.get("cluster"))
                .and_then(|s| s.get("server"))
                .and_then(|v| v.as_str())
            {
                Some(url) => url,
                None => {
                    error!("Cluster name not provided and spec.cluster.server not found in KUBERNETES_EXEC_INFO. Please provide the cluster name as argument.");
                    return;
                }
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
                    return;
                }
            }
        };

        // Resolve server config (via --server-url arg or default)
        let server_config =
            match self
                .config
                .get_server_config_by_url(if self.server_url.is_empty() {
                    None
                } else {
                    Some(self.server_url.clone())
                }) {
                Ok(config) => config,
                Err(e) => {
                    error!(
                    "Error retrieving server configuration: {}. Please login to the server first.",
                    e
                );
                    return;
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

        // Retrieve the cluster token from keyring
        let token = match server_config.get_cluster_token(namespace.clone(), cluster_name.clone()) {
            Ok(token) => token,
            Err(e) => {
                error!(
                    "Failed to retrieve token for cluster '{}/{}': {}. Please login using 'login --cluster-name {}'.",
                    namespace, cluster_name, e, cluster_name
                );
                return;
            }
        };

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
            Ok(output) => println!("{}", output),
            Err(e) => {
                error!("Failed to serialize ExecCredential response: {}", e);
            }
        }
    }
}
