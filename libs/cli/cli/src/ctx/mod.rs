//! The command context: everything a handler needs, resolved once at startup.
//!
//! [`CliCtx`] is built from the parsed [`Cli`] arguments (see `build.rs`) and
//! then threaded through every command handler.

use std::{env, path::PathBuf};

use cli_trace::level::VerboseLevel;
use kube::config::Kubeconfig;
use serde::{Deserialize, Serialize};

use crate::{cli_config::CliConfig, error::ProxyAuthK8sError};

mod build;
pub mod format;

pub use format::ContextFormat;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CliCtx {
    pub namespace: String,
    pub kubeconfig_path: PathBuf,
    pub kubeconfig: Kubeconfig,
    pub context: Option<String>,
    pub verbose: Option<u8>,
    pub server_url: String,
    pub format: ContextFormat,
    pub invoked_from_kubectl: bool,
    pub config: CliConfig,
    pub config_path: PathBuf,
}

impl CliCtx {
    #[must_use]
    pub fn detect_kubeconfig_path(kubeconfig: Option<String>) -> Option<String> {
        if let Some(path) = kubeconfig {
            Some(path)
        } else if let Ok(env_path) = env::var("KUBECONFIG") {
            Some(env_path)
        } else {
            let home_env = env::var("HOME").unwrap_or_default();
            if home_env.is_empty() {
                None
            } else {
                Some(format!("{home_env}/.kube/config"))
            }
        }
    }

    pub fn write_kubeconfig(&self) -> Result<(), ProxyAuthK8sError> {
        let yaml_content = serde_yaml::to_string(&self.kubeconfig)
            .map_err(|e| ProxyAuthK8sError::YamlSerializeError(e.to_string()))?;
        crate::helper::secure_write(&self.kubeconfig_path, &yaml_content)
            .map_err(|e| ProxyAuthK8sError::KubeconfigWriteError(e.to_string()))
    }

    #[must_use]
    pub fn to_tracing_verbose_level(&self) -> VerboseLevel {
        match self.verbose.unwrap_or(0) {
            0 => VerboseLevel::INFO,
            1 => VerboseLevel::DEBUG,
            2 => VerboseLevel::TRACE,
            _ => VerboseLevel::TRACE,
        }
    }
}
