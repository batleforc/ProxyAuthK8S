use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::ProxyAuthK8sError;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CliCtx {
    pub namespace: String,
    pub kubeconfig: PathBuf,
    pub verbose: Option<u8>,
    pub server_url: String,
    pub format: String,
}

impl From<super::Cli> for CliCtx {
    fn from(cli: super::Cli) -> Self {
        let kubeconfig_path = match CliCtx::detect_kubeconfig_path(
            cli.kubeconfig.map(|p| p.to_string_lossy().to_string()),
        ) {
            Some(path) => PathBuf::from(path),
            None => {
                panic!("{}", ProxyAuthK8sError::KubeconfigPathCouldNotBeCalculated)
            }
        };
        if !kubeconfig_path.exists() {
            panic!(
                "{}",
                ProxyAuthK8sError::KubeconfigReadError(format!(
                    "Kubeconfig file does not exist at path: {}",
                    kubeconfig_path.to_string_lossy()
                ))
            );
        }
        CliCtx {
            namespace: cli.namespace,
            kubeconfig: kubeconfig_path,
            verbose: cli.verbose,
            server_url: cli.server_url,
            format: cli.format,
        }
    }
}

impl CliCtx {
    pub fn detect_kubeconfig_path(kubeconfig: Option<String>) -> Option<String> {
        if let Some(path) = kubeconfig {
            Some(path)
        } else if let Ok(env_path) = std::env::var("KUBECONFIG") {
            Some(env_path)
        } else {
            let home_env = std::env::var("HOME").unwrap_or_default();
            if !home_env.is_empty() {
                Some(format!("{}/.kube/config", home_env))
            } else {
                None
            }
        }
    }
}
