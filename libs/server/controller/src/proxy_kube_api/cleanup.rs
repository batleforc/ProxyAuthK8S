use common::State;
use crd::ProxyKubeApi;
use kube::{Resource as _, ResourceExt as _, runtime::controller::Action};
use std::sync::Arc;
use tracing::{info, instrument};
use weebo_kit_runtime::events::{CLEANUP, announce_change};

use crate::error::{ControllerError, Result};
use crate::proxy_kube_api::{REDIS_PREFIX, recorder};
use crate::reason::ReasonCode;

/// Drop a deleted cluster from Redis. Closed (RFC 0011, decision 25): the
/// proxy routes from Redis and the key has no TTL, so a failed `DEL` (or
/// index removal) fails the cleanup and the finalizer keeps the CR until it
/// succeeds, instead of leaving a deleted cluster routable.
#[instrument(skip(proxy, ctx), fields(name = %proxy.to_identifier()))]
pub async fn clean_proxy_kube_api(proxy: &ProxyKubeApi, ctx: Arc<State>) -> Result<Action> {
    info!("Cleaning ProxyKubeApi: {}", proxy.to_identifier());
    let id = proxy.to_identifier();

    // Both prefixes (RFC 0014, decision 42): a cluster deleted mid-switch may
    // still sit under the one it is leaving.
    for prefix in [REDIS_PREFIX, crd::REDIS_TRUSTED_PREFIX] {
        let key = format!("{prefix}:{}", proxy.to_path());
        ctx.delete_key(&key).await.map_err(ControllerError::Redis)?;
        // Drop the index entry too, otherwise the dashboard keeps listing a
        // cluster whose configuration is gone.
        ctx.index_remove(prefix, &key)
            .await
            .map_err(ControllerError::Redis)?;
    }
    info!("Successfully deleted ProxyKubeApi: {}", id);

    // Drop this replica's cached upstream client/TLS configuration. Other
    // replicas let theirs expire (`PROXY_UPSTREAM_CLIENT_TTL`): a deleted proxy
    // is no longer routable, so a lingering entry is only memory.
    common::upstream_cache::evict_proxy(&proxy.namespace().unwrap_or_default(), &proxy.name_any());

    announce_change(
        recorder(&ctx),
        &proxy.object_ref(&()),
        ReasonCode::Deleted,
        CLEANUP,
        "The cluster was removed from the proxy",
    )
    .await;
    Ok(Action::await_change()) // No need to requeue, object is being deleted
}
