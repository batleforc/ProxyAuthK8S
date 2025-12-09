use clap::ValueEnum;
use cli_trace::level::VerboseLevel;
use kube::config::Kubeconfig;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf};

use crate::{cli_config::CliConfig, error::ProxyAuthK8sError};

#[derive(Serialize, Deserialize, Debug, Clone, Default, ValueEnum)]
pub enum ContextFormat {
    #[default]
    Table,
    Json,
    Yaml,
}

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
        let kubeconfig = match fs::read_to_string(kubeconfig_path.clone()) {
            Ok(content) => Kubeconfig::from_yaml(&content).unwrap_or_else(|e| {
                panic!(
                    "{}",
                    ProxyAuthK8sError::KubeconfigReadError(format!(
                        "Failed to parse kubeconfig file at {}: {}",
                        kubeconfig_path.to_string_lossy(),
                        e
                    ))
                )
            }),
            Err(e) => {
                panic!(
                    "{}",
                    ProxyAuthK8sError::KubeconfigReadError(format!(
                        "Failed to read kubeconfig file at {}: {}",
                        kubeconfig_path.to_string_lossy(),
                        e
                    ))
                )
            }
        };
        let invoked_from_kubectl = env::args().next().map_or(false, |arg0| {
            PathBuf::from(arg0)
                .file_stem()
                .map_or(false, |stem| stem == "kubectl")
        });
        // Validate that server_url is a valid URL
        if let Err(err) = Url::parse(&cli.server_url) {
            panic!(
                "{}",
                ProxyAuthK8sError::InvalidServerUrl(cli.server_url.clone(), err.to_string())
            );
        }
        // Load CLI configuration
        let config_path = if let Some(path) = cli.proxy_auth_config {
            path
        } else {
            let home_env = env::var("HOME").unwrap_or_default();
            if home_env.is_empty() {
                panic!("{}", ProxyAuthK8sError::ConfigPathCouldNotBeCalculated);
            }
            PathBuf::from(format!("{}/.kube/proxyauth_config.yaml", home_env))
        };
        let config = if !config_path.exists() {
            match CliConfig::default().write_to_file(config_path.clone()) {
                Ok(config) => config.clone(),
                Err(e) => {
                    panic!(
                        "{}",
                        ProxyAuthK8sError::KubeconfigWriteError(format!(
                            "Failed to create default config file at {}: {}",
                            config_path.to_string_lossy(),
                            e
                        ))
                    )
                }
            }
        } else {
            CliConfig::read_from_file(config_path.clone()).unwrap_or_else(|e| {
                panic!(
                    "{}",
                    ProxyAuthK8sError::KubeconfigReadError(format!(
                        "Failed to read config file at {}: {}",
                        config_path.to_string_lossy(),
                        e
                    ))
                )
            })
        };

        CliCtx {
            namespace: cli.namespace,
            kubeconfig,
            kubeconfig_path,
            context: cli.context,
            verbose: cli.verbose,
            server_url: cli.server_url,
            format: cli.format,
            invoked_from_kubectl,
            config: config,
            config_path,
        }
    }
}

impl CliCtx {
    pub fn detect_kubeconfig_path(kubeconfig: Option<String>) -> Option<String> {
        if let Some(path) = kubeconfig {
            Some(path)
        } else if let Ok(env_path) = env::var("KUBECONFIG") {
            Some(env_path)
        } else {
            let home_env = env::var("HOME").unwrap_or_default();
            if !home_env.is_empty() {
                Some(format!("{}/.kube/config", home_env))
            } else {
                None
            }
        }
    }

    pub fn write_kubeconfig(&self) -> Result<(), ProxyAuthK8sError> {
        let yaml_content = serde_yaml::to_string(&self.kubeconfig)
            .map_err(|e| ProxyAuthK8sError::YamlSerializeError(e.to_string()))?;
        fs::write(&self.kubeconfig_path, yaml_content)
            .map_err(|e| ProxyAuthK8sError::KubeconfigWriteError(e.to_string()))
    }

    pub fn to_tracing_verbose_level(&self) -> VerboseLevel {
        match self.verbose.unwrap_or(0) {
            0 => VerboseLevel::INFO,
            1 => VerboseLevel::DEBUG,
            2 => VerboseLevel::TRACE,
            _ => VerboseLevel::TRACE,
        }
    }
}
