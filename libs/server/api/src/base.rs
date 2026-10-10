use std::time::Duration;

use actix_web::{HttpResponse, Responder, get, web};
use common::State;
use serde::Serialize;
use tracing::{debug, instrument, warn};
use utoipa::ToSchema;

/// How long the readiness probe waits for Redis before reporting it down.
const READINESS_REDIS_TIMEOUT: Duration = Duration::from_secs(2);

/// Liveness: the process is up and serving HTTP.
///
/// Deliberately checks no dependency, so a Redis or IdP outage never makes
/// Kubernetes restart otherwise-healthy pods. Use `/management/ready` to decide
/// whether a pod should receive traffic.
#[utoipa::path(
    tag = "health",
    responses(
        (status = 200, description = "The server process is up."),
    )
)]
#[get("/health")]
#[instrument(name = "health", level = "debug")]
pub async fn health() -> impl Responder {
    debug!("Health check OK");
    HttpResponse::Ok().finish()
}

/// Body of the readiness probe.
#[derive(Serialize, ToSchema)]
pub struct ReadinessBody {
    /// `ok` when Redis answered a `PING`, `unavailable` otherwise.
    pub redis: &'static str,
    /// `ok` once the service-wide OIDC provider has been discovered, `pending`
    /// while the boot-time discovery is still retrying.
    pub oidc: &'static str,
}

/// Readiness: the pod can serve proxied traffic.
///
/// Every proxied request needs Redis (cluster registry, sessions, throttling),
/// so the pod is only ready when Redis answers a `PING` within 2 seconds, and
/// once the boot-time OIDC discovery has succeeded (it retries with backoff
/// instead of crashing the pod while the IdP is down).
#[utoipa::path(
    tag = "health",
    responses(
        (status = 200, description = "Ready to serve traffic.", body = ReadinessBody),
        (status = 503, description = "A required dependency (Redis, or the OIDC provider at boot) is unavailable.", body = ReadinessBody),
    )
)]
#[get("/ready")]
#[instrument(name = "ready", level = "debug", skip(state))]
pub async fn ready(state: web::Data<State>) -> impl Responder {
    let redis_ok = match tokio::time::timeout(READINESS_REDIS_TIMEOUT, state.redis_ping()).await {
        Ok(Ok(())) => true,
        Ok(Err(err)) => {
            warn!("Readiness check failed: Redis error: {}", err);
            false
        }
        Err(_) => {
            warn!("Readiness check failed: Redis PING timed out");
            false
        }
    };
    let oidc_ok = state.is_oidc_ready();
    if !oidc_ok {
        warn!("Readiness check failed: OIDC discovery has not succeeded yet");
    }
    let body = ReadinessBody {
        redis: if redis_ok { "ok" } else { "unavailable" },
        oidc: if oidc_ok { "ok" } else { "pending" },
    };
    if redis_ok && oidc_ok {
        HttpResponse::Ok().json(body)
    } else {
        HttpResponse::ServiceUnavailable().json(body)
    }
}
