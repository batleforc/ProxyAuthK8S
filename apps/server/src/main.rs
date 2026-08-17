use std::net::Ipv4Addr;

use actix_cors::Cors;
use actix_web::{App, HttpServer, dev::Service, http::header, middleware::Compress, web::Data};
use api::{api_doc::ApiDoc, init_api, init_base_api, init_cluster_api};
use trace::{shutdown_tracing, start_tracing};
use tracing_actix_web::{RequestId, TracingLogger};
use utoipa::OpenApi;
use utoipa_actix_web::{AppExt, scope};
use utoipa_scalar::{Scalar, Servable};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install default CryptoProvider");
    println!(include_str!("../../../.docs/public/banner.art"));
    let tracing_output = start_tracing(&trace::Context {
        pod_name: std::env::var("POD_NAME").unwrap_or_else(|_| "not_a_pod".to_string()),
        service_name: "proxyauthk8s".to_string(),
    });

    let state = common::State::new().await?;
    let server_config = common::ServerConfig::new();
    let controller = controller::run(state.clone());
    let mut api_doc = ApiDoc::openapi();
    api_doc.info.version = env!("CARGO_PKG_VERSION").to_string();

    // CORS: permissive by default (kept for backward compatibility — the API is
    // Bearer-authenticated, so a browser never auto-attaches credentials). Set
    // `CORS_ALLOWED_ORIGINS` to a comma-separated allow-list to restrict which
    // origins may drive the API from a browser.
    let cors_allowed_origins: Option<Vec<String>> = std::env::var("CORS_ALLOWED_ORIGINS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(|origin| origin.trim().to_string())
                .filter(|origin| !origin.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|origins| !origins.is_empty());

    let mut server = HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);
        let cors = match &cors_allowed_origins {
            Some(origins) => origins
                .iter()
                .fold(cors, |cors, origin| cors.allowed_origin(origin)),
            None => cors.allow_any_origin(),
        };
        let (app, api) = App::new()
            .into_utoipa_app()
            .openapi(api_doc.clone())
            .map(|app| {
                app.wrap_fn(|mut req, srv| {
                    let request_id_asc = req.extract::<RequestId>();
                    let fut = srv.call(req);
                    async move {
                        let mut res = fut.await?;
                        // Never panic a worker over a response decoration: if the
                        // request id is unavailable or unrepresentable, ship the
                        // response without the header.
                        if let Ok(request_id) = request_id_asc.await {
                            let request_id: RequestId = request_id;
                            if let Ok(value) =
                                header::HeaderValue::from_str(&format!("{request_id}"))
                            {
                                res.headers_mut()
                                    .insert(header::HeaderName::from_static("x-request-id"), value);
                            }
                        }
                        Ok(res)
                    }
                })
            })
            .map(|app| app.wrap(TracingLogger::default()))
            .map(|app| app.wrap(Compress::default()))
            .map(|app| app.wrap(cors))
            .app_data(Data::new(state.clone()))
            .service(scope("/management").configure(init_base_api()))
            .service(scope("/api/v1").configure(init_api()))
            .service(scope("/clusters").configure(init_cluster_api()))
            .split_for_parts();
        app.service(Scalar::with_url("/api/docs", api))
    })
    .shutdown_timeout(5);
    if server_config.https {
        let tls_config = server_config.rustls_config()?;
        server =
            server.bind_rustls_0_23((Ipv4Addr::UNSPECIFIED, server_config.port), tls_config)?;
    } else {
        server = server.bind((Ipv4Addr::UNSPECIFIED, server_config.port))?;
    }

    // `select!` rather than `join!`: if the HTTP server exits on its own (bind
    // loss / internal error) we must stop instead of blocking forever on the
    // controller future, and vice-versa on shutdown signal.
    tokio::select! {
        () = controller => {
            eprintln!("Controller task exited, shutting down");
        }
        res = server.run() => {
            res?;
        }
    }
    if let Err(e) = shutdown_tracing(tracing_output) {
        eprintln!("Error during the shutdown of tracing: {e}");
    }
    Ok(())
}
