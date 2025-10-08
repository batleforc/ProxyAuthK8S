use std::net::Ipv4Addr;

use actix_cors::Cors;
use actix_web::{dev::Service, http::header, middleware::Compress, web::Data, App, HttpServer};
use api::{api_doc::ApiDoc, init_api};
use opentelemetry_instrumentation_actix_web::{RequestMetrics, RequestTracing};
use trace::{shutdown_tracing, start_tracing};
use utoipa::OpenApi;
use utoipa_actix_web::{scope, AppExt};
use utoipa_scalar::{Scalar, Servable};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!(include_str!("banner.art"));
    let (logger, trace, meter) = start_tracing(&trace::Context {
        pod_name: std::env::var("POD_NAME").unwrap_or_else(|_| "not_a_pod".to_string()),
    });

    let state = common::State::new().await;
    let controller = controller::run(state.clone());
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
                app.wrap_fn(|req, srv| {
                    // check if the request has a request id header
                    let request_id = if let Some(request_id) = req.headers().get("x-request-id") {
                        // if it has, set the request id in the span
                        request_id
                            .to_str()
                            .unwrap_or("invalid_request_id")
                            .to_string()
                    }
                    // if not, generate a new request id
                    else {
                        uuid::Uuid::new_v4().to_string()
                    };
                    let fut = srv.call(req);
                    async move {
                        let mut res = fut.await?;
                        // set the request id in the response header
                        let headers = res.headers_mut();
                        headers.insert(
                            header::HeaderName::from_static("x-request-id"),
                            header::HeaderValue::from_str(request_id.as_str()).unwrap(),
                        );
                        tracing::info!(request_id = %request_id);
                        tracing::Span::current().record("service.request.id", &request_id);
                        Ok(res)
                    }
                })
            })
            .map(|app| app.wrap(RequestTracing::new()))
            .map(|app| app.wrap(RequestMetrics::default()))
            .map(|app| app.wrap(Compress::default()))
            .map(|app| app.wrap(cors))
            .app_data(Data::new(state.clone()))
            .service(scope("/api/v1").configure(init_api()))
            .split_for_parts();
        app.service(Scalar::with_url("/api/docs", api))
    })
    .bind((Ipv4Addr::UNSPECIFIED, 5437))?
    .shutdown_timeout(5);

    tokio::join!(controller, server.run()).1?;
    if let Err(e) = shutdown_tracing(logger, trace, meter) {
        eprintln!("Error during the shutdown of tracing: {e}");
    }
    Ok(())
}
