use common::redis_pool::RedisPoolError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ControllerError {
    /// Kubernetes API error
    #[error("Kubernetes API error: {0}")]
    Kube(#[source] kube::Error),

    // Serialization/Deserialization error
    #[error("Serialization/Deserialization error: {0}")]
    Serde(#[source] serde_json::Error),

    // Finalizer error
    #[error("Finalizer error: {0}")]
    FinalizerError(#[source] Box<kube::runtime::finalizer::Error<ControllerError>>),

    // Invalid resource error
    #[error("Invalid resource: {0}")]
    InvalidResource(String),

    // Redis error
    #[error("Redis error: {0}")]
    Redis(#[source] RedisPoolError),
}

pub type Result<T, E = ControllerError> = std::result::Result<T, E>;

impl ControllerError {
    pub fn metric_label(&self) -> String {
        format!("{self:?}").to_lowercase()
    }
}

impl From<RedisPoolError> for ControllerError {
    fn from(e: RedisPoolError) -> Self {
        ControllerError::Redis(e)
    }
}
