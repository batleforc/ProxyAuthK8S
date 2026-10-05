//! The per-cluster upstream client cache keeps connections to the apiserver
//! alive across proxied requests (see `common::upstream_cache`).
//!
//! The upstream is a bare `TcpListener` speaking keep-alive HTTP/1.1, so the
//! test observes what matters: how many TCP connections the proxy opened. With
//! a client per request every request needs its own connection; with the
//! cached client the second request reuses the first one's.
//!
//! The cache is off under `common`'s `test-util` feature; this binary turns it
//! on for itself, and every test uses its own proxy name and listener so they
//! cannot share an entry.

mod harness;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::redirect;
use crd::certificate::CertSource;
use harness::{
    delete_proxy, proxy_fixture, seed_proxy, test_state, try_redis_pool, unique_cluster,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
};

macro_rules! proxy_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new()
                .app_data(web::Data::new($state))
                .service(web::scope("/clusters").service(redirect::get_redirect)),
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

/// A keep-alive HTTP/1.1 upstream answering `200 ok` to every (body-less)
/// request; returns its base URL and the number of connections accepted.
async fn counting_upstream() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let url = format!("http://{}", listener.local_addr().expect("local addr"));
    let connections = Arc::new(AtomicUsize::new(0));
    let accepted = Arc::clone(&connections);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            accepted.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut pending = Vec::new();
                let mut buf = [0_u8; 4096];
                loop {
                    let Ok(read) = socket.read(&mut buf).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    pending.extend_from_slice(&buf[..read]);
                    while let Some(end) = pending.windows(4).position(|w| w == b"\r\n\r\n") {
                        pending.drain(..end + 4);
                        let response = b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 2\r\n\r\nok";
                        if socket.write_all(response).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    (url, connections)
}

/// `GET /api/v1/pods` through the proxy, expecting the upstream's `200 ok`.
macro_rules! get_ok {
    ($app:expr_2021, $ns:expr_2021, $cluster:expr_2021) => {{
        let req = test::TestRequest::get()
            .uri(&format!("/clusters/{}/{}/api/v1/pods", $ns, $cluster))
            .to_request();
        let resp = test::call_service($app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(test::read_body(resp).await, "ok");
    }};
}

#[actix_web::test]
async fn consecutive_requests_reuse_one_upstream_connection() {
    common::upstream_cache::enable_for_tests();
    let pool = redis_or_skip!();
    let (upstream, connections) = counting_upstream().await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream)).await;
    let app = proxy_app!(test_state(upstream.clone()));

    for _ in 0..3 {
        get_ok!(&app, &ns, &cluster);
    }
    assert_eq!(
        connections.load(Ordering::SeqCst),
        1,
        "the cached client must keep its connection alive across requests"
    );

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn a_tls_relevant_spec_change_builds_a_new_client() {
    common::upstream_cache::enable_for_tests();
    let pool = redis_or_skip!();
    let (upstream, connections) = counting_upstream().await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream);
    seed_proxy(&pool, &proxy).await;
    let app = proxy_app!(test_state(upstream.clone()));

    get_ok!(&app, &ns, &cluster);
    assert_eq!(connections.load(Ordering::SeqCst), 1);

    // Same proxy, different certificate source: a fresh client (and pool).
    proxy.spec.cert = CertSource::Insecure(false);
    seed_proxy(&pool, &proxy).await;
    get_ok!(&app, &ns, &cluster);
    assert_eq!(connections.load(Ordering::SeqCst), 2);

    // ...which is then reused in turn.
    get_ok!(&app, &ns, &cluster);
    assert_eq!(connections.load(Ordering::SeqCst), 2);

    delete_proxy(&pool, &ns, &cluster).await;
}
