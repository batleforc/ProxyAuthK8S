use std::net::Ipv4Addr;

use actix_cors::Cors;
use actix_web::{dev::Service, http::header, middleware::Compress, App, HttpServer};
use api::{api_doc::ApiDoc, init_api};
use trace::{shutdown_tracing, start_tracing};
use tracing_actix_web::{RequestId, TracingLogger};
use utoipa::OpenApi;
use utoipa_actix_web::{scope, AppExt};
use utoipa_scalar::{Scalar, Servable};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!(include_str!("banner.art"));
    let (logger, trace, meter) = start_tracing(&trace::Context {
        pod_name: std::env::var("POD_NAME").unwrap_or_else(|_| "not_a_pod".to_string()),
    });

    let mut api_doc = ApiDoc::openapi();
    api_doc.info.version = env!("CARGO_PKG_VERSION").to_string();
    let server = HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);
        let (app, api) = App::new()
            .into_utoipa_app()
            .openapi(api_doc.clone())
            .map(|app| {
                app.wrap_fn(|mut req, srv| {
                    let request_id_asc = req.extract::<RequestId>();
                    let fut = srv.call(req);
                    async move {
                        let mut res = fut.await?;
                        let request_id: RequestId = request_id_asc.await.unwrap();
                        let request_id_str = format!("{}", request_id);
                        let headers = res.headers_mut();
                        headers.insert(
                            header::HeaderName::from_static("x-request-id"),
                            header::HeaderValue::from_str(request_id_str.as_str()).unwrap(),
                        );
                        Ok(res)
                    }
                })
            })
            .map(|app| app.wrap(TracingLogger::default()))
            .map(|app| app.wrap(Compress::default()))
            .map(|app| app.wrap(cors))
            .service(scope("/api/v1").configure(init_api()))
            .split_for_parts();
        app.service(Scalar::with_url("/api/docs", api))
    })
    .bind((Ipv4Addr::UNSPECIFIED, 5437))?
    .shutdown_timeout(5);

    tokio::join!(server.run()).0?;
    if let Err(e) = shutdown_tracing(logger, trace, meter) {
        eprintln!("Error during the shutdown of tracing: {e}");
    }
    Ok(())
}
