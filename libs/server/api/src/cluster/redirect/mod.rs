use actix_web::{delete, dev::PeerAddr, get, http, patch, post, put, web, HttpRequest, Responder};
use common::State;
use kube_redirect::redirect;
use tracing::instrument;

pub mod audit;
pub mod forwarded;
pub mod kube_redirect;
pub mod status_response;
pub mod throttle;

// https://kubernetes.io/docs/reference/using-api/api-concepts/#api-verbs

/// One handler per HTTP method, all delegating to the same `redirect`.
///
/// actix needs a distinct function per method attribute, and utoipa needs the
/// documentation attached to each of them. Writing that out by hand meant six
/// copies of the same doc block drifting apart; the macro keeps a single source
/// of truth for both.
macro_rules! redirect_handler {
    ($name:ident, $method:ident, $span:literal) => {
        /// Cluster redirect
        ///
        /// Redirect to the cluster if exists.
        #[utoipa::path(
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
        #[$method("/{ns}/{cluster}/{path:.*}")]
        #[instrument(name = $span, skip(data, payload))]
        pub async fn $name(
            req: HttpRequest,
            data: web::Data<State>,
            payload: web::Payload,
            method: http::Method,
            peer_addr: Option<PeerAddr>,
        ) -> impl Responder {
            redirect(req, data, payload, method, peer_addr).await
        }
    };
}

redirect_handler!(get_redirect, get, "get_redirect");
redirect_handler!(post_redirect, post, "post_redirect");
redirect_handler!(put_redirect, put, "put_redirect");
redirect_handler!(patch_redirect, patch, "patch_redirect");
redirect_handler!(delete_redirect, delete, "delete_redirect");
