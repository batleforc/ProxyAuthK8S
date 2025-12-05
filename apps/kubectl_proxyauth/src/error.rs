use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyAuthK8sError {
    #[error("ERR000001: Kubeconfig path could not be calculated, either provide via --kubeconfig flag or set the KUBECONFIG environment variable")]
    KubeconfigPathCouldNotBeCalculated,
    #[error("ERR000002: Failed to read kubeconfig file: {0}")]
    KubeconfigReadError(String),
    #[error("ERR000003: Failed to parse kubeconfig file: {0}")]
    KubeconfigParseError(String),
}
