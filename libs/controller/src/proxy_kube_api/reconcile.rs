use common::State;
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
    let proxy_cloned = proxy.clone();
    let metadata = proxy_cloned.clone().metadata;
    let ns = metadata.namespace.as_deref().unwrap_or("default");
    let name = metadata.name.as_deref().unwrap_or("unknown");
    let proxys: Api<ProxyKubeApi> = Api::namespaced(ctx.client.clone(), &ns);

    match proxy.clone().is_reachable(ctx.clone()).await {
        Ok(reachable) => {
            if reachable {
                tracing::info!("ProxyKubeApi {} is reachable", proxy.to_identifier());
            } else {
                tracing::warn!("ProxyKubeApi {} is not reachable", proxy.to_identifier());
                let new_status = ProxyKubeApiStatus::new(
                    false,
                    None,
                    Some("Target service is not reachable".to_string()),
                )
                .get_patch();
                let _ = proxys
                    .patch_status(name, &ps, &new_status)
                    .await
                    .map_err(ControllerError::Kube)?;
                return Ok(Action::requeue(std::time::Duration::from_secs(5 * 60)));
            }
        }
        Err(e) => {
            tracing::error!(
                "Failed to check if ProxyKubeApi {} is reachable: {}",
                proxy.to_identifier(),
                e
            );

            let new_status = ProxyKubeApiStatus::new(
                false,
                None,
                Some(format!(
                    "Failed to check if target service is reachable: {}",
                    e
                )),
            )
            .get_patch();
            let _ = proxys
                .patch_status(name, &ps, &new_status)
                .await
                .map_err(ControllerError::Kube)?;
            return Ok(Action::requeue(std::time::Duration::from_secs(5 * 60)));
        }
    };

    let mut redis_conn = ctx.get_redis_conn().await?;
    let proxy_json = proxy.to_json();
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

    let new_status =
        ProxyKubeApiStatus::new(true, Some(format!("/clusters/{}", path)), None).get_patch();
    let _ = proxys
        .patch_status(name, &ps, &new_status)
        .await
        .map_err(ControllerError::Kube)?;
    Ok(Action::requeue(std::time::Duration::from_secs(60 * 60)))
}
