use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{Method, StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use crd::ProxyKubeApi;
use tracing::{error, instrument};

use crate::{
    cluster::redirect::{forwarded, status_response::too_many_requests, throttle},
    helper::{extract_authorization_header, forwarded_for, peer_ip},
    model::user::User,
    visible_clusters::get_all_visible_cluster_model::{GetAllVisibleClusterBody, VisibleCluster},
};

/// Get all cluster visible to the user.
///
/// Get all cluster visible to the user,
/// if the user is not authenticated return 401,
/// if none return an empty array.
#[utoipa::path(
    get,
    path = "/clusters",
    tag = "api_clusters",
    responses(
        (status = 200, description = "Get all visible clusters.", body = GetAllVisibleClusterBody),
        (status = 401, description = "User is not authenticated."),
        (status = 429, description = "Rate limited or banned (see UNSCOPED_* environment knobs)."),
        (status = 500, description = "Internal server error."),
        (status = 503, description = "Redis is unavailable.")
    ),
    security(
        ("bearer_auth" = [])
    ),
)]
#[instrument(name = "get_all_visible_cluster", skip(state, req))]
pub async fn get_all_visible_cluster(
    State(state): State<Arc<common::State>>,
    method: Method,
    req: Parts,
) -> Response {
    // Authentication happens here rather than through the `User` extractor: an
    // extractor runs before the handler, so its outbound `/userinfo` call to the
    // provider would already have happened by the time any gate could refuse the
    // caller. Doing it in the body puts the ban check first and lets a rejected
    // token feed the failure counter, which the extractor's bare 401 never did.
    //
    // Unlike the cluster endpoints there is no `ProxyKubeApi` here to carry a
    // policy, so this uses the unscoped budget — off unless an operator has
    // configured it (see `throttle::unscoped`).
    let peer_id = forwarded::throttle_client_ip(forwarded_for(&req.headers), peer_ip(&req));

    if throttle::unscoped::is_banned(&state, &peer_id).await {
        let retry_after = throttle::unscoped::ban_retry_after(&state, &peer_id).await;
        let throttled = throttle::Throttled::Banned { retry_after };
        error!(peer = %peer_id, "refusing a banned client on the dashboard surface");
        return too_many_requests(&throttled.message(), throttled.retry_after());
    }
    if let Some(throttled) = throttle::unscoped::check_rate_limit(&state, &peer_id).await {
        return too_many_requests(&throttled.message(), throttled.retry_after());
    }

    let token = match extract_authorization_header(&req.headers) {
        Ok(token) => token,
        Err(e) => {
            error!("Authorization header extraction failed: {}", e);
            throttle::unscoped::record_auth_failure(&state, &peer_id).await;
            return StatusCode::UNAUTHORIZED.into_response();
        }
    };
    let user =
        match User::get_user_info_from_oidc_token(token.to_string(), state.oidc_client.clone())
            .await
        {
            Ok(Some(user)) => {
                throttle::unscoped::clear_auth_failures(&state, &peer_id).await;
                user
            }
            Ok(None) => {
                error!("User info not found in OIDC response");
                throttle::unscoped::record_auth_failure(&state, &peer_id).await;
                return StatusCode::UNAUTHORIZED.into_response();
            }
            Err(e) => {
                error!(error = %e, "couldn't resolve the caller's token");
                // A provider outage must not ban whoever happened to be calling.
                if e.is_caller_fault() {
                    throttle::unscoped::record_auth_failure(&state, &peer_id).await;
                }
                return StatusCode::UNAUTHORIZED.into_response();
            }
        };

    // Read through the index the controller maintains rather than scanning with
    // `KEYS`: the scan is O(N) and blocking, and a Redis cluster only answers it
    // for the node that happened to be reached.
    let cached: Vec<ProxyKubeApi> = match state.list_objects(crd::REDIS_PREFIX).await {
        Ok(cached) => cached,
        Err(e) => {
            error!(error = %e, "couldn't list the cached clusters");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };

    let proxies: Vec<VisibleCluster> = cached
        .into_iter()
        .filter(|kube_api| kube_api.is_user_allowed(&user.groups))
        .map(VisibleCluster::from)
        .collect();
    let body = GetAllVisibleClusterBody { clusters: proxies };
    (StatusCode::OK, Json(body)).into_response()
}
