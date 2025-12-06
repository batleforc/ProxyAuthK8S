use thiserror::Error;

use crate::cli_config::error::CliConfigError;

#[derive(Debug, Error)]
pub enum ProxyAuthK8sError {
    #[error("ERR000001: Kubeconfig path could not be calculated, either provide via --kubeconfig flag or set the KUBECONFIG environment variable")]
    KubeconfigPathCouldNotBeCalculated,
    #[error("ERR000002: Failed to read kubeconfig file: {0}")]
    KubeconfigReadError(String),
    #[error("ERR000003: Failed to parse kubeconfig file: {0}")]
    KubeconfigParseError(String),
    #[error("ERR000004: Invalid server URL provided: {0} with error: {1}")]
    InvalidServerUrl(String, String),
    #[error("ERR000005: Server '{0}' not found in configuration")]
    ServerNotFound(String),
    #[error("ERR000006: YAML Parse Error: {0}")]
    YamlParseError(String),
    #[error("ERR000007: YAML Serialize Error: {0}")]
    YamlSerializeError(String),
    #[error("ERR000008: Configuration path could not be calculated, either provide via --proxy-auth-config flag or set the HOME environment variable")]
    ConfigPathCouldNotBeCalculated,
}

impl From<CliConfigError> for ProxyAuthK8sError {
    fn from(err: CliConfigError) -> Self {
        match err {
            CliConfigError::InvalidServerUrl(url, error) => {
                ProxyAuthK8sError::InvalidServerUrl(url, error)
            }
            CliConfigError::ServerNotFound(server) => ProxyAuthK8sError::ServerNotFound(server),
            CliConfigError::YamlParseError(error) => ProxyAuthK8sError::YamlParseError(error),
            CliConfigError::YamlSerializeError(error) => {
                ProxyAuthK8sError::YamlSerializeError(error)
            }
        }
    }
}
