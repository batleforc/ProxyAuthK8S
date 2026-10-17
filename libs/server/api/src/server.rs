//! The HTTP server: routes, middlewares and listener.
//!
//! [`serve`] has a framework-independent signature (a bound std listener and
//! [`AppState`]) so `main.rs` and the `http_contract` suite start the exact
//! same server; RFC 0007 rewrites its body on axum behind that signature.

use actix_cors::Cors;
use actix_web::{App, HttpServer, dev::Service, http::header, middleware::Compress, web::Data};
use common::config::CorsOrigins;
use tracing_actix_web::{RequestId, TracingLogger};
use utoipa::OpenApi;
use utoipa_actix_web::{AppExt, scope};
use utoipa_scalar::{Scalar, Servable};

use crate::{
    api_doc::ApiDoc,
    init_api, init_base_api, init_cluster_api,
    metrics::{HttpMetrics, PrometheusRegistry},
    root_span::OtelRootSpanBuilder,
};

/// Everything the server needs besides its listener.
#[derive(Clone)]
pub struct AppState {
    pub state: common::State,
    /// `None` inside: `/management/metrics` answers 404.
    pub prometheus_registry: PrometheusRegistry,
    pub http_metrics: HttpMetrics,
    pub cors_allowed_origins: CorsOrigins,
    /// TLS for the listener; `None` serves plain HTTP.
    pub tls: Option<rustls::ServerConfig>,
    /// Seconds in-flight requests get to finish on shutdown.
    pub shutdown_timeout_secs: u64,
}

/// Pending connections the kernel queues before `accept`: actix's own
/// default. A `std::net::TcpListener` would listen with 128 only.
pub const LISTEN_BACKLOG: i32 = 2048;

/// Bind `addr` the way actix's `bind` did (`SO_REUSEADDR`, backlog of
/// [`LISTEN_BACKLOG`]), for [`serve`].
pub fn bind_listener(addr: std::net::SocketAddr) -> std::io::Result<std::net::TcpListener> {
    use socket2::{Domain, Socket, Type};
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, None)?;
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(LISTEN_BACKLOG)?;
    Ok(socket.into())
}

/// Serve the API on `listener` until the server stops.
pub async fn serve(listener: std::net::TcpListener, app_state: AppState) -> std::io::Result<()> {
    let AppState {
        state,
        prometheus_registry,
        http_metrics,
        cors_allowed_origins,
        tls,
        shutdown_timeout_secs,
    } = app_state;
    let mut api_doc = ApiDoc::openapi();
    api_doc.info.version = env!("CARGO_PKG_VERSION").to_string();

    let server = HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);
        let cors = match &cors_allowed_origins {
            CorsOrigins::SameOriginOnly => cors,
            CorsOrigins::Any => cors.allow_any_origin(),
            CorsOrigins::List(origins) => origins
                .iter()
                .fold(cors, |cors, origin| cors.allowed_origin(origin)),
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
            .map(|app| app.wrap(TracingLogger::<OtelRootSpanBuilder>::new()))
            .map(|app| app.wrap(http_metrics.clone()))
            .map(|app| app.wrap(Compress::default()))
            .map(|app| app.wrap(cors))
            .app_data(Data::new(state.clone()))
            .app_data(Data::new(prometheus_registry.clone()))
            .service(scope("/management").configure(init_base_api()))
            .service(scope("/api/v1").configure(init_api()))
            .service(scope("/clusters").configure(init_cluster_api()))
            .split_for_parts();
        app.service(
            Scalar::with_url("/api/docs", api).custom_html(include_str!("../res/scalar.html")),
        )
    })
    .shutdown_timeout(shutdown_timeout_secs);

    let server = match tls {
        Some(tls) => server.listen_rustls_0_23(listener, tls)?,
        None => server.listen(listener)?,
    };
    server.run().await
}
