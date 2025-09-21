use std::sync::Arc;

use crd::ProxyKubeApi;
use tracing::warn;

use crate::{ctx::Context, error::ControllerError};

pub fn error_policy_proxy_kube_api(
    proxy: Arc<ProxyKubeApi>,
    _error: &ControllerError,
    _ctx: Arc<Context>,
) -> kube::runtime::controller::Action {
    warn!(
        "Reconciliation error for ProxyKubeApi {}/{}",
        proxy.metadata.namespace.as_deref().unwrap_or_default(),
        proxy.metadata.name.as_deref().unwrap_or_default()
    );
    // Requeue after 5 seconds
    kube::runtime::controller::Action::requeue(std::time::Duration::from_secs(5))
}
