use actix_web::{delete, dev::PeerAddr, get, http, patch, post, put, web, HttpRequest, Responder};
use common::State;
use kube_redirect::redirect;
use tracing::instrument;

pub mod kube_redirect;

// https://kubernetes.io/docs/reference/using-api/api-concepts/#api-verbs

/// Cluster redirect
///
/// Redirect to the cluster if exists
#[utoipa::path(
    tag = "proxy_clusters",
    responses(
        (status = 200, description = "Response from remote cluster"),
        (status = 404, description = "Cluster not found or disabled."),
        (status = 500, description = "Internal server error."),
    ),
    params(
        ("ns" = String, description = "Namespace"),
        ("cluster" = String, description = "Cluster name"),
        ("path" = String, description = "Path to resource")
    )
)]
#[get("/{ns}/{cluster}/{path:.*}")]
#[instrument(name = "get_redirect", skip(data, payload))]
pub async fn get_redirect(
    req: HttpRequest,
    data: web::Data<State>,
    payload: web::Payload,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
) -> impl Responder {
    redirect(req, data, payload, method, peer_addr).await
}

/// Cluster redirect
///
/// Redirect to the cluster if exists
#[utoipa::path(
    tag = "proxy_clusters",
    responses(
        (status = 200, description = "Response from remote cluster"),
        (status = 404, description = "Cluster not found or disabled."),
        (status = 500, description = "Internal server error."),
    ),
    params(
        ("ns" = String, description = "Namespace"),
        ("cluster" = String, description = "Cluster name"),
        ("path" = String, description = "Path to resource")
    )
)]
#[post("/{ns}/{cluster}/{path:.*}")]
#[instrument(name = "post_redirect", skip(data, payload))]
pub async fn post_redirect(
    req: HttpRequest,
    data: web::Data<State>,
    payload: web::Payload,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
) -> impl Responder {
    redirect(req, data, payload, method, peer_addr).await
}

/// Cluster redirect
///
/// Redirect to the cluster if exists
#[utoipa::path(
    tag = "proxy_clusters",
    responses(
        (status = 200, description = "Response from remote cluster"),
        (status = 404, description = "Cluster not found or disabled."),
        (status = 500, description = "Internal server error."),
    ),
    params(
        ("ns" = String, description = "Namespace"),
        ("cluster" = String, description = "Cluster name"),
        ("path" = String, description = "Path to resource")
    )
)]
#[put("/{ns}/{cluster}/{path:.*}")]
#[instrument(name = "put_redirect", skip(data, payload))]
pub async fn put_redirect(
    req: HttpRequest,
    data: web::Data<State>,
    payload: web::Payload,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
) -> impl Responder {
    redirect(req, data, payload, method, peer_addr).await
}

/// Cluster redirect
///
/// Redirect to the cluster if exists
#[utoipa::path(
    tag = "proxy_clusters",
    responses(
        (status = 200, description = "Response from remote cluster"),
        (status = 404, description = "Cluster not found or disabled."),
        (status = 500, description = "Internal server error."),
    ),
    params(
        ("ns" = String, description = "Namespace"),
        ("cluster" = String, description = "Cluster name"),
        ("path" = String, description = "Path to resource")
    )
)]
#[patch("/{ns}/{cluster}/{path:.*}")]
#[instrument(name = "patch_redirect", skip(data, payload))]
pub async fn patch_redirect(
    req: HttpRequest,
    data: web::Data<State>,
    payload: web::Payload,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
) -> impl Responder {
    redirect(req, data, payload, method, peer_addr).await
}

/// Cluster redirect
///
/// Redirect to the cluster if exists
#[utoipa::path(
    tag = "proxy_clusters",
    responses(
        (status = 200, description = "Response from remote cluster"),
        (status = 404, description = "Cluster not found or disabled."),
        (status = 500, description = "Internal server error."),
    ),
    params(
        ("ns" = String, description = "Namespace"),
        ("cluster" = String, description = "Cluster name"),
        ("path" = String, description = "Path to resource")
    )
)]
#[delete("/{ns}/{cluster}/{path:.*}")]
#[instrument(name = "delete_redirect", skip(data, payload))]
pub async fn delete_redirect(
    req: HttpRequest,
    data: web::Data<State>,
    payload: web::Payload,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
) -> impl Responder {
    redirect(req, data, payload, method, peer_addr).await
}
