//! The CLI's single error type.
//!
//! Every handler returns `Result<(), ProxyAuthK8sError>`; `run_cli` turns an
//! `Err` into a non-zero exit code. Each variant carries a stable `ERRxxxxxx`
//! code so a failure can be grepped for in a support thread.
//!
//! Conversions from the lower-level error types live in `convert.rs`.

use thiserror::Error;

mod convert;

#[derive(Debug, Error)]
pub enum ProxyAuthK8sError {
    #[error(
        "ERR000001: Kubeconfig path could not be calculated, either provide via --kubeconfig flag or set the KUBECONFIG environment variable"
    )]
    KubeconfigPathCouldNotBeCalculated,
    #[error("ERR000002: Failed to read kubeconfig file: {0}")]
    KubeconfigReadError(String),
    #[error("ERR000003: Failed to parse kubeconfig file: {0}")]
    KubeconfigParseError(String),
    #[error("ERR000004: Invalid server URL provided: {0} with error: {1}")]
    InvalidServerUrl(String, String),
    #[error("ERR000005: Server '{0}' not found in configuration")]
    ServerNotFound(String),
    #[error("ERR000016: Cluster '{cluster}' not found under server '{server}'")]
    ClusterNotFound { server: String, cluster: String },
    #[error("ERR000006: YAML Parse Error: {0}")]
    YamlParseError(String),
    #[error("ERR000007: YAML Serialize Error: {0}")]
    YamlSerializeError(String),
    #[error(
        "ERR000008: Configuration path could not be calculated, either provide via --proxy-auth-config flag or set the HOME environment variable"
    )]
    ConfigPathCouldNotBeCalculated,
    #[error("ERR000009: Failed to write kubeconfig file: {0}")]
    KubeconfigWriteError(String),
    #[error("ERR000010: Failed to read from keyring: {0}")]
    KeyringReadError(String),
    #[error("ERR000011: Failed to write to keyring: {0}")]
    KeyringWriteError(String),
    #[error("ERR000012: Failed to delete from keyring: {0}")]
    KeyringDeleteError(String),
    #[error("ERR000013: Remote Server error: {0}")]
    RemoteServerError(String),
    #[error("ERR000014: Unauthenticated: {0}")]
    Unauthenticated(String),
    #[error("ERR000015: Interactive SSO login failed: {0}")]
    SsoLoginError(String),
    #[error("ERR000017: {0}")]
    InvalidUsage(String),
    #[error("ERR000018: kubectl exec credential protocol error: {0}")]
    ExecCredential(String),
    #[error("ERR000019: Invalid certificate authority: {0}")]
    InvalidCertificateAuthority(String),
    #[error("ERR000020: Server certificate not trusted: {0}")]
    UntrustedServerTls(String),
    #[error("ERR000021: Not logged in to any ProxyAuthK8S server")]
    NotLoggedIn,
}

impl ProxyAuthK8sError {
    /// The next step for the user, printed under the error by `run_cli`.
    #[must_use]
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            Self::NotLoggedIn => {
                Some("log in first: kubectl proxyauth login --server-url https://<server>")
            }
            Self::ServerNotFound(_) => Some(
                "log in to that server first (kubectl proxyauth login --server-url <URL>); kubectl proxyauth config get lists the known ones",
            ),
            Self::ClusterNotFound { .. } => {
                Some("kubectl proxyauth get lists the clusters visible to you")
            }
            Self::Unauthenticated(_) => {
                Some("your session is over: run kubectl proxyauth login again")
            }
            Self::UntrustedServerTls(_) => Some(
                "rerun kubectl proxyauth login from a terminal to review the certificate, or pass --certificate-authority <FILE>",
            ),
            _ => None,
        }
    }
}
