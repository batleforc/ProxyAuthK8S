//! The agent gateway of RFC 0012 (decision 26): the tunnels agents hold, and
//! the `CONNECT` the proxy reaches them by.

pub mod ca;
pub mod connect;
pub mod enroll;
pub mod listener;
pub mod presence;
pub mod registry;

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use common::State;
use common::config::AgentGatewaySettings;
use tokio::net::TcpListener;
use tracing::info;

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("AGENT_GATEWAY_TOKEN is required by the gateway role (T18)")]
    NoToken,
    #[error(transparent)]
    Ca(#[from] ca::CaError),
    #[error("agent gateway listener: {0}")]
    Io(#[from] std::io::Error),
}

/// Run the agent gateway until `shutdown`: the agents' listener on `port`,
/// `CONNECT` on `connect_port`, the presences. On shutdown, this pod leaves
/// every presence set and closes its tunnels, so agents and proxy move on.
pub async fn run(
    state: State,
    settings: AgentGatewaySettings,
    pod: String,
    shutdown: impl Future<Output = ()>,
) -> Result<(), GatewayError> {
    let token = settings.token.clone().ok_or(GatewayError::NoToken)?;
    let ca = Arc::new(ca::load_or_create(state.client.clone(), &settings.namespace).await?);
    let agents = TcpListener::bind((Ipv4Addr::UNSPECIFIED, settings.port)).await?;
    let connect = TcpListener::bind((Ipv4Addr::UNSPECIFIED, settings.connect_port)).await?;
    info!(
        port = settings.port,
        connect_port = settings.connect_port,
        "agent gateway listening"
    );
    let registry = registry::TunnelRegistry::new();
    let gateway = listener::Gateway::new(state, Arc::clone(&registry), ca, settings, pod);
    // The peer address names the enrolling client (`TRUSTED_PROXY_COUNT`).
    let app =
        listener::router(Arc::clone(&gateway)).into_make_service_with_connect_info::<SocketAddr>();
    let result = tokio::select! {
        served = axum::serve(agents, app) => served.map_err(GatewayError::from),
        () = connect::serve(connect, connect::ConnectService::new(registry, Some(token))) => Ok(()),
        () = presence::run(Arc::clone(&gateway)) => Ok(()),
        () = shutdown => Ok(()),
    };
    presence::withdraw(&gateway).await;
    result
}
