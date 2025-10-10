use std::sync::Arc;

use common::State;
use crd::ProxyKubeApi;
use futures::StreamExt;
use kube::{
    runtime::{watcher::Config, Controller},
    Api,
};
use tracing::instrument;

use crate::proxy_kube_api::{error_policy_proxy_kube_api, reconcile_proxy_kube_api};

pub mod error;
pub mod proxy_kube_api;

#[instrument(skip(state))]
pub async fn run(state: State) {
    let client = state.client.clone();
    let proxy_kube_apis = Api::<ProxyKubeApi>::all(client.clone());
    if let Err(e) = proxy_kube_apis.list(&Default::default()).await {
        tracing::error!(
            "Failed to list ProxyKubeApi resources (the CRD maybe not installed) : {}",
            e
        );
        panic!("Failed to list ProxyKubeApi resources: {}", e);
    }
    let controller_state = Arc::new(state.clone());

    Controller::new(proxy_kube_apis, Config::default().any_semantic())
        .shutdown_on_signal()
        .run(
            reconcile_proxy_kube_api,
            error_policy_proxy_kube_api,
            controller_state,
        )
        .filter_map(|x| async move { std::result::Result::ok(x) })
        .for_each(|_| futures::future::ready(()))
        .await;
}
