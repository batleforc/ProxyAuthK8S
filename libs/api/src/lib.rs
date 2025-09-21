use crate::{api_doc::ApiDoc, base::health};
use actix_web::App;
use utoipa::{openapi::OpenApi as OpenApiType, OpenApi};
use utoipa_actix_web::{scope, service_config::ServiceConfig, AppExt};

pub mod api_doc;
pub mod base;

pub fn init_api() -> impl FnOnce(&mut ServiceConfig) {
    |cfg: &mut ServiceConfig| {
        cfg.service(health);
    }
}

pub fn gen_openapi() -> OpenApiType {
    let mut api_doc = ApiDoc::openapi();
    api_doc.info.version = env!("CARGO_PKG_VERSION").to_string();
    let (_, api) = App::new()
        .into_utoipa_app()
        .openapi(api_doc.clone())
        .service(scope("/api/v1").configure(init_api()))
        .split_for_parts();
    api
}
