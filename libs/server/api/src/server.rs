//! The HTTP server: routes, middlewares and listener.
//!
//! [`serve`] has a framework-independent signature (a bound std listener and
//! [`AppState`]) so `main.rs` and the `http_contract` suite start the exact
//! same server (RFC 0007).

use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Extension, Router,
    extract::Request,
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use axum_server::{Handle, tls_rustls::RustlsConfig};
use common::config::CorsOrigins;
use tower_http::compression::{
    CompressionLayer,
    predicate::{DefaultPredicate, Predicate},
};
use utoipa_scalar::{Scalar, Servable};

use crate::{
    app_router,
    metrics::{HttpMetrics, PrometheusRegistry, carry_matched_route, record_duration},
    root_span::root_span,
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

/// Pending connections the kernel queues before `accept` (actix's default,
/// kept). A `std::net::TcpListener` would listen with 128 only.
pub const LISTEN_BACKLOG: i32 = 2048;

/// Header lines a request may carry (`host` included); one more is a `431`.
/// actix-http's limit, frozen by the RFC 0007 contract.
pub const MAX_HEADERS: usize = 96;

/// Bytes of a request head (request line and headers); one more is a `431`.
/// actix-http's limit, frozen by the RFC 0007 contract.
pub const MAX_HEAD_BYTES: usize = 128 * 1024;

/// Bind `addr` with `SO_REUSEADDR` and a backlog of [`LISTEN_BACKLOG`], for
/// [`serve`].
pub fn bind_listener(addr: SocketAddr) -> std::io::Result<std::net::TcpListener> {
    use socket2::{Domain, Socket, Type};
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, None)?;
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(LISTEN_BACKLOG)?;
    Ok(socket.into())
}

/// Where the API reference is served, out of the OpenAPI document it shows.
pub const DOCS_PATH: &str = "/api/docs";

/// The application: every route, the API reference, and the middlewares in
/// actix's order (CORS outermost, then compression, metrics, root span).
pub fn app(app_state: &AppState) -> Router {
    let (router, api) = app_router();
    router
        .merge(Scalar::with_url(DOCS_PATH, api).custom_html(include_str!("../res/scalar.html")))
        // Inside the router, where the matched route is known.
        .route_layer(middleware::from_fn(carry_matched_route))
        // A known path with another method: actix answered 404, not 405.
        .method_not_allowed_fallback(|| async { StatusCode::NOT_FOUND })
        .layer(middleware::from_fn(root_span))
        .layer(middleware::from_fn_with_state(
            app_state.http_metrics.clone(),
            record_duration,
        ))
        .layer(Extension(app_state.prometheus_registry.clone()))
        .layer(CompressionLayer::new().compress_when(
            // Never a `101`: the upgraded connection is not a body.
            DefaultPredicate::new().and(|status: StatusCode, _: _, _: &_, _: &_| {
                status != StatusCode::SWITCHING_PROTOCOLS
            }),
        ))
        .layer(middleware::from_fn_with_state(
            app_state.cors_allowed_origins.clone(),
            cors,
        ))
        .with_state(Arc::new(app_state.state.clone()))
}

fn origin_allowed(origins: &CorsOrigins, origin: &HeaderValue) -> bool {
    match origins {
        CorsOrigins::SameOriginOnly => false,
        CorsOrigins::Any => true,
        CorsOrigins::List(origins) => origins
            .iter()
            .any(|allowed| allowed.as_bytes() == origin.as_bytes()),
    }
}

