use core::error;

use client_api::apis::api_clusters_api::GetAllVisibleClusterError;
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
    #[error("ERR000009: Failed to write kubeconfig file: {0}")]
    KubeconfigWriteError(String),
    #[error("ERR000010: Failed to read from keyring: {0}")]
    KeyringReadError(String),
    #[error("ERR000011: Failed to write to keyring: {0}")]
    KeyringWriteError(String),
    #[error("ERR000012: Remote Server error: {0}")]
    RemoteServerError(String),
    #[error("ERR000013: Unauthenticated: {0}")]
    Unauthenticated(String),
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

impl From<GetAllVisibleClusterError> for ProxyAuthK8sError {
    fn from(value: GetAllVisibleClusterError) -> Self {
        match value {
            GetAllVisibleClusterError::Status401() => ProxyAuthK8sError::Unauthenticated(format!(
                "Authentification failed, please re-login to the server."
            )),
            GetAllVisibleClusterError::Status500() => ProxyAuthK8sError::RemoteServerError(
                format!("Invalid response from server, see debug to have more details",),
            ),
            GetAllVisibleClusterError::UnknownValue(val) => {
                ProxyAuthK8sError::RemoteServerError(format!("Unknown error from server: {}", val))
            }
        }
    }
}

impl From<client_api::apis::Error<GetAllVisibleClusterError>> for ProxyAuthK8sError {
    fn from(value: client_api::apis::Error<GetAllVisibleClusterError>) -> Self {
        match value {
            client_api::apis::Error::ResponseError(resp_content) => match resp_content.entity {
                Some(err) => ProxyAuthK8sError::from(err),
                None => ProxyAuthK8sError::RemoteServerError(
                    "No error details provided by server".to_string(),
                ),
            },
            client_api::apis::Error::Serde(err) => {
                ProxyAuthK8sError::RemoteServerError(format!("Serialization error: {}", err))
            }
            other => ProxyAuthK8sError::RemoteServerError(format!("Unexpected error: {:?}", other)),
        }
    }
}
