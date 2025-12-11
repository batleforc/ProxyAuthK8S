use tracing::info;

use crate::ctx::CliCtx;

impl CliCtx {
    pub async fn handle_login(&mut self, cluster_name: Option<String>, token: Option<String>) {
        if let Some(cluster) = cluster_name {
            info!("Logging in to cluster: {}", cluster);
            // Here you would implement the logic to actually log in to the specified cluster
            if let Some(tok) = token {
                info!("Using provided token for authentication. {}", tok);
                // Use the token for authentication
            } else {
                info!("No token provided, proceeding without authentication token.");
                // Proceed without a token
            }
        } else {
            info!("Logging in to ProxyAuthK8S server.");
            // Here you would implement the logic to log in to the ProxyAuthK8S server
            if let Some(tok) = token {
                info!("Using provided token for authentication. {}", tok);
                // Use the token for authentication
            } else {
                info!("No token provided, proceeding without authentication token.");
                // Proceed without a token
            }
        }
    }
}
