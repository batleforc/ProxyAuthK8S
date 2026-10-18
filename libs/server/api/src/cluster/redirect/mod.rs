use std::sync::Arc;

use axum::{
    extract::{Path, Request, State},
    response::Response,
};
use kube_redirect::redirect;
use tracing::instrument;

use crate::helper::NsCluster;

pub mod audit;
pub mod forwarded;
pub mod kube_redirect;
pub mod status_response;
pub mod throttle;

// https://kubernetes.io/docs/reference/using-api/api-concepts/#api-verbs

/// One handler per HTTP method, all delegating to the same `redirect`.
///
/// utoipa needs the documentation attached to a function per method (the
/// `operationId`s `get_redirect`, `post_redirect`… of `swagger.json`). Writing that out by hand meant six
/// copies of the same doc block drifting apart; the macro keeps a single source
/// of truth for both.
macro_rules! redirect_handler {
    ($name:ident, $method:ident, $span:literal) => {
        /// Cluster redirect
        ///
        /// Redirect to the cluster if exists.
        #[utoipa::path(
            $method,
            path = "/clusters/{ns}/{cluster}/{path}",
            tag = "proxy_clusters",
            responses(
                (status = 200, description = "Response from remote cluster."),
                (status = 401, description = "Missing or invalid credentials."),
                (status = 403, description = "Cluster or resource not allowed for this user."),
                (status = 404, description = "Cluster not found or disabled."),
                (status = 429, description = "Rate limited, or banned after repeated failures."),
                (status = 500, description = "Internal server error."),
                (status = 503, description = "Cluster or cache unreachable."),
            ),
            params(
                ("ns" = String, description = "Namespace containing the cluster."),
                ("cluster" = String, description = "Cluster name, must match an enabled cluster in the namespace."),
                ("path" = String, description = "Corresponding path to resource given to the kube api server.")
            )
        )]
        #[instrument(name = $span, skip(data, req))]
        pub async fn $name(
            State(data): State<Arc<common::State>>,
            Path(NsCluster { ns, cluster }): Path<NsCluster>,
            req: Request,
        ) -> Response {
            redirect(data, ns, cluster, req).await
        }
    };
}

redirect_handler!(get_redirect, get, "get_redirect");
redirect_handler!(post_redirect, post, "post_redirect");
redirect_handler!(put_redirect, put, "put_redirect");
redirect_handler!(patch_redirect, patch, "patch_redirect");
redirect_handler!(delete_redirect, delete, "delete_redirect");
