use std::net::Ipv4Addr;

use actix_cors::Cors;
use actix_web::{App, HttpServer, dev::Service, http::header, middleware::Compress, web::Data};
use api::{
    api_doc::ApiDoc,
    init_api, init_base_api, init_cluster_api,
    metrics::{HttpMetrics, PrometheusRegistry},
};
use common::config::CorsOrigins;
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
    // Read the whole configuration once; it is installed (and its fallback
    // warnings logged) only once tracing is up.
    let config = common::config::Config::from_env();
    let tracing_output = start_tracing(&trace::Context {
        pod_name: config.observability.pod_name.clone(),
        service_name: "proxyauthk8s".to_string(),
        log_format: match config.observability.log_format {
            common::config::LogFormat::Text => trace::LogFormat::Text,
            common::config::LogFormat::Json => trace::LogFormat::Json,
        },
        prometheus_enabled: config.observability.metrics_prometheus_enabled,
    })?;
    let config = common::config::init(config);
    // `None` when disabled: `/management/metrics` then answers 404.
    let prometheus_registry = PrometheusRegistry(tracing_output.prometheus_registry.clone());
    if prometheus_registry.0.is_some() {
        tracing::info!("Prometheus metrics served on /management/metrics");
    }
    // Created after `start_tracing` installed the global meter provider.
    let http_metrics = HttpMetrics::new(&opentelemetry::global::meter("proxyauthk8s"));

    let state = common::State::new().await?;
    // Runs next to the server: an IdP that is down at boot keeps the pod alive
    // and not-ready (`/management/ready` → 503) instead of crash-looping.
    let oidc_discovery = tokio::spawn({
        let state = state.clone();
        async move {
            state
                .discover_oidc_until_ready(
                    common::state::OIDC_DISCOVERY_RETRY_BASE,
                    common::state::OIDC_DISCOVERY_RETRY_MAX,
                )
                .await;
        }
    });
    let server_config = common::ServerConfig::new();
    let controller = controller::run(state.clone());
    let shutdown_timeout = config.server.shutdown_timeout_secs;
    tracing::info!(seconds = shutdown_timeout, "Graceful shutdown timeout");
    let mut api_doc = ApiDoc::openapi();
    api_doc.info.version = env!("CARGO_PKG_VERSION").to_string();

    // CORS: same-origin only unless `CORS_ALLOWED_ORIGINS` lists the origins
    // allowed to drive the API from a browser (`*` explicitly allows any).
    let cors_allowed_origins = config.server.cors_allowed_origins.clone();
    match &cors_allowed_origins {
        CorsOrigins::SameOriginOnly => tracing::info!(
            "CORS: CORS_ALLOWED_ORIGINS is unset, cross-origin browser requests are refused"
        ),
        CorsOrigins::Any => {
            tracing::warn!("CORS: CORS_ALLOWED_ORIGINS=*, any origin may call the API");
        }
        CorsOrigins::List(origins) => tracing::info!(?origins, "CORS: allowed origins"),
    }

    let mut server = HttpServer::new(move || {
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
            .map(|app| app.wrap(TracingLogger::default()))
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
    .shutdown_timeout(shutdown_timeout);
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
    //
    // A controller that cannot start (CRD missing, RBAC denied) is fatal: the
    // process exits non-zero so Kubernetes restarts the pod and the problem
    // shows up as CrashLoopBackOff, instead of a pod that looks healthy but
    // whose Redis is never filled.
    let result: anyhow::Result<()> = tokio::select! {
        res = controller => match res {
            Ok(()) => {
                tracing::warn!("Controller task exited, shutting down");
                Ok(())
            }
            Err(e) => Err(anyhow::Error::new(e).context("controller failed to start")),
        },
        res = server.run() => res.map_err(Into::into),
    };
    oidc_discovery.abort();
    if let Err(e) = &result {
        tracing::error!(error = format!("{e:#}"), "Server stopped with an error");
    }
    if let Err(e) = shutdown_tracing(tracing_output) {
        eprintln!("Error during the shutdown of tracing: {e}");
    }
    result
}
