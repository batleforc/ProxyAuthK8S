use std::sync::Arc;

use common::State;
use crd::ProxyKubeApi;
use crd::PROXY_KUBE_FINALIZER;
use kube::runtime::controller::Action;
use kube::runtime::finalizer;
use kube::Api;
use kube::ResourceExt;
use opentelemetry::TraceId;
use trace::helper::get_trace_id;
use tracing::{instrument, warn};

use crate::error::ControllerError;
use crate::error::Result;

pub mod cleanup;
pub mod reconcile;

#[instrument(skip(_ctx))]
pub fn error_policy_proxy_kube_api(
    proxy: Arc<ProxyKubeApi>,
    _error: &ControllerError,
    _ctx: Arc<State>,
) -> kube::runtime::controller::Action {
    warn!(
        "Reconciliation error for ProxyKubeApi {}/{}",
        proxy.metadata.namespace.as_deref().unwrap_or_default(),
        proxy.metadata.name.as_deref().unwrap_or_default()
    );
    // Requeue after 5 seconds
    kube::runtime::controller::Action::requeue(std::time::Duration::from_secs(5))
}

#[instrument(skip(ctx, proxy), fields(trace_id))]
pub async fn main_reconcile_proxy_kube_api(
    proxy: Arc<ProxyKubeApi>,
    ctx: Arc<State>,
) -> Result<Action> {
    let trace_id = get_trace_id();
    if trace_id != TraceId::INVALID {
        tracing::Span::current().record("trace_id", &tracing::field::display(trace_id));
    }
    let ns = match proxy.namespace() {
        Some(ns) => ns,
        None => {
            tracing::error!(name = proxy.metadata.name, "ProxyKubeApi has no namespace");
            return Err(ControllerError::InvalidResource(
                "ProxyKubeApi has no namespace".to_string(),
            )
            .into());
        }
    };
    let proxys: Api<ProxyKubeApi> = Api::namespaced(ctx.client.clone(), &ns);

    tracing::info!("Reconciling ProxyKubeApi {}/{}", ns, proxy.name_any());
    finalizer(&proxys, PROXY_KUBE_FINALIZER, proxy, |event| async {
        match event {
            finalizer::Event::Apply(proxy) => {
                tracing::info!("Applying ProxyKubeApi {}/{}", ns, proxy.name_any());
                reconcile::reconcile_proxy_kube_api(&proxy, ctx.clone()).await
            }
            finalizer::Event::Cleanup(proxy) => {
                tracing::info!("Cleaning up ProxyKubeApi {}/{}", ns, proxy.name_any());
                cleanup::clean_proxy_kube_api(&proxy, ctx.clone()).await
            }
        }
    })
    .await
    .map_err(|e| ControllerError::FinalizerError(Box::new(e)))
}
