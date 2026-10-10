use std::sync::Arc;
use std::time::Duration;

use common::State;
use crd::PROXY_KUBE_FINALIZER;
use crd::ProxyKubeApi;
use kube::Api;
use kube::ResourceExt;
use kube::runtime::controller::Action;
use kube::runtime::finalizer;
use opentelemetry::TraceId;
use trace::helper::get_trace_id;
use tracing::{instrument, warn};

use crate::error::ControllerError;
use crate::error::Result;

pub mod cleanup;
pub mod reconcile;

/// Redis key prefix under which cluster configurations are cached.
///
/// Must match [`ProxyKubeApi::to_identifier`], which produces
/// `proxyk8sauth:{namespace}/{name}`.
pub const REDIS_PREFIX: &str = crd::REDIS_PREFIX;

#[instrument(skip(ctx))]
pub fn error_policy_proxy_kube_api(
    proxy: Arc<ProxyKubeApi>,
    _error: &ControllerError,
    ctx: Arc<State>,
) -> Action {
    if !ctx.is_leader.load(std::sync::atomic::Ordering::Relaxed) {
        tracing::info!(
            "This instance is not the leader, skipping reconciliation for ProxyKubeApi {}/{}",
            proxy.namespace().as_deref().unwrap_or_default(),
            proxy.name_any()
        );
        // Keep this short (~lease TTL) so that once this instance is promoted it
        // converges quickly instead of leaving stale state for up to an hour.
        return Action::requeue(Duration::from_secs(20));
    }
    warn!(
        "Reconciliation error for ProxyKubeApi {}/{}",
        proxy.metadata.namespace.as_deref().unwrap_or_default(),
        proxy.metadata.name.as_deref().unwrap_or_default()
    );
    // Requeue after 5 seconds
    Action::requeue(std::time::Duration::from_secs(5))
}

#[instrument(skip(ctx, proxy), fields(trace_id))]
pub async fn main_reconcile_proxy_kube_api(
    proxy: Arc<ProxyKubeApi>,
    ctx: Arc<State>,
) -> Result<Action> {
    if !ctx.is_leader.load(std::sync::atomic::Ordering::Relaxed) {
        tracing::info!(
            "This instance is not the leader, skipping reconciliation for ProxyKubeApi {}/{}",
            proxy.namespace().as_deref().unwrap_or_default(),
            proxy.name_any()
        );
        // Even if not the leader, requeue soon (~lease TTL) so a freshly promoted
        // leader re-reconciles already-seen objects quickly instead of leaving
        // their state stale for up to an hour, while still avoiding a hot loop.
        return Ok(Action::requeue(Duration::from_secs(20)));
    }
    let trace_id = get_trace_id();
    if trace_id != TraceId::INVALID {
        tracing::Span::current().record("trace_id", tracing::field::display(trace_id));
    }
    let ns = if let Some(ns) = proxy.namespace() {
        ns
    } else {
        tracing::error!(name = proxy.metadata.name, "ProxyKubeApi has no namespace");
        return Err(ControllerError::InvalidResource(
            "ProxyKubeApi has no namespace".to_string(),
        ));
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use common::{oidc_conf::OidcConf, redis_pool::RedisPool};
    use crd::{ProxyKubeApiSpec, certificate::CertSource, service::Service};

    use super::*;

    /// A `State` whose Kubernetes API and Redis point at a closed port: any
    /// test that reaches them would fail, which proves the short-circuits below
    /// never touch either.
    fn offline_state(is_leader: bool) -> Arc<State> {
        static CRYPTO: std::sync::Once = std::sync::Once::new();
        CRYPTO.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
        let kube_config = kube::Config::new(
            "http://127.0.0.1:1"
                .parse()
                .expect("static uri should parse"),
        );
        let state = State::from_parts(
            kube::Client::try_from(kube_config).expect("kube client should build"),
            RedisPool::from_url("redis://127.0.0.1:1").expect("pool should build"),
            OidcConf {
                client_id: "proxyauthk8s".to_string(),
                client_secret: None,
                issuer_url: "http://127.0.0.1:1".to_string(),
                scopes: "openid".to_string(),
                audience: "proxyauthk8s".to_string(),
                accept_authorized_party: false,
                redirect_url: None,
            },
            "https://proxy.example.com".to_string(),
            "https://front.example.com".to_string(),
        );
        state.is_leader.store(is_leader, Ordering::Relaxed);
        Arc::new(state)
    }

    fn proxy(namespace: Option<&str>) -> Arc<ProxyKubeApi> {
        let mut proxy = ProxyKubeApi::new(
            "test-cluster",
            ProxyKubeApiSpec {
                enabled: true,
                cert: CertSource::SystemRoots(true),
                client_cert: None,
                service: Service::ExternalService {
                    url: "https://127.0.0.1:1".to_string(),
                },
                auth_config: None,
                security_config: None,
                expose_via_dashboard: false,
                dashboard_group: None,
                proxy_group: None,
                virtual_apis: Vec::new(),
            },
        );
        proxy.metadata.namespace = namespace.map(str::to_string);
        Arc::new(proxy)
    }

    #[tokio::test]
    async fn follower_skips_reconcile_and_requeues_near_the_lease_ttl() {
        let action = main_reconcile_proxy_kube_api(proxy(Some("default")), offline_state(false))
            .await
            .expect("a follower must not fail the reconcile");
        assert_eq!(action, Action::requeue(Duration::from_secs(20)));
    }

    #[tokio::test]
    async fn leader_rejects_a_proxy_without_namespace() {
        let result = main_reconcile_proxy_kube_api(proxy(None), offline_state(true)).await;
        assert!(
            matches!(result, Err(ControllerError::InvalidResource(_))),
            "expected InvalidResource, got {result:?}"
        );
    }

    #[tokio::test]
    async fn error_policy_requeues_quickly_on_the_leader() {
        let error = ControllerError::InvalidResource("boom".to_string());
        let action =
            error_policy_proxy_kube_api(proxy(Some("default")), &error, offline_state(true));
        assert_eq!(action, Action::requeue(Duration::from_secs(5)));
    }

    #[tokio::test]
    async fn error_policy_on_a_follower_waits_for_promotion() {
        let error = ControllerError::InvalidResource("boom".to_string());
        let action =
            error_policy_proxy_kube_api(proxy(Some("default")), &error, offline_state(false));
        assert_eq!(action, Action::requeue(Duration::from_secs(20)));
    }
}
