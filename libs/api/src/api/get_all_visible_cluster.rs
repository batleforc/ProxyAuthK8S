use actix_web::{dev::PeerAddr, get, http, web::Data, HttpRequest, HttpResponse, Responder};
use common::State;
use crd::ProxyKubeApi;
use deadpool_redis::redis::AsyncCommands;
use tracing::instrument;

use crate::{
    api::get_all_visible_cluster_model::{GetAllVisibleClusterBody, VisibleCluster},
    model::user::User,
};

/// Get all cluster visible to the user
///
/// if none return an empty array
#[utoipa::path(
    tag = "api_clusters",
    responses(
        (status = 200, description = "ATM nothing real", body = GetAllVisibleClusterBody),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error."),
    ),
    security(
        ("oauth2" = [])
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
    let mut conn = state.get_redis_conn().await.unwrap();
    // Get all the keys matching the pattern "cluster:*"
    let keys: Vec<String> = conn.keys("proxyk8sauth:*").await.unwrap_or_default();
    // Get all the values for the keys
    let values: Vec<String> = conn.mget(&keys).await.unwrap_or_default();
    // Parse the values into ProxyKubeApi objects
    let proxies: Vec<VisibleCluster> = values
        .into_iter()
        .filter_map(|v| ProxyKubeApi::from_json(&v))
        .filter(|kube_api| kube_api.is_user_allowed(&user.groups))
        .map(VisibleCluster::from)
        .collect();
    let body = GetAllVisibleClusterBody { clusters: proxies };
    HttpResponse::Ok().json(body)
}
