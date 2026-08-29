//! Conversions into [`ProxyAuthK8sError`] from the layers below: the persisted
//! CLI config and the generated `client_api` HTTP client.

use client_api::apis::api_clusters_api::GetAllVisibleClusterError;

use crate::{cli_config::error::CliConfigError, error::ProxyAuthK8sError};

impl From<CliConfigError> for ProxyAuthK8sError {
    fn from(err: CliConfigError) -> Self {
        match err {
            CliConfigError::InvalidServerUrl(url, error) => {
                ProxyAuthK8sError::InvalidServerUrl(url, error)
            }
            CliConfigError::ServerNotFound(server) => ProxyAuthK8sError::ServerNotFound(server),
            CliConfigError::ClusterNotFound { server, cluster } => {
                ProxyAuthK8sError::ClusterNotFound { server, cluster }
            }
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
            GetAllVisibleClusterError::Status401() => ProxyAuthK8sError::Unauthenticated(
                "Authentification failed, please re-login to the server.".to_owned(),
            ),
            GetAllVisibleClusterError::Status500() => ProxyAuthK8sError::RemoteServerError(
                "Invalid response from server, see debug to have more details".to_owned(),
            ),
            GetAllVisibleClusterError::UnknownValue(val) => {
                ProxyAuthK8sError::RemoteServerError(format!("Unknown error from server: {val}"))
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
                ProxyAuthK8sError::RemoteServerError(format!("Serialization error: {err}"))
            }
            other => ProxyAuthK8sError::RemoteServerError(format!("Unexpected error: {other:?}")),
        }
    }
}
