use common::redis_pool::RedisPoolError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ControllerError {
    /// The `ProxyKubeApi` resources could not be listed at startup.
    #[error(
        "failed to list ProxyKubeApi resources (is the CRD installed and the RBAC granted?): {0}"
    )]
    CrdUnavailable(#[source] kube::Error),

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
    fn redis_errors_convert_into_the_redis_variant() {
        let err: ControllerError = RedisPoolError::Pool("connection refused".to_string()).into();
        assert!(matches!(err, ControllerError::Redis(_)));
        assert_eq!(
            err.to_string(),
            "Redis error: could not get a redis connection: connection refused"
        );
    }

    #[test]
    fn invalid_resource_displays_its_reason() {
        let err = ControllerError::InvalidResource("ProxyKubeApi has no namespace".to_string());
        assert_eq!(
            err.to_string(),
            "Invalid resource: ProxyKubeApi has no namespace"
        );
    }

    #[test]
    fn crd_unavailable_names_the_likely_cause_and_keeps_the_source() {
        use std::error::Error as _;
        let err = ControllerError::CrdUnavailable(kube::Error::LinesCodecMaxLineLengthExceeded);
        assert!(
            err.to_string()
                .starts_with("failed to list ProxyKubeApi resources (is the CRD installed"),
            "{err}"
        );
        assert!(err.source().is_some());
    }

    #[test]
    fn metric_label_is_lowercase_and_names_the_variant() {
        let err = ControllerError::InvalidResource("Bad".to_string());
        let label = err.metric_label();
        assert_eq!(label, label.to_lowercase());
        assert!(label.starts_with("invalidresource"), "{label}");
    }
}
