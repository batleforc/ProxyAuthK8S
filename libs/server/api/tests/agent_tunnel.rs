//! RFC 0012: clusters reached through an agent tunnel.
//!
//! An `Agent` cluster has no address the proxy may dial: without a tunnel it
//! is refused with `503 agent unavailable`, never reached directly.
mod harness;

use axum::http::StatusCode;
use crd::certificate::CertSource;
use crd::service::{
    AgentBootstrap, AgentService, DEFAULT_AUDIENCE, DEFAULT_SERVER_NAME, Service,
    ServiceAccountTokenBootstrap,
};
use harness::test;
use harness::{
    delete_proxy, proxy_fixture, seed_proxy, test_state, try_redis_pool, unique_cluster,
};

macro_rules! redis_or_skip {
    () => {
        match try_redis_pool().await {
            Some(pool) => pool,
            None => return,
        }
    };
}

fn agent_service() -> Service {
    Service::Agent(AgentService {
        server_name: DEFAULT_SERVER_NAME.to_string(),
        epoch: 0,
        bootstrap: AgentBootstrap::ServiceAccountToken(ServiceAccountTokenBootstrap {
            subject: "system:serviceaccount:proxyauth-agent:proxyauth-agent".to_string(),
            audience: DEFAULT_AUDIENCE.to_string(),
            jwks: None,
            jwks_url: Some("https://issuer.example.com/keys".to_string()),
            jwks_from: None,
        }),
    })
}

/// §6.P point 4: no tunnel, `503 agent unavailable`, and no fallback to the
/// address `url_to_call` names (`https://kubernetes.default.svc`, which from
/// the proxy's pod would be its own cluster).
#[tokio::test]
async fn an_agent_cluster_without_a_tunnel_is_refused_and_never_dialled() {
    let pool = redis_or_skip!();
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://unused.example.com");
    proxy.spec.service = agent_service();
    proxy.spec.cert = CertSource::ConfigMap {
        name: "prod-ca".to_string(),
        key: "ca.crt".to_string(),
        namespace: None,
    };
    seed_proxy(&pool, &proxy).await;

    let app = test::init_service(harness::app(test_state(
        "https://idp.example.com".to_string(),
    )))
    .await;
    for request in [
        test::TestRequest::get().uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods")),
        // An upgrade takes another path after the gates; the refusal comes first.
        test::TestRequest::get()
            .uri(&format!(
                "/clusters/{ns}/{cluster}/api/v1/namespaces/default/pods/web/exec?command=sh&stdout=true"
            ))
            .insert_header(("connection", "Upgrade"))
            .insert_header(("upgrade", "websocket")),
    ] {
        let resp = test::call_service(&app, request.to_request()).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["kind"], "Status");
        assert_eq!(body["message"], "agent unavailable");
    }

    delete_proxy(&pool, &ns, &cluster).await;
}
