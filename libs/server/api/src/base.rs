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
}

/// Readiness: the pod can serve proxied traffic.
///
/// Every proxied request needs Redis (cluster registry, sessions, throttling),
/// so the pod is only ready when Redis answers a `PING` within 2 seconds.
#[utoipa::path(
    tag = "health",
    responses(
        (status = 200, description = "Ready to serve traffic.", body = ReadinessBody),
        (status = 503, description = "A required dependency (Redis) is unavailable.", body = ReadinessBody),
    )
)]
#[get("/ready")]
#[instrument(name = "ready", level = "debug", skip(state))]
pub async fn ready(state: web::Data<State>) -> impl Responder {
    match tokio::time::timeout(READINESS_REDIS_TIMEOUT, state.redis_ping()).await {
        Ok(Ok(())) => HttpResponse::Ok().json(ReadinessBody { redis: "ok" }),
        Ok(Err(err)) => {
            warn!("Readiness check failed: Redis error: {}", err);
            HttpResponse::ServiceUnavailable().json(ReadinessBody {
                redis: "unavailable",
            })
        }
        Err(_) => {
            warn!("Readiness check failed: Redis PING timed out");
            HttpResponse::ServiceUnavailable().json(ReadinessBody {
                redis: "unavailable",
            })
        }
    }
}
