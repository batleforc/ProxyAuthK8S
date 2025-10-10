use std::sync::Arc;

use common::State;
use crd::ProxyKubeApi;
use kube::runtime::controller::Action;

use crate::error::Result;

pub async fn reconcile_proxy_kube_api(proxy: &ProxyKubeApi, ctx: Arc<State>) -> Result<Action> {
    Ok(Action::requeue(std::time::Duration::from_secs(300)))
}
