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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_config_errors_keep_their_payload_across_the_boundary() {
        assert!(matches!(
            ProxyAuthK8sError::from(CliConfigError::InvalidServerUrl(
                "not a url".to_string(),
                "relative URL without a base".to_string(),
            )),
            ProxyAuthK8sError::InvalidServerUrl(url, error)
                if url == "not a url" && error == "relative URL without a base"
        ));
        assert!(matches!(
            ProxyAuthK8sError::from(CliConfigError::ServerNotFound("localhost-5437".to_string())),
            ProxyAuthK8sError::ServerNotFound(server) if server == "localhost-5437"
        ));
        assert!(matches!(
            ProxyAuthK8sError::from(CliConfigError::ClusterNotFound {
                server: "localhost-5437".to_string(),
                cluster: "prod".to_string(),
            }),
            ProxyAuthK8sError::ClusterNotFound { server, cluster }
                if server == "localhost-5437" && cluster == "prod"
        ));
        assert!(matches!(
            ProxyAuthK8sError::from(CliConfigError::YamlParseError("bad yaml".to_string())),
            ProxyAuthK8sError::YamlParseError(error) if error == "bad yaml"
        ));
        assert!(matches!(
            ProxyAuthK8sError::from(CliConfigError::YamlSerializeError("bad yaml".to_string())),
            ProxyAuthK8sError::YamlSerializeError(error) if error == "bad yaml"
        ));
    }

    #[test]
    fn a_401_from_the_cluster_list_becomes_unauthenticated() {
        // The distinction matters: `handle_get_clusters` only tells the user to
        // re-login on this variant.
        assert!(matches!(
            ProxyAuthK8sError::from(GetAllVisibleClusterError::Status401()),
            ProxyAuthK8sError::Unauthenticated(_)
        ));
    }

    #[test]
    fn other_cluster_list_failures_become_remote_server_errors() {
        assert!(matches!(
            ProxyAuthK8sError::from(GetAllVisibleClusterError::Status500()),
            ProxyAuthK8sError::RemoteServerError(_)
        ));

        let unknown = GetAllVisibleClusterError::UnknownValue(serde_json::json!({"code": 418}));
        match ProxyAuthK8sError::from(unknown) {
            // The unmodelled body is carried through so it can be grepped for.
            ProxyAuthK8sError::RemoteServerError(message) => assert!(message.contains("418")),
            other => panic!("expected a remote server error, got {other:?}"),
        }
    }

    #[test]
    fn a_response_error_unwraps_the_status_it_carries() {
        let response = client_api::apis::Error::ResponseError(client_api::apis::ResponseContent {
            status: reqwest::StatusCode::UNAUTHORIZED,
            content: String::new(),
            entity: Some(GetAllVisibleClusterError::Status401()),
        });
        assert!(matches!(
            ProxyAuthK8sError::from(response),
            ProxyAuthK8sError::Unauthenticated(_)
        ));
    }

    #[test]
    fn a_response_error_without_a_parsed_entity_still_reports_a_server_error() {
        let response: client_api::apis::Error<GetAllVisibleClusterError> =
            client_api::apis::Error::ResponseError(client_api::apis::ResponseContent {
                status: reqwest::StatusCode::BAD_GATEWAY,
                content: "<html>502</html>".to_string(),
                entity: None,
            });
        match ProxyAuthK8sError::from(response) {
            ProxyAuthK8sError::RemoteServerError(message) => {
                assert!(message.contains("No error details"));
            }
            other => panic!("expected a remote server error, got {other:?}"),
        }
    }

    #[test]
    fn transport_and_serde_failures_become_remote_server_errors() {
        let serde_err = serde_json::from_str::<i32>("nope").expect_err("this does not parse");
        match ProxyAuthK8sError::from(client_api::apis::Error::<GetAllVisibleClusterError>::Serde(
            serde_err,
        )) {
            ProxyAuthK8sError::RemoteServerError(message) => {
                assert!(message.contains("Serialization error"));
            }
            other => panic!("expected a remote server error, got {other:?}"),
        }

        let io_err = std::io::Error::other("socket closed");
        match ProxyAuthK8sError::from(client_api::apis::Error::<GetAllVisibleClusterError>::Io(
            io_err,
        )) {
            ProxyAuthK8sError::RemoteServerError(message) => {
                assert!(message.contains("Unexpected error"));
            }
            other => panic!("expected a remote server error, got {other:?}"),
        }
    }
}
