//! HTTP API surface for the proxy-auth server (actix-web).
//!
//! Hosts the management endpoints, the OIDC login/callback flow, and the
//! authenticated reverse-proxy path that forwards requests to downstream
//! Kubernetes clusters after authorizing the caller.

use crate::{
    api_doc::ApiDoc,
    base::health,
    cluster::{auth, redirect},
    visible_clusters::get_all_visible_cluster::get_all_visible_cluster,
};
use actix_web::App;
use utoipa::{openapi::OpenApi as OpenApiType, OpenApi};
use utoipa_actix_web::{scope, service_config::ServiceConfig, AppExt};

pub mod api_doc;
pub mod base;
pub mod cluster;
pub mod duration;
pub mod helper;
pub mod model;
pub mod visible_clusters;

pub fn init_base_api() -> impl FnOnce(&mut ServiceConfig) {
    |cfg: &mut ServiceConfig| {
        cfg.service(health);
    }
}

pub fn init_api() -> impl FnOnce(&mut ServiceConfig) {
    |cfg: &mut ServiceConfig| {
        cfg.service(get_all_visible_cluster);
    }
}

pub fn init_cluster_api() -> impl FnOnce(&mut ServiceConfig) {
    |cfg: &mut ServiceConfig| {
        cfg.service(auth::login::cluster_login)
            .service(auth::callback::callback_login)
            .service(auth::well_known::oauth_authorization_server)
            .service(auth::oauth::authorize::authorize)
            .service(auth::oauth::callback::callback)
            .service(auth::oauth::token::token)
            .service(auth::oauth::jwks::jwks)
            .service(redirect::get_redirect)
            .service(redirect::post_redirect)
            .service(redirect::put_redirect)
            .service(redirect::patch_redirect)
            .service(redirect::delete_redirect);
    }
}

#[must_use]
pub fn gen_openapi() -> OpenApiType {
    let mut api_doc = ApiDoc::openapi();
    api_doc.info.version = env!("CARGO_PKG_VERSION").to_string();
    let (_, api) = App::new()
        .into_utoipa_app()
        .openapi(api_doc.clone())
        .service(scope("/management").configure(init_base_api()))
        .service(scope("/api/v1").configure(init_api()))
        .service(scope("/clusters").configure(init_cluster_api()))
        .split_for_parts();
    api
}
