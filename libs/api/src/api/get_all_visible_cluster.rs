use actix_web::{dev::PeerAddr, get, http, HttpRequest, Responder};
use tracing::instrument;

use crate::model::user::User;

/// Get all cluster visible to the user
///
/// if none return an empty array
#[utoipa::path(
    tag = "front",
    responses(
        (status = 200, description = "ATM nothing real"),
        (status = 500, description = "Internal server error."),
    )
)]
#[get("/clusters")]
#[instrument(name = "get_all_visible_cluster")]
pub async fn get_all_visible_cluster(
    req: HttpRequest,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
    user: User,
) -> impl Responder {
    format!(
        "Hello {}, this is a placeholder for all visible clusters.",
        user.username
    )
}
