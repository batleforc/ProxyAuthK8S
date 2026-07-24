use common::State;
use crd::ProxyKubeApi;
use kube::runtime::controller::Action;
use std::sync::Arc;
use tracing::{info, instrument, warn};

use crate::error::Result;
use crate::proxy_kube_api::REDIS_PREFIX;

#[instrument(skip(proxy, ctx), fields(name = %proxy.to_identifier()))]
pub async fn clean_proxy_kube_api(proxy: &ProxyKubeApi, ctx: Arc<State>) -> Result<Action> {
    info!("Cleaning ProxyKubeApi: {}", proxy.to_identifier());
    let id = proxy.to_identifier();

    match ctx.delete_key(&id).await {
        Ok(_) => info!("Successfully deleted ProxyKubeApi: {}", id),
        Err(err) => {
            info!("Failed to delete ProxyKubeApi: {}. Error: {}", id, err);
        }
    }

    // Drop the index entry too, otherwise the dashboard keeps listing a cluster
    // whose configuration is gone.
    if let Err(err) = ctx.index_remove(REDIS_PREFIX, &id).await {
        warn!(
            "Failed to remove ProxyKubeApi from the index: {}. Error: {}",
            id, err
        );
    }

    Ok(Action::await_change()) // No need to requeue, object is being deleted
}
