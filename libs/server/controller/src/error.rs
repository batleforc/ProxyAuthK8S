use common::redis_pool::RedisPoolError;
use kube::runtime::finalizer;
use thiserror::Error;
use weebo_kit_runtime::events::{CLEANUP, RECONCILE};

use crate::reason::ReasonCode;

#[derive(Debug, Error)]
pub enum ControllerError {
    /// The `ProxyKubeApi` resources could not be listed at startup.
    #[error(
        "failed to list ProxyKubeApi resources (is the CRD installed and the RBAC granted?): {0}"
    )]
    CrdUnavailable(#[source] kube::Error),

    /// The installed `ProxyKubeApi` CRD predates RFC 0014: the apiserver
    /// would prune `upstream_auth` and a trusted cluster would read as
    /// `Passthrough` (decision 42).
    #[error(
        "the installed ProxyKubeApi CRD has no spec.upstream_auth (RFC 0014): upgrade chart-crd to this release before starting the controller"
    )]
    CrdOutdated,

    /// Kubernetes API error
    #[error("Kubernetes API error: {0}")]
    Kube(#[source] kube::Error),

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
    /// The reason code and Event `action` the error policy announces this
    /// error under (RFC 0011 §4.2), or `None` when there is no named,
    /// namespaced object to publish an Event on, or the error never reaches
    /// the error policy. No `_` arm: a new variant does not compile without
    /// its code.
    #[must_use]
    pub fn reason(&self) -> Option<(ReasonCode, &'static str)> {
        match self {
            Self::Redis(_) => Some((ReasonCode::CacheWriteFailed, RECONCILE)),
            Self::Kube(_) => Some((ReasonCode::StatusWriteFailed, RECONCILE)),
            Self::FinalizerError(error) => match error.as_ref() {
                finalizer::Error::ApplyFailed(inner) => inner.reason(),
                finalizer::Error::CleanupFailed(inner) => {
                    inner.reason().map(|(code, _)| (code, CLEANUP))
                }
                finalizer::Error::AddFinalizer(_) | finalizer::Error::RemoveFinalizer(_) => {
                    Some((ReasonCode::FinalizerFailed, RECONCILE))
                }
                finalizer::Error::UnnamedObject | finalizer::Error::InvalidFinalizer => None,
            },
            Self::InvalidResource(_) | Self::CrdUnavailable(_) | Self::CrdOutdated => None,
        }
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

    fn redis() -> ControllerError {
        RedisPoolError::Pool("connection refused".to_string()).into()
    }

    fn kube() -> kube::Error {
        kube::Error::LinesCodecMaxLineLengthExceeded
    }

    fn finalizer(error: finalizer::Error<ControllerError>) -> ControllerError {
        ControllerError::FinalizerError(Box::new(error))
    }

    #[test]
    fn redis_and_status_errors_have_their_codes() {
        assert_eq!(
            redis().reason(),
            Some((ReasonCode::CacheWriteFailed, RECONCILE))
        );
        assert_eq!(
            ControllerError::Kube(kube()).reason(),
            Some((ReasonCode::StatusWriteFailed, RECONCILE))
        );
    }

    #[test]
    fn the_finalizer_wrapper_keeps_the_inner_code_and_names_the_cleanup() {
        assert_eq!(
            finalizer(finalizer::Error::ApplyFailed(redis())).reason(),
            Some((ReasonCode::CacheWriteFailed, RECONCILE))
        );
        assert_eq!(
            finalizer(finalizer::Error::CleanupFailed(redis())).reason(),
            Some((ReasonCode::CacheWriteFailed, CLEANUP))
        );
        assert_eq!(
            finalizer(finalizer::Error::AddFinalizer(kube())).reason(),
            Some((ReasonCode::FinalizerFailed, RECONCILE))
        );
        assert_eq!(
            finalizer(finalizer::Error::RemoveFinalizer(kube())).reason(),
            Some((ReasonCode::FinalizerFailed, RECONCILE))
        );
    }

    #[test]
    fn errors_without_an_object_to_announce_on_have_no_code() {
        assert_eq!(finalizer(finalizer::Error::UnnamedObject).reason(), None);
        assert_eq!(finalizer(finalizer::Error::InvalidFinalizer).reason(), None);
        assert_eq!(ControllerError::InvalidResource("x".into()).reason(), None);
        assert_eq!(ControllerError::CrdUnavailable(kube()).reason(), None);
    }
}
