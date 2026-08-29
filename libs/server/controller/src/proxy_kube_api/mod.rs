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
    use super::*;
    use crd::{ProxyKubeApiSpec, certificate::CertSource, service::Service};

    /// A `State` whose Kubernetes client and Redis both point at a port nothing
    /// listens on: every test below must return before touching either, so a
    /// regression that starts calling out shows up as a failure rather than as
    /// a silent extra round-trip.
    fn unreachable_state() -> State {
        // Building a kube client reaches for the rustls provider the server
        // installs in `main`; without it `Client::try_from` panics.
        static CRYPTO: std::sync::Once = std::sync::Once::new();
        CRYPTO.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });

        let kube_config =
            kube::Config::new("http://127.0.0.1:1".parse().expect("static uri parses"));
        State::from_parts(
            kube::Client::try_from(kube_config).expect("kube client builds"),
            common::redis_pool::RedisPool::from_url("redis://127.0.0.1:1")
                .expect("redis pool builds"),
            common::oidc_conf::OidcConf {
                client_id: "proxyauthk8s".to_string(),
                client_secret: None,
                issuer_url: "https://oidc.example.com".to_string(),
                scopes: "openid".to_string(),
                audience: "proxyauthk8s".to_string(),
                accept_authorized_party: false,
                redirect_url: None,
            },
            "https://proxy.example.com".to_string(),
            "https://front.example.com".to_string(),
        )
    }

    fn proxy(namespace: Option<&str>) -> Arc<ProxyKubeApi> {
        let mut proxy = ProxyKubeApi::new(
            "test-cluster",
            ProxyKubeApiSpec {
                enabled: true,
                cert: CertSource::Insecure(true),
                client_cert: None,
                service: Service::ExternalService {
                    url: "https://cluster.example.com".to_string(),
                },
                auth_config: None,
                security_config: None,
                expose_via_dashboard: false,
                dashboard_group: None,
                proxy_group: None,
                virtual_apis: Vec::new(),
            },
        );
        proxy.metadata.namespace = namespace.map(std::string::ToString::to_string);
        Arc::new(proxy)
    }

    fn leader(state: &State, is_leader: bool) {
        state
            .is_leader
            .store(is_leader, std::sync::atomic::Ordering::Relaxed);
    }

    #[tokio::test]
    async fn a_follower_requeues_without_reconciling() {
        // A non-leader replica must not reconcile at all — two replicas writing
        // the same Redis keys and patching the same status is the split-brain
        // the lease exists to prevent.
        let state = unreachable_state();
        leader(&state, false);

        let action = main_reconcile_proxy_kube_api(proxy(Some("default")), Arc::new(state))
            .await
            .expect("a follower short-circuits successfully");

        // ~lease TTL, so a freshly promoted replica converges quickly instead of
        // leaving state stale until the hourly success requeue.
        assert_eq!(action, Action::requeue(Duration::from_secs(20)));
    }

    #[tokio::test]
    async fn a_follower_requeues_on_the_error_policy_too() {
        let state = unreachable_state();
        leader(&state, false);

        let action = error_policy_proxy_kube_api(
            proxy(Some("default")),
            &ControllerError::InvalidResource("boom".to_string()),
            Arc::new(state),
        );

        assert_eq!(action, Action::requeue(Duration::from_secs(20)));
    }

    // Async only because building the kube client needs a Tokio reactor; the
    // error policy itself is synchronous.
    #[tokio::test]
    async fn the_leader_retries_quickly_after_a_reconcile_error() {
        let state = unreachable_state();
        leader(&state, true);

        let action = error_policy_proxy_kube_api(
            proxy(Some("default")),
            &ControllerError::InvalidResource("boom".to_string()),
            Arc::new(state),
        );

        assert_eq!(action, Action::requeue(Duration::from_secs(5)));
    }

    #[tokio::test]
    async fn a_namespaceless_proxy_is_rejected_before_any_api_call() {
        // `Api::namespaced` would otherwise silently target "default". The check
        // runs before the client is used, which is why an unreachable one here
        // still returns promptly.
        let state = unreachable_state();
        leader(&state, true);

        let error = main_reconcile_proxy_kube_api(proxy(None), Arc::new(state))
            .await
            .expect_err("a cluster-scoped ProxyKubeApi is not reconcilable");

        assert!(
            matches!(error, ControllerError::InvalidResource(_)),
            "expected an invalid-resource error, got {error:?}"
        );
    }
}
