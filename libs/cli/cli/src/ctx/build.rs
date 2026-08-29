//! Construction of the [`CliCtx`] from the parsed [`Cli`] arguments.
//!
//! Split out of `mod.rs` because it is the one place that touches the
//! filesystem: locating and reading the kubeconfig and the CLI config, and
//! creating either when missing.

use std::{env, fs, path::PathBuf};

use kube::config::Kubeconfig;

use crate::{Cli, cli_config::CliConfig, ctx::CliCtx, error::ProxyAuthK8sError};

impl TryFrom<Cli> for CliCtx {
    type Error = ProxyAuthK8sError;

    /// Build the context, surfacing every failure as a clean error instead of a
    /// panic. This matters because the CLI is also a `kubectl` exec-credential
    /// plugin: a panic would emit a Rust backtrace and a non-protocol exit,
    /// breaking `kubectl` auth with an opaque crash on a merely malformed config.
    fn try_from(cli: Cli) -> Result<Self, Self::Error> {
        let kubeconfig_path =
            CliCtx::detect_kubeconfig_path(cli.kubeconfig.map(|p| p.to_string_lossy().to_string()))
                .map(PathBuf::from)
                .ok_or(ProxyAuthK8sError::KubeconfigPathCouldNotBeCalculated)?;

        if !kubeconfig_path.exists() {
            // If the kubeconfig file does not exist, create an empty one (0600).
            crate::helper::secure_write(&kubeconfig_path, "").map_err(|e| {
                ProxyAuthK8sError::KubeconfigWriteError(format!(
                    "Failed to create kubeconfig file at {}: {}",
                    kubeconfig_path.to_string_lossy(),
                    e
                ))
            })?;
        }
        let kubeconfig_content = fs::read_to_string(&kubeconfig_path).map_err(|e| {
            ProxyAuthK8sError::KubeconfigReadError(format!(
                "Failed to read kubeconfig file at {}: {}",
                kubeconfig_path.to_string_lossy(),
                e
            ))
        })?;
        let kubeconfig = Kubeconfig::from_yaml(&kubeconfig_content).map_err(|e| {
            ProxyAuthK8sError::KubeconfigReadError(format!(
                "Failed to parse kubeconfig file at {}: {}",
                kubeconfig_path.to_string_lossy(),
                e
            ))
        })?;
        let invoked_from_kubectl = env::args().next().is_some_and(|arg0| {
            PathBuf::from(arg0)
                .file_stem()
                .is_some_and(|stem| stem == "kubectl")
        });
        // Load CLI configuration
        let config_path = if let Some(path) = cli.proxy_auth_config {
            path
        } else {
            let home_env = env::var("HOME").unwrap_or_default();
            if home_env.is_empty() {
                return Err(ProxyAuthK8sError::ConfigPathCouldNotBeCalculated);
            }
            PathBuf::from(format!("{home_env}/.kube/proxyauth_config.yaml"))
        };
        let config = if config_path.exists() {
            CliConfig::read_from_file(config_path.clone()).map_err(|e| {
                ProxyAuthK8sError::KubeconfigReadError(format!(
                    "Failed to read config file at {}: {}",
                    config_path.to_string_lossy(),
                    e
                ))
            })?
        } else {
            CliConfig::default()
                .write_to_file(config_path.clone())
                .cloned()
                .map_err(|e| {
                    ProxyAuthK8sError::KubeconfigWriteError(format!(
                        "Failed to create default config file at {}: {}",
                        config_path.to_string_lossy(),
                        e
                    ))
                })?
        };

        Ok(CliCtx {
            namespace: cli.namespace,
            kubeconfig,
            kubeconfig_path,
            context: cli.context,
            verbose: cli.verbose,
            server_url: cli.server_url,
            format: cli.format,
            invoked_from_kubectl,
            config,
            config_path,
        })
    }
}
