use actix_web::{get, HttpResponse, Responder};
use tracing::{info, instrument};

/// Base path just to answer if the server is up and running.
///
/// This is also used for health checks.
#[utoipa::path(
    tag = "health",
    responses(
        (status = 200, description = "Nothing"),
        (status = 500, description = "Internal server error."),
    )
)]
#[get("/health")]
#[instrument(name = "health")]
pub async fn health() -> impl Responder {
    info!("Health check OK");
    HttpResponse::Ok().finish()
}
