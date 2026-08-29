use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::{
    ctx::CliCtx,
    error::ProxyAuthK8sError,
    output::{KubeList, TableRow},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetClusterOutput {
    pub name: String,
    pub namespace: String,
    pub enabled: bool,
    pub is_reachable: Option<bool>,
    pub sso_enabled: bool,
}

impl TableRow for GetClusterOutput {
    fn headers() -> Vec<String> {
        vec![
            "NAME".to_string(),
            "NAMESPACE".to_string(),
            "ENABLED".to_string(),
            "REACHABLE".to_string(),
            "SSO".to_string(),
        ]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.name.clone(),
            self.namespace.clone(),
            self.enabled.to_string(),
            self.is_reachable
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            self.sso_enabled.to_string(),
        ]
    }
}

impl CliCtx {
    pub async fn handle_get_clusters(
        &mut self,
        cluster_name: Option<String>,
    ) -> Result<(), ProxyAuthK8sError> {
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
                        "Error retrieving server configuration, please login to server first: {}",
                        e
                    );
                    return Err(e.into());
                }
            };

        let namespace_filter = if self.namespace.is_empty() {
            None
        } else {
            Some(self.namespace.clone())
        };

        match server_config.clusters_from_remote().await {
            Ok(clusters) => {
                let mut outputs: Vec<GetClusterOutput> = clusters
                    .clusters
                    .iter()
                    .filter(|cluster| {
                        let namespace_matches = namespace_filter
                            .as_ref()
                            .is_none_or(|namespace| cluster.namespace == *namespace);
                        let name_matches = cluster_name
                            .as_ref()
                            .is_none_or(|name| cluster.name == *name);
                        namespace_matches && name_matches
                    })
                    .map(|cluster| GetClusterOutput {
                        name: cluster.name.clone(),
                        namespace: cluster.namespace.clone(),
                        enabled: cluster.enabled,
                        is_reachable: cluster.is_reachable.flatten(),
                        sso_enabled: cluster.sso_enabled,
                    })
                    .collect();

                outputs.sort_by(|a, b| {
                    a.namespace
                        .cmp(&b.namespace)
                        .then_with(|| a.name.cmp(&b.name))
                });

                let output = KubeList::new(outputs);
                println!("{}", output.to_output(self.format.clone()));
                Ok(())
            }
            Err(e) => {
                error!("Failed to retrieve clusters: {}", e);
                match &e {
                    ProxyAuthK8sError::Unauthenticated(_) => {
                        info!(
                            "Authentication failed: invalid or missing server token. Please run login first."
                        );
                    }
                    ProxyAuthK8sError::RemoteServerError(_) => {
                        info!("Server error occurred while retrieving clusters.");
                    }
                    _ => {
                        info!("An unexpected error occurred while retrieving clusters.");
                    }
                }
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(is_reachable: Option<bool>) -> GetClusterOutput {
        GetClusterOutput {
            name: "prod".to_string(),
            namespace: "team-a".to_string(),
            enabled: true,
            is_reachable,
            sso_enabled: false,
        }
    }

    #[test]
    fn a_row_lines_up_with_the_headers() {
        assert_eq!(
            GetClusterOutput::headers(),
            vec!["NAME", "NAMESPACE", "ENABLED", "REACHABLE", "SSO"]
        );
        assert_eq!(
            output(Some(true)).row(),
            vec!["prod", "team-a", "true", "true", "false"]
        );
        assert_eq!(
            GetClusterOutput::headers().len(),
            output(Some(true)).row().len()
        );
    }

    #[test]
    fn reachability_renders_the_three_states_apart() {
        // The server reports reachability as an Option: "not probed yet" must
        // not be flattened into "unreachable".
        assert_eq!(output(Some(true)).row()[3], "true");
        assert_eq!(output(Some(false)).row()[3], "false");
        assert_eq!(output(None).row()[3], "unknown");
    }
}
