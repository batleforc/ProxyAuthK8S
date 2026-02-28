use common::{traits::ObjectRedis, State};
use crd::{status::ProxyKubeApiStatus, ProxyKubeApi};
use deadpool_redis::redis::cmd;
use kube::{api::PatchParams, runtime::controller::Action, Api};
use std::sync::Arc;
use tracing::{info, instrument};

use crate::error::{ControllerError, Result};

#[instrument(skip(proxy, ctx), fields(name = %proxy.to_identifier()))]
pub async fn reconcile_proxy_kube_api(proxy: &ProxyKubeApi, ctx: Arc<State>) -> Result<Action> {
    info!("Reconciling ProxyKubeApi: {}", proxy.to_identifier());
    let id = proxy.to_identifier();
    let path = proxy.to_path();
    let ps: PatchParams = PatchParams::apply("proxy-kube-api-controller").force();
    let mut proxy_cloned = proxy.clone();
    let metadata = proxy_cloned.clone().metadata;
    let ns = metadata.namespace.as_deref().unwrap_or("default");
    let name = metadata.name.as_deref().unwrap_or("unknown");
    let proxys: Api<ProxyKubeApi> = Api::namespaced(ctx.client.clone(), ns);

    // validate the proxy configuration, if it's invalid, set the status to not reachable with the error message and requeue after 5 minutes
    let mut new_status = match proxy_cloned.validate() {
        Ok(_) => ProxyKubeApiStatus::new(false, None, None),
        Err(e) => {
            tracing::error!(
                "Failed to validate ProxyKubeApi {}: {}",
                proxy.to_identifier(),
                e
            );
            ProxyKubeApiStatus::new(
                false,
                None,
                Some(format!("Failed to validate proxy configuration: {}", e)),
            )
        }
    };

    if new_status.error.is_none() {
        new_status = match proxy.clone().is_reachable(ctx.clone()).await {
            Ok(reachable) => {
                if reachable {
                    tracing::info!("ProxyKubeApi {} is reachable", proxy.to_identifier());
                    ProxyKubeApiStatus::new(true, Some(format!("/clusters/{}", path)), None)
                } else {
                    tracing::warn!("ProxyKubeApi {} is not reachable", proxy.to_identifier());
                    ProxyKubeApiStatus::new(
                        false,
                        None,
                        Some("Target service is not reachable".to_string()),
                    )
                }
            }
            Err(e) => {
                tracing::error!(
                    "Failed to check if ProxyKubeApi {} is reachable: {}",
                    proxy.to_identifier(),
                    e
                );
                ProxyKubeApiStatus::new(
                    false,
                    None,
                    Some(format!(
                        "Failed to check if target service is reachable: {}",
                        e
                    )),
                )
            }
        };
    }
    info!(
        "Updating status of ProxyKubeApi {}: reachable={}, error={:?}",
        proxy.to_identifier(),
        new_status.exposed,
        new_status.error
    );
    let mut redis_conn = ctx.get_redis_conn().await?;
    proxy_cloned.status = Some(new_status.clone());
    let proxy_json = proxy_cloned.to_json();
    match cmd("SET")
        .arg(&id)
        .arg(&proxy_json)
        .query_async::<()>(&mut redis_conn)
        .await
    {
        Ok(_) => info!("Successfully upsert ProxyKubeApi: {}", id),
        Err(err) => {
            info!("Failed to upsert ProxyKubeApi: {}. Error: {}", id, err);
        }
    }
    drop(redis_conn);

    let patch = new_status.get_patch();
    let _ = proxys
        .patch_status(name, &ps, &patch)
        .await
        .map_err(ControllerError::Kube)?;
    if new_status.error.is_some() {
        Ok(Action::requeue(std::time::Duration::from_secs(5 * 60)))
    } else {
        Ok(Action::requeue(std::time::Duration::from_secs(60 * 60)))
    }
}