/// CORS as actix-cors applied it (RFC 0007 contract): a preflight (`OPTIONS`
/// with an `Origin` and an `Access-Control-Request-Method`) is answered here,
/// `400` for an origin that is not allowed; any other request goes on, and
/// carries `Access-Control-Allow-Origin` back when its origin is allowed. The
/// origin is echoed, never `*`; methods and headers are mirrored; a preflight
/// is cached for an hour. tower-http's `CorsLayer` would answer every
/// `OPTIONS` itself, preflight or not.
async fn cors(
    axum::extract::State(origins): axum::extract::State<CorsOrigins>,
    req: Request,
    next: Next,
) -> Response {
    let headers = req.headers();
    let origin = headers.get(header::ORIGIN).cloned();
    let allowed = origin
        .as_ref()
        .filter(|origin| origin_allowed(&origins, origin))
        .cloned();
    let requested_method = headers.get(header::ACCESS_CONTROL_REQUEST_METHOD).cloned();

    if req.method() == Method::OPTIONS
        && origin.is_some()
        && let Some(requested_method) = requested_method
    {
        let Some(origin) = allowed else {
            return (
                StatusCode::BAD_REQUEST,
                "Origin is not allowed to make this request",
            )
                .into_response();
        };
        let requested_headers = headers.get(header::ACCESS_CONTROL_REQUEST_HEADERS).cloned();
        let mut response = StatusCode::OK.into_response();
        let out = response.headers_mut();
        out.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        out.insert(header::ACCESS_CONTROL_ALLOW_METHODS, requested_method);
        if let Some(requested_headers) = requested_headers {
            out.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, requested_headers);
        }
        out.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from(3600));
        out.insert(
            header::VARY,
            HeaderValue::from_static(
                "origin, access-control-request-method, access-control-request-headers",
            ),
        );
        return response;
    }

    let mut response = next.run(req).await;
    if let Some(origin) = allowed {
        let out = response.headers_mut();
        out.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        out.append(header::VARY, HeaderValue::from_static("origin"));
    }
    response
}

/// Serve the API on `listener` until the server stops: on `SIGTERM` or
/// `SIGINT`, in-flight requests get `shutdown_timeout_secs` to finish.
pub async fn serve(listener: std::net::TcpListener, app_state: AppState) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let app = app(&app_state).into_make_service_with_connect_info::<SocketAddr>();
    let handle = Handle::new();
    tokio::spawn(shutdown_on_signal(
        handle.clone(),
        Duration::from_secs(app_state.shutdown_timeout_secs),
    ));
    match app_state.tls {
        Some(mut tls) => {
            // h2 and http/1.1, as actix's `listen_rustls_0_23` announced them:
            // kubectl's requests go h2, an upgrade stays on http/1.1.
            tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
            let mut server =
                axum_server::from_tcp_rustls(listener, RustlsConfig::from_config(Arc::new(tls)))?
                    .handle(handle);
            set_limits(server.http_builder());
            server.serve(app).await
        }
        None => {
            let mut server = axum_server::from_tcp(listener)?.handle(handle);
            set_limits(server.http_builder());
            server.serve(app).await
        }
    }
}

/// The HTTP/1 behaviour of actix (RFC 0007 contract): its limits, and a
/// request still answered after the client closed its sending side.
fn set_limits(
    builder: &mut hyper_util::server::conn::auto::Builder<hyper_util::rt::TokioExecutor>,
) {
    builder
        .http1()
        .max_headers(MAX_HEADERS)
        .max_buf_size(MAX_HEAD_BYTES)
        .half_close(true);
    // hyper's h2 default is 16 KiB of headers; keep the h1 head budget so a
    // large bearer token does not fail on h2 only.
    builder
        .http2()
        .max_header_list_size(u32::try_from(MAX_HEAD_BYTES).unwrap_or(u32::MAX));
}

/// Resolves on SIGINT or SIGTERM.
pub async fn shutdown_signal() {
    let interrupt = tokio::signal::ctrl_c();
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = interrupt => {}
        () = terminate => {}
    }
}

async fn shutdown_on_signal(handle: Handle<SocketAddr>, timeout: Duration) {
    shutdown_signal().await;
    tracing::info!(
        seconds = timeout.as_secs(),
        "Shutting down, waiting for in-flight requests"
    );
    handle.graceful_shutdown(Some(timeout));
}
