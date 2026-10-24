//! `proxyauthk8s-agent` (RFC 0012 §6.5): the agent, and its probes and
//! metrics on `AGENT_PROBE_PORT` (default `9090`); `publish-jwks`, the
//! `jwksPublisher` CronJob's one run.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use agent::{Config, Health};
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("Failed to install default CryptoProvider"))?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    // The `jwksPublisher` CronJob (§6.5): one copy, then exit.
    if std::env::args().nth(1).as_deref() == Some("publish-jwks") {
        let written = agent::jwks::publish(|name| std::env::var(name).ok())
            .await
            .map_err(anyhow::Error::msg)?;
        tracing::info!(written, "JWKS published");
        return Ok(());
    }
    let config =
        Config::from_lookup(|name| std::env::var(name).ok()).map_err(anyhow::Error::msg)?;
    let port = std::env::var("AGENT_PROBE_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(9090u16);
    let health = Arc::new(Health::default());
    let probes = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/readyz", get(ready))
        .route(
            "/metrics",
            get(|State(health): State<Arc<Health>>| async move { health.render() }),
        )
        .with_state(Arc::clone(&health));
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port))).await?;
    tracing::info!(cluster = %config.cluster, %port, "agent starting");
    tokio::select! {
        served = axum::serve(listener, probes) => served?,
        () = agent::run(config, health) => {}
        _ = tokio::signal::ctrl_c() => {}
        () = terminate() => {}
    }
    Ok(())
}

/// Ready while a tunnel is open (§6.5).
async fn ready(State(health): State<Arc<Health>>) -> StatusCode {
    if health.tunnel_up.load(Ordering::Relaxed) {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn terminate() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut signal) => {
            signal.recv().await;
        }
        Err(_) => std::future::pending().await,
    }
}
