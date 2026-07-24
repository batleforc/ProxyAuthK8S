use actix_web::{dev::PeerAddr, get, http, web::Data, HttpRequest, HttpResponse, Responder};
use common::State;
use crd::ProxyKubeApi;
use tracing::{error, instrument};

use crate::{
    api::get_all_visible_cluster_model::{GetAllVisibleClusterBody, VisibleCluster},
    model::user::User,
};

/// Get all cluster visible to the user.
///
/// Get all cluster visible to the user,
/// if the user is not authenticated return 401,
/// if none return an empty array.
#[utoipa::path(
    tag = "api_clusters",
    responses(
        (status = 200, description = "Get all visible clusters.", body = GetAllVisibleClusterBody),
        (status = 401, description = "User is not authenticated."),
        (status = 500, description = "Internal server error."),
    ),
    security(
        ("bearer_auth" = [])
    ),
)]
#[get("/clusters")]
#[instrument(name = "get_all_visible_cluster", skip(state))]
pub async fn get_all_visible_cluster(
    req: HttpRequest,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
    user: User,
    state: Data<State>,
) -> impl Responder {
    // Read through the index the controller maintains rather than scanning with
    // `KEYS`: the scan is O(N) and blocking, and a Redis cluster only answers it
    // for the node that happened to be reached.
    let cached: Vec<ProxyKubeApi> = match state.list_objects("proxyk8sauth".to_string()).await {
        Ok(cached) => cached,
        Err(e) => {
            error!(error = %e, "couldn't list the cached clusters");
            return HttpResponse::ServiceUnavailable().body(e.to_string());
        }
    };

    let proxies: Vec<VisibleCluster> = cached
        .into_iter()
        .filter(|kube_api| kube_api.is_user_allowed(&user.groups))
        .map(VisibleCluster::from)
        .collect();
    let body = GetAllVisibleClusterBody { clusters: proxies };
    HttpResponse::Ok().json(body)
}
