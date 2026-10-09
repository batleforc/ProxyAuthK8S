//! Error types raised while booting the shared state and the HTTP(S) listener.

use crate::{oidc_error, redis_pool::RedisPoolError};

/// Everything that can stop [`crate::State::new`] from booting.
#[derive(Debug, thiserror::Error)]
pub enum StateInitError {
    #[error("failed to create the Redis pool: {0}")]
    Redis(#[from] RedisPoolError),
    #[error("failed to create the Kubernetes client: {0}")]
    Kube(#[from] kube::Error),
    #[error("OIDC discovery failed: {0}")]
    Oidc(#[from] oidc_error::OidcError),
}

/// Everything that can stop [`crate::ServerConfig::rustls_config`] from
/// producing a TLS configuration.
#[derive(Debug, thiserror::Error)]
pub enum TlsConfigError {
    #[error("HTTPS is enabled but {0} is not set")]
    MissingPath(&'static str),
    #[error("failed to load the certificate chain: {0}")]
    Cert(String),
    #[error("failed to load the private key: {0}")]
    Key(String),
    #[error("rustls rejected the certificate/key pair: {0}")]
    Build(#[from] rustls::Error),
}
