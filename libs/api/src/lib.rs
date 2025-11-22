use crate::{
    api::get_all_visible_cluster::get_all_visible_cluster,
    api_doc::ApiDoc,
    base::health,
    cluster::{base::base_cluster, redirect},
};
use actix_web::App;
use utoipa::{openapi::OpenApi as OpenApiType, OpenApi};
use utoipa_actix_web::{scope, service_config::ServiceConfig, AppExt};

pub mod api;
pub mod api_doc;
pub mod base;
pub mod cluster;
pub mod helper;
pub mod model;

pub fn init_api() -> impl FnOnce(&mut ServiceConfig) {
    |cfg: &mut ServiceConfig| {
        cfg.service(health).service(get_all_visible_cluster);
    }
}

pub fn init_cluster_api() -> impl FnOnce(&mut ServiceConfig) {
    |cfg: &mut ServiceConfig| {
        cfg.service(base_cluster)
            .service(redirect::get_redirect)
            .service(redirect::post_redirect)
            .service(redirect::put_redirect)
            .service(redirect::patch_redirect)
            .service(redirect::delete_redirect);
    }
}

pub fn gen_openapi() -> OpenApiType {
    let mut api_doc = ApiDoc::openapi();
    api_doc.info.version = env!("CARGO_PKG_VERSION").to_string();
    let (_, api) = App::new()
        .into_utoipa_app()
        .openapi(api_doc.clone())
        .service(scope("/api/v1").configure(init_api()))
        .service(scope("/clusters").configure(init_cluster_api()))
        .split_for_parts();
    api
}
