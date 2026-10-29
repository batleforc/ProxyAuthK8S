//! HTTP API surface for the proxy-auth server (axum).
//!
//! Hosts the management endpoints, the OIDC login/callback flow, and the
//! authenticated reverse-proxy path that forwards requests to downstream
//! Kubernetes clusters after authorizing the caller.

use std::sync::Arc;

use crate::{
    api_doc::ApiDoc,
    cluster::{auth, redirect},
};
use axum::{Router, extract::DefaultBodyLimit, routing::get};
use utoipa::{OpenApi, openapi::OpenApi as OpenApiType};
use utoipa_axum::{router::OpenApiRouter, routes};

pub mod agent;
pub mod api_doc;
pub mod base;
pub mod cluster;
pub mod duration;
pub mod helper;
pub mod metrics;
pub mod model;
#[cfg(feature = "server")]
mod root_span;
#[cfg(feature = "server")]
pub mod server;
pub mod upstream_probe;
pub mod visible_clusters;

/// The state every handler shares.
pub type SharedState = Arc<common::State>;

/// `/management`: health, readiness and the Prometheus endpoint.
#[must_use]
pub fn management_router() -> OpenApiRouter<SharedState> {
    OpenApiRouter::new()
        .routes(routes!(base::health))
        .routes(routes!(base::ready))
        .routes(routes!(metrics::prometheus_metrics))
}

/// `/api/v1`: what the dashboard calls.
#[must_use]
pub fn api_router() -> OpenApiRouter<SharedState> {
    OpenApiRouter::new().routes(routes!(
        visible_clusters::get_all_visible_cluster::get_all_visible_cluster
    ))
}

/// `/clusters`: the cluster logins, the mediated OAuth server and the proxy.
#[must_use]
pub fn cluster_router() -> OpenApiRouter<SharedState> {
    let token = OpenApiRouter::new()
        .routes(routes!(auth::oauth::token::token))
        .layer(DefaultBodyLimit::max(auth::oauth::token::TOKEN_FORM_LIMIT));
    OpenApiRouter::new()
        .routes(routes!(auth::login::cluster_login))
        .routes(routes!(auth::callback::callback_login))
        .routes(routes!(auth::well_known::oauth_authorization_server))
        .routes(routes!(auth::oauth::authorize::authorize))
        .routes(routes!(auth::oauth::callback::callback))
        .routes(routes!(auth::oauth::jwks::jwks))
        .merge(token)
        // The catch-all is routed here and documented in `ApiDoc`: the
        // OpenAPI path is `{path}`, the router's `{*path}`.
        .route(
            "/{ns}/{cluster}/{*path}",
            get(redirect::get_redirect)
                .post(redirect::post_redirect)
                .put(redirect::put_redirect)
                .patch(redirect::patch_redirect)
                .delete(redirect::delete_redirect),
        )
}

/// Every route, and the OpenAPI document they describe.
pub fn app_router() -> (Router<SharedState>, OpenApiType) {
    let mut api_doc = ApiDoc::openapi();
    api_doc.info.version = env!("CARGO_PKG_VERSION").to_string();
    OpenApiRouter::with_openapi(api_doc)
        .nest("/management", management_router())
        .nest("/api/v1", api_router())
        .nest("/clusters", cluster_router())
        .split_for_parts()
}

#[must_use]
pub fn gen_openapi() -> OpenApiType {
    app_router().1
}
