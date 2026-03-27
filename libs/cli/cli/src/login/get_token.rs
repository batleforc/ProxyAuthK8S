use crate::ctx::CliCtx;
use tracing::{debug, error, info};
//https://kubernetes.io/docs/reference/access-authn-authz/authentication/#input-and-output-formats
impl CliCtx {
    pub async fn handle_get_token(&mut self, cluster_name: Option<String>) {
        debug!("Handling get token for cluster: {:?}", cluster_name);
        // if server_url is not provided and none exist in config, return error
        if self.server_url.is_empty() && self.config.default_server_name.is_empty() {
            error!("Error: No ProxyAuthK8S server URL provided and no existing configuration found. Please provide a server URL using the --server-url option or login to server first.");
            return;
        }
        let exec_ctx = match std::env::var("KUBERNETES_EXEC_INFO") {
            Ok(info) => info,
            Err(_) => {
                error!(
                    "KUBERNETES_EXEC_INFO environment variable is not set. Cannot retrieve ctx."
                );
                return;
            }
        };
        info!("truc");
        info!("KUBERNETES_EXEC_INFO: {}", exec_ctx);
        // parse to json
        let exec_info: serde_json::Value = match serde_json::from_str(&exec_ctx) {
            Ok(info) => info,
            Err(e) => {
                error!("Failed to parse KUBERNETES_EXEC_INFO as JSON: {}", e);
                return;
            }
        };

        let api_version = exec_info
            .get("apiVersion")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        debug!("API Version from exec info: {}", api_version);

        let _server = exec_info
            .get("spec")
            .and_then(|s| s.get("cluster"))
            .and_then(|s| s.get("server"))
            .and_then(|v| v.as_str());

        // determine cluster name, server_url and namespace. Either from the provided cluster_name or from the exec_info
        let (_cluster_name, _server_url, _namespace) = if let Some(cluster_name) = cluster_name {
            debug!("Cluster name provided as argument: {}", cluster_name);
            let server_url = exec_info
                .get("spec")
                .and_then(|s| s.get("cluster"))
                .and_then(|s| s.get("server"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let namespace = exec_info
                .get("spec")
                .and_then(|s| s.get("cluster"))
                .and_then(|s| s.get("namespace"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            (cluster_name, server_url.to_string(), namespace.to_string())
        } else {
            debug!("No cluster name provided as argument, attempting to extract from exec info");
            panic!("Cluster name is required to get token. Please provide a cluster name using the --cluster-name option.");
        };
    }
}
