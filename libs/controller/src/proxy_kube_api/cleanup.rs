use common::State;
use crd::ProxyKubeApi;
use deadpool_redis::redis::cmd;
use kube::runtime::controller::Action;
use std::sync::Arc;
use tracing::{info, instrument};

use crate::error::Result;

#[instrument(skip(proxy, ctx), fields(name = %proxy.to_identifier()))]
pub async fn clean_proxy_kube_api(proxy: &ProxyKubeApi, ctx: Arc<State>) -> Result<Action> {
    info!("Cleaning ProxyKubeApi: {}", proxy.to_identifier());
    let id = proxy.to_identifier();
    {
        let mut redis_conn = ctx.get_redis_conn().await?;
        match cmd("DEL").arg(&id).query_async::<()>(&mut redis_conn).await {
            Ok(_) => info!("Successfully deleted ProxyKubeApi: {}", id),
            Err(err) => {
                info!("Failed to delete ProxyKubeApi: {}. Error: {}", id, err);
            }
        }
    }
    Ok(Action::await_change()) // No need to requeue, object is being deleted
}
