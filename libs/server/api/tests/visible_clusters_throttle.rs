//! Throttling on `/api/v1/clusters`, the one authenticated endpoint with no
//! cluster to key a budget on.
//!
//! Every request here authenticates, so an anonymous caller still costs one
//! outbound `/userinfo` call to the provider before it can be refused. The
//! cluster endpoints gate that with the proxy's own `SecurityConfiguration`;
//! this one has no proxy, so the budget comes from the `UNSCOPED_*` environment
//! knobs — and, critically, is **off** unless an operator sets them. Behind an
//! ingress without `TRUSTED_PROXY_COUNT`, every caller collapses to one address,
//! so a default-on ban would let a single bad client lock out everybody.
//!
//! The IdP here is a wiremock server with no routes: a test that sees 429 proves
//! the caller was refused before the provider was ever contacted.

mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::visible_clusters::get_all_visible_cluster::get_all_visible_cluster;
use deadpool_redis::redis::AsyncTypedCommands;
use harness::{test_state, try_redis_pool};
use tokio::sync::{Mutex, MutexGuard};
use wiremock::MockServer;

/// Serializes the tests that mutate the process environment. Under nextest each
/// test is its own process, but a plain `cargo test` runs them as threads of
/// one, where concurrent `set_var`/`var` on the same variable would race.
///
/// A `tokio` mutex rather than `std`: these tests hold the guard across `.await`
/// points, and a `std` guard held across an await can deadlock the runtime (and
/// is rejected by clippy). It also cannot be poisoned, so a panicking test
/// simply releases it.
static ENV_LOCK: Mutex<()> = Mutex::const_new(());

async fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().await
}

/// Restores the `UNSCOPED_*` knobs to whatever they held before.
struct EnvGuard {
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn set(pairs: &[(&'static str, &str)]) -> Self {
        let keys = [
            "UNSCOPED_MAX_FAILED_LOGINS",
            "UNSCOPED_BAN_DURATION_SECONDS",
            "UNSCOPED_RATE_LIMIT_PER_MINUTE",
        ];
        let saved = keys
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect();
        // SAFETY: the caller holds the environment lock (see `lock_env`).
        unsafe {
            for key in keys {
                std::env::remove_var(key);
            }
            for (key, value) in pairs {
                std::env::set_var(key, value);
            }
        }
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: the caller holds the environment lock (see `lock_env`).
        unsafe {
            for (key, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

macro_rules! clusters_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new()
                .app_data(web::Data::new($state))
                .service(web::scope("/api/v1").service(get_all_visible_cluster)),
        )
        .await
    };
}

macro_rules! redis_or_skip {
    () => {
        match try_redis_pool().await {
            Some(pool) => pool,
            None => return,
        }
    };
}

/// Every test keys on its own address.
///
/// The unscoped scope is a single Redis namespace shared by the whole binary,
/// and these tests run concurrently, so a shared subject would have one test's
/// cleanup delete the ban another had just earned. `peer_addr` is absent under
/// `test::call_service`, so an `x-forwarded-for` of one hop is what the subject
/// resolves to regardless of `TRUSTED_PROXY_COUNT`.
async fn clear(pool: &deadpool_redis::Pool, subject: &str) {
    let mut conn = pool.get().await.expect("redis connection");
    for kind in ["ban", "fail2login", "ratelimit"] {
        let _ = conn
            .del(format!("proxyk8sauth:{kind}:_unscoped:{subject}"))
            .await;
    }
}

/// A request from `subject`, so each test occupies its own budget.
fn request_from(subject: &str) -> test::TestRequest {
    test::TestRequest::get()
        .uri("/api/v1/clusters")
        .insert_header(("x-forwarded-for", subject.to_string()))
}

/// The default must not change behaviour for anyone: with nothing configured an
/// anonymous caller still gets 401, never 429.
#[actix_web::test]
async fn nothing_is_throttled_by_default() {
    const SUBJECT: &str = "10.0.0.1";
    let pool = redis_or_skip!();
    clear(&pool, SUBJECT).await;
    let _lock = lock_env().await;
    let _guard = EnvGuard::set(&[]);
    let idp = MockServer::start().await;

    let app = clusters_app!(test_state(idp.uri()));
    for _ in 1..=5 {
        let resp = test::call_service(&app, request_from(SUBJECT).to_request()).await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "an unconfigured deployment must behave exactly as before"
        );
    }
    clear(&pool, SUBJECT).await;
}

/// Once configured, repeated failures earn a ban — which is what stops an
/// anonymous caller driving `/userinfo` at the provider indefinitely.
#[actix_web::test]
async fn repeated_auth_failures_earn_a_ban_once_configured() {
    const SUBJECT: &str = "10.0.0.2";
    let pool = redis_or_skip!();
    clear(&pool, SUBJECT).await;
    let _lock = lock_env().await;
    let _guard = EnvGuard::set(&[
        ("UNSCOPED_MAX_FAILED_LOGINS", "2"),
        ("UNSCOPED_BAN_DURATION_SECONDS", "300"),
    ]);
    let idp = MockServer::start().await;

    let app = clusters_app!(test_state(idp.uri()));
    for attempt in 1..=2 {
        let resp = test::call_service(&app, request_from(SUBJECT).to_request()).await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "failure {attempt} should be counted, not yet banned"
        );
    }

    let resp = test::call_service(&app, request_from(SUBJECT).to_request()).await;
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the threshold should have produced a ban"
    );

