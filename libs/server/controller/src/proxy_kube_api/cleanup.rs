use common::State;
use crd::ProxyKubeApi;
use kube::{ResourceExt as _, runtime::controller::Action};
use std::sync::Arc;
use tracing::{info, instrument, warn};

use crate::error::Result;
use crate::proxy_kube_api::REDIS_PREFIX;
use crate::proxy_kube_api::reconcile::retry_counter_key;

#[instrument(skip(proxy, ctx), fields(name = %proxy.to_identifier()))]
pub async fn clean_proxy_kube_api(proxy: &ProxyKubeApi, ctx: Arc<State>) -> Result<Action> {
    info!("Cleaning ProxyKubeApi: {}", proxy.to_identifier());
    let id = proxy.to_identifier();

    match ctx.delete_key(&id).await {
        Ok(()) => info!("Successfully deleted ProxyKubeApi: {}", id),
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

    // Drop the backoff counter as well: a proxy recreated under the same name
    // would otherwise inherit its predecessor's failure count (for up to 24h).
    if let Err(err) = ctx.delete_key(&retry_counter_key(&id)).await {
        warn!(
            "Failed to reset the retry counter of ProxyKubeApi: {}. Error: {}",
            id, err
        );
    }

    // Drop this replica's cached upstream client/TLS configuration. Other
    // replicas let theirs expire (`PROXY_UPSTREAM_CLIENT_TTL`): a deleted proxy
    // is no longer routable, so a lingering entry is only memory.
    common::upstream_cache::evict_proxy(&proxy.namespace().unwrap_or_default(), &proxy.name_any());

    Ok(Action::await_change()) // No need to requeue, object is being deleted
}
