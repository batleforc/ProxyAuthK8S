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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_metric_label_is_a_lowercased_debug_rendering() {
        // The label ends up as a Prometheus label value, so it must stay a
        // stable, lowercase, single-line string per variant.
        let label = ControllerError::InvalidResource("No Namespace".to_string()).metric_label();
        assert_eq!(label, r#"invalidresource("no namespace")"#);
        assert_eq!(label, label.to_lowercase());
        assert!(!label.contains('\n'));
    }

    #[test]
    fn a_redis_error_converts_into_the_controller_error() {
        // `reconcile` relies on `?`/`From` to surface a failed Redis write
        // rather than reporting a success it did not achieve.
        let error: ControllerError =
            RedisPoolError::Pool("no connection available".to_string()).into();
        assert!(
            matches!(error, ControllerError::Redis(_)),
            "expected a Redis error, got {error:?}"
        );
        assert!(error.to_string().starts_with("Redis error:"));
    }
}