    let mut conn = pool.get().await.expect("redis connection");
    assert!(
        conn.exists(format!("proxyk8sauth:ban:_unscoped:{SUBJECT}"))
            .await
            .expect("redis exists should succeed"),
        "a ban key should have been written under the unscoped scope"
    );
    clear(&pool, SUBJECT).await;
}

#[actix_web::test]
async fn the_rate_limit_applies_once_configured() {
    const SUBJECT: &str = "10.0.0.3";
    let pool = redis_or_skip!();
    clear(&pool, SUBJECT).await;
    let _lock = lock_env().await;
    let _guard = EnvGuard::set(&[("UNSCOPED_RATE_LIMIT_PER_MINUTE", "2")]);
    let idp = MockServer::start().await;

    let app = clusters_app!(test_state(idp.uri()));
    for _ in 1..=2 {
        let resp = test::call_service(&app, request_from(SUBJECT).to_request()).await;
        assert_ne!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    let resp = test::call_service(&app, request_from(SUBJECT).to_request()).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        resp.headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("60")
    );
    clear(&pool, SUBJECT).await;
}

/// The ban must be checked before the token, so a banned caller costs no
/// outbound call at all. The IdP mock has no routes: reaching it would surface
/// as a 401 from a failed lookup, not a 429.
#[actix_web::test]
async fn a_banned_caller_is_refused_before_the_provider_is_contacted() {
    const SUBJECT: &str = "10.0.0.4";
    let pool = redis_or_skip!();
    clear(&pool, SUBJECT).await;
    let _lock = lock_env().await;
    let _guard = EnvGuard::set(&[("UNSCOPED_MAX_FAILED_LOGINS", "3")]);

    let mut conn = pool.get().await.expect("redis connection");
    conn.set_ex(format!("proxyk8sauth:ban:_unscoped:{SUBJECT}"), "1", 300)
        .await
        .expect("ban key should be set");

    let idp = MockServer::start().await;
    let app = clusters_app!(test_state(idp.uri()));
    let resp = test::call_service(
        &app,
        request_from(SUBJECT)
            .insert_header(("Authorization", "Bearer anything"))
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        idp.received_requests()
            .await
            .expect("wiremock should record requests")
            .is_empty(),
        "a banned caller must not have reached the provider at all"
    );
    clear(&pool, SUBJECT).await;
}
