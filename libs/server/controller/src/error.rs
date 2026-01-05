use deadpool_redis::PoolError;
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

    // Redis pool error
    #[error("Redis pool error: {0}")]
    RedisPool(#[source] deadpool_redis::PoolError),
}

pub type Result<T, E = ControllerError> = std::result::Result<T, E>;

impl ControllerError {
    pub fn metric_label(&self) -> String {
        format!("{self:?}").to_lowercase()
    }
}

impl From<PoolError> for ControllerError {
    fn from(e: PoolError) -> Self {
        ControllerError::RedisPool(e)
    }
}
