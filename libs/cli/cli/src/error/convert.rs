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
            // The server applies a throttle on this endpoint when an operator
            // has configured the `UNSCOPED_*` budget; retrying immediately would
            // only deepen a ban, so the message says to wait rather than
            // suggesting the request itself was malformed.
            GetAllVisibleClusterError::Status429() => ProxyAuthK8sError::RemoteServerError(
                "Rate limited or temporarily banned by the server, retry in a minute".to_owned(),
            ),
            GetAllVisibleClusterError::Status500() => ProxyAuthK8sError::RemoteServerError(
                "Invalid response from server, see debug to have more details".to_owned(),
            ),
            GetAllVisibleClusterError::Status503() => ProxyAuthK8sError::RemoteServerError(
                "The server is temporarily unable to serve this request, retry shortly".to_owned(),
            ),
            GetAllVisibleClusterError::UnknownValue(val) => {
                ProxyAuthK8sError::RemoteServerError(format!("Unknown error from server: {val}"))
            }
        }
    }
}

/// Longest slice of an unmodelled error body carried into the message.
const MAX_ERROR_BODY_CHARS: usize = 200;

impl From<client_api::apis::Error<GetAllVisibleClusterError>> for ProxyAuthK8sError {
    fn from(value: client_api::apis::Error<GetAllVisibleClusterError>) -> Self {
        match value {
            // Map from the HTTP status, not `entity`: the generated client
            // deserialises the body into an untagged enum whose unit variants
            // match `[]` and nothing else, so the server's empty 401 had no
            // entity and any `[]` error body passed for a 401.
            client_api::apis::Error::ResponseError(resp_content) => {
                match resp_content.status.as_u16() {
                    401 => GetAllVisibleClusterError::Status401().into(),
                    429 => GetAllVisibleClusterError::Status429().into(),
                    500 => GetAllVisibleClusterError::Status500().into(),
                    503 => GetAllVisibleClusterError::Status503().into(),
                    _ => {
                        let body = resp_content.content.trim();
                        let body: String = body.chars().take(MAX_ERROR_BODY_CHARS).collect();
                        if body.is_empty() {
                            ProxyAuthK8sError::RemoteServerError(format!(
                                "Server answered {} without error details",
                                resp_content.status
                            ))
                        } else {
                            ProxyAuthK8sError::RemoteServerError(format!(
                                "Server answered {}: {body}",
                                resp_content.status
                            ))
                        }
                    }
                }
            }
            client_api::apis::Error::Serde(err) => {
                ProxyAuthK8sError::RemoteServerError(format!("Serialization error: {err}"))
            }
            client_api::apis::Error::Reqwest(err) if is_certificate_error(&err) => {
                ProxyAuthK8sError::UntrustedServerCertificate(
                    "the server's TLS certificate is not trusted; if it was renewed, run \
                     `kubectl proxyauth login` from a terminal to review the new one, or pass \
                     --certificate-authority"
                        .to_string(),
                )
            }
            other => ProxyAuthK8sError::RemoteServerError(format!("Unexpected error: {other:?}")),
        }
    }
}

