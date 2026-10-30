//! What the proxy throttle does when one store command fails while the store
//! is up (RFC 0015 decision 20), the case a closed Redis port cannot produce:
//! the request goes through by default, and is refused under
//! `THROTTLE_FAIL_CLOSED`.
//!
//! Each test installs its own process-wide configuration, which holds because
//! nextest runs every test in its own process; under plain `cargo test` the
//! `configure` assertion fails loudly instead of testing the wrong setting.

mod harness;

use api::cluster::redirect::throttle::{self, Throttled};
use common::kv_memory::KvOp;
use harness::{
    fail2login_config, memory_state, memory_store, proxy_fixture, rate_limited_config,
    unique_cluster,
};

const SUBJECT: &str = "alice";

fn configure(fail_closed: bool) {
    let value = fail_closed.to_string();
    let config = common::config::init(common::config::Config::from_lookup(|key| {
        (key == "THROTTLE_FAIL_CLOSED").then(|| value.clone())
    }));
    assert_eq!(
        config.proxy.throttle_fail_closed, fail_closed,
        "another configuration was installed first; run under nextest"
    );
}

fn proxy_with(security: crd::security::SecurityConfiguration) -> crd::ProxyKubeApi {
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com");
    proxy.spec.security_config = Some(security);
    proxy
}

#[tokio::test]
async fn an_unreadable_ban_lets_the_request_through_by_default() {
    configure(false);
    let store = memory_store();
    let state = memory_state(&store, "https://idp.example.com".to_string());
    let proxy = proxy_with(fail2login_config(3, 60));

    store.fail_on(KvOp::Exists);
    assert!(!throttle::is_banned(&state, &proxy, SUBJECT).await);
}

#[tokio::test]
async fn an_unreadable_ban_is_refused_when_failing_closed() {
    configure(true);
    let store = memory_store();
    let state = memory_state(&store, "https://idp.example.com".to_string());
    let proxy = proxy_with(fail2login_config(3, 60));

    store.fail_on(KvOp::Exists);
    assert!(throttle::is_banned(&state, &proxy, SUBJECT).await);
}

#[tokio::test]
async fn an_uncountable_request_goes_through_by_default() {
    configure(false);
    let store = memory_store();
    let state = memory_state(&store, "https://idp.example.com".to_string());
    let proxy = proxy_with(rate_limited_config(1));

    store.fail_on(KvOp::Incr);
    assert_eq!(
        throttle::check_rate_limit(&state, &proxy, SUBJECT, &[]).await,
        None
    );
}

#[tokio::test]
async fn an_uncountable_request_is_refused_when_failing_closed() {
    configure(true);
    let store = memory_store();
    let state = memory_state(&store, "https://idp.example.com".to_string());
    let proxy = proxy_with(rate_limited_config(1));

    store.fail_on(KvOp::Incr);
    assert!(matches!(
        throttle::check_rate_limit(&state, &proxy, SUBJECT, &[]).await,
        Some(Throttled::RateLimited { limit: 1, .. })
    ));
}

/// `record_auth_failure` swallows the error: failures that could not be counted
/// ban nobody, and counting resumes once the store answers again.
#[tokio::test]
async fn uncounted_failures_ban_nobody_and_counting_resumes() {
    configure(false);
    let store = memory_store();
    let state = memory_state(&store, "https://idp.example.com".to_string());
    let proxy = proxy_with(fail2login_config(2, 60));

    store.fail_on(KvOp::Incr);
    for _ in 0..5 {
        throttle::record_auth_failure(&state, &proxy, SUBJECT).await;
    }
    store.heal();
    assert!(!throttle::is_banned(&state, &proxy, SUBJECT).await);

    for _ in 0..2 {
        throttle::record_auth_failure(&state, &proxy, SUBJECT).await;
    }
    assert!(throttle::is_banned(&state, &proxy, SUBJECT).await);
}
