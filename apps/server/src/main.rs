use std::net::{Ipv4Addr, SocketAddr};

use api::{
    metrics::{HttpMetrics, PrometheusRegistry},
    server::{AppState, bind_listener, serve},
};
use common::config::CorsOrigins;
use trace::{shutdown_tracing, start_tracing};

/// How long the HTTP server's shutdown waits for the controller to stop and
/// release the leader Lease.
const CONTROLLER_STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("Failed to install default CryptoProvider"))?;
    println!(include_str!("../../../docs/public/banner.art"));
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
        prometheus_enabled: config.observability.metrics_prometheus_enabled
            || config.observability.weebo_metrics_addr.is_some(),
    })?;
    let config = common::config::init(config);
    // `None` when disabled: `/management/metrics` then answers 404.
    let prometheus_registry = PrometheusRegistry(tracing_output.prometheus_registry.clone());
    if prometheus_registry.0.is_some() {
        tracing::info!("Prometheus metrics served on /management/metrics");
    }
    // `WEEBO_METRICS_ADDR`: the same registry on `/metrics`, a listener of its
    // own (what weebo-kit-perf scrapes, RFC 0011 decision 42).
    let metrics_listener = match config.observability.weebo_metrics_addr {
        Some(addr) => {
            let listener = tokio::net::TcpListener::bind(addr).await?;
            tracing::info!(%addr, "Prometheus metrics also served on /metrics");
            Some(tokio::spawn(api::metrics::serve_metrics(
                listener,
                prometheus_registry.clone(),
            )))
        }
        None => None,
    };
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
    // A task of its own: on SIGTERM the HTTP server may finish first, and the
    // controller must still get to release the leader Lease (RFC 0011,
    // decision 36) instead of being dropped mid-shutdown.
    let mut controller = tokio::spawn(controller::run(state.clone()));
    let shutdown_timeout = config.server.shutdown_timeout_secs;
    tracing::info!(seconds = shutdown_timeout, "Graceful shutdown timeout");
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

    let tls = if server_config.https {
        Some(server_config.rustls_config()?)
    } else {
        None
    };
    let listener = bind_listener(SocketAddr::from((
        Ipv4Addr::UNSPECIFIED,
        server_config.port,
    )))?;
    let server = serve(
        listener,
        AppState {
            state: state.clone(),
            prometheus_registry,
            http_metrics,
            cors_allowed_origins,
            tls,
            shutdown_timeout_secs: shutdown_timeout,
        },
    );

    // `select!` rather than `join!`: if the HTTP server exits on its own (bind
    // loss / internal error) we must stop instead of blocking forever on the
    // controller future, and vice-versa on shutdown signal.
    //
    // A controller that cannot start (CRD missing, RBAC denied) is fatal: the
    // process exits non-zero so Kubernetes restarts the pod and the problem
    // shows up as CrashLoopBackOff, instead of a pod that looks healthy but
    // whose Redis is never filled.
    let result: anyhow::Result<()> = tokio::select! {
        res = &mut controller => match res {
            Ok(Ok(())) => {
                tracing::warn!("Controller task exited, shutting down");
                Ok(())
            }
            Ok(Err(e)) => Err(anyhow::Error::new(e).context("controller failed to start")),
            Err(e) => Err(anyhow::Error::new(e).context("controller task panicked")),
        },
        res = server => {
            // The controller saw the same signal: give it a moment to stop and
            // release the Lease.
            if tokio::time::timeout(CONTROLLER_STOP_TIMEOUT, &mut controller).await.is_err() {
                tracing::warn!("Controller did not stop in time");
                controller.abort();
            }
            res.map_err(Into::into)
        }
    };
    oidc_discovery.abort();
    if let Some(metrics) = metrics_listener {
        metrics.abort();
    }
    if let Err(e) = &result {
        tracing::error!(error = format!("{e:#}"), "Server stopped with an error");
    }
    if let Err(e) = shutdown_tracing(tracing_output) {
        eprintln!("Error during the shutdown of tracing: {e}");
    }
    result
}