/// Whether a request failed on the server's certificate, whichever TLS
/// backend reqwest used (rustls: "invalid peer certificate: UnknownIssuer";
/// OpenSSL: "certificate verify failed"; Secure Transport/SChannel likewise
/// name the certificate).
fn is_certificate_error(err: &reqwest::Error) -> bool {
    let mut source: Option<&dyn std::error::Error> = Some(err);
    while let Some(current) = source {
        let message = current.to_string().to_ascii_lowercase();
        if message.contains("certificate") || message.contains("unknownissuer") {
            return true;
        }
        source = current.source();
    }
    false
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

    fn response_error(
        status: reqwest::StatusCode,
        content: &str,
        entity: Option<GetAllVisibleClusterError>,
    ) -> client_api::apis::Error<GetAllVisibleClusterError> {
        client_api::apis::Error::ResponseError(client_api::apis::ResponseContent {
            status,
            content: content.to_string(),
            entity,
        })
    }

    #[test]
    fn a_401_is_unauthenticated_whatever_the_body() {
        // The server answers its 401 with an empty body: that must still ask
        // the user to re-login.
        for content in ["", "[]", "{\"reason\":\"expired\"}"] {
            assert!(
                matches!(
                    ProxyAuthK8sError::from(response_error(
                        reqwest::StatusCode::UNAUTHORIZED,
                        content,
                        None,
                    )),
                    ProxyAuthK8sError::Unauthenticated(_)
                ),
                "body {content:?}"
            );
        }
    }

    #[test]
    fn the_status_wins_over_the_parsed_entity() {
        // `[]` parses as `Status401()` whatever the status: a 500 with that
        // body is still a server error.
        match ProxyAuthK8sError::from(response_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "[]",
            Some(GetAllVisibleClusterError::Status401()),
        )) {
            ProxyAuthK8sError::RemoteServerError(message) => {
                assert!(message.contains("Invalid response from server"));
            }
            other => panic!("expected a remote server error, got {other:?}"),
        }
    }

    #[test]
    fn modelled_statuses_keep_their_message() {
        for (status, needle) in [
            (reqwest::StatusCode::TOO_MANY_REQUESTS, "Rate limited"),
            (
                reqwest::StatusCode::SERVICE_UNAVAILABLE,
                "temporarily unable",
            ),
        ] {
            match ProxyAuthK8sError::from(response_error(status, "", None)) {
                ProxyAuthK8sError::RemoteServerError(message) => {
                    assert!(message.contains(needle), "{status}: {message}");
                }
                other => panic!("expected a remote server error, got {other:?}"),
            }
        }
    }

    #[test]
    fn other_statuses_report_the_status_and_a_bounded_body() {
        match ProxyAuthK8sError::from(response_error(
            reqwest::StatusCode::BAD_GATEWAY,
            "<html>502</html>",
            None,
        )) {
            ProxyAuthK8sError::RemoteServerError(message) => {
                assert!(message.contains("502"));
                assert!(message.contains("<html>502</html>"));
            }
            other => panic!("expected a remote server error, got {other:?}"),
        }

        match ProxyAuthK8sError::from(response_error(reqwest::StatusCode::FORBIDDEN, " ", None)) {
            ProxyAuthK8sError::RemoteServerError(message) => {
                assert!(message.contains("403"));
                assert!(message.contains("without error details"));
            }
            other => panic!("expected a remote server error, got {other:?}"),
        }

        let long = "x".repeat(MAX_ERROR_BODY_CHARS * 2);
        match ProxyAuthK8sError::from(response_error(
            reqwest::StatusCode::BAD_REQUEST,
            &long,
            None,
        )) {
            ProxyAuthK8sError::RemoteServerError(message) => {
                assert!(
                    message.len() < MAX_ERROR_BODY_CHARS + 50,
                    "{}",
                    message.len()
                );
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

    #[tokio::test]
    async fn an_untrusted_server_certificate_says_how_to_recover() {
        use crate::test_support::{TlsTestServer, self_signed_cert};
        let server = TlsTestServer::start(&[&self_signed_cert("proxy", &["localhost"])]).await;
        let err = client_api::apis::api_clusters_api::get_all_visible_cluster(
            &client_api::apis::configuration::Configuration {
                base_path: server.url(),
                client: crate::cli_config::cli_server_config::http_client(None).unwrap(),
                ..Default::default()
            },
        )
        .await
        .expect_err("the certificate is not trusted");
        match ProxyAuthK8sError::from(err) {
            ProxyAuthK8sError::UntrustedServerCertificate(message) => {
                assert!(message.contains("kubectl proxyauth login"), "{message}");
            }
            other => panic!("expected an untrusted certificate error, got {other:?}"),
        }
    }
}
