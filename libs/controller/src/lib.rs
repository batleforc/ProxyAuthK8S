use common::State;
use crd::ProxyKubeApi;
use kube::Api;
use tracing::instrument;

pub mod ctx;
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
}
