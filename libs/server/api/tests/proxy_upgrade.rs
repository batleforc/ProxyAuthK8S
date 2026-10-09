//! Upgrade (exec/attach/port-forward) path, driven over real sockets.
//!
//! The upgrade path writes the handshake onto a raw upstream socket, so both
//! sides have to be real: an actix server bound on loopback, and a hand-rolled
//! upstream that records every byte it receives on its connection.

mod harness;

use std::time::Duration;

use actix_web::{App, HttpServer, web};
use api::cluster::redirect;
use crd::security::{
    AllowedPathConfiguration, AllowedPathConfigurationEnum, PortSpec, SecurityConfiguration,
};
use flate2::{Compress, Compression, FlushCompress};
use harness::{
    delete_proxy, proxy_fixture, seed_proxy, test_state, try_redis_pool, unique_cluster,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

/// Bind a test Redis, or bail out of the test (see `harness::try_redis_pool`).
macro_rules! redis_or_skip {
    () => {
        match try_redis_pool().await {
            Some(pool) => pool,
            None => return,
        }
    };
}

const SMUGGLED_MARKER: &str = "smuggled-request-marker";

/// Serve the redirect handlers on an ephemeral loopback port.
fn start_proxy() -> (std::net::SocketAddr, actix_web::dev::ServerHandle) {
    let state = web::Data::new(test_state("http://127.0.0.1:1".to_string()));
    let server = HttpServer::new(move || {
        App::new().app_data(state.clone()).service(
            web::scope("/clusters")
                .service(redirect::get_redirect)
                .service(redirect::post_redirect),
        )
    })
    .workers(1)
    .bind(("127.0.0.1", 0))
    .expect("proxy should bind");
    let addr = server.addrs()[0];
    let server = server.run();
    let handle = server.handle();
    actix_web::rt::spawn(server);
    (addr, handle)
}

/// A one-connection upstream: answer the handshake with `response`, then
/// record what else arrives on the connection until `linger` has elapsed or
/// the proxy closes it. With `echo`, those bytes are also sent back.
async fn start_upstream(
    response: &'static [u8],
    echo: bool,
    linger: Duration,
) -> (String, oneshot::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut received = Vec::new();
        let mut buf = [0u8; 4096];

        let header_end = loop {
            let read = socket.read(&mut buf).await.unwrap();
            assert!(read > 0, "proxy closed before sending the handshake");
            received.extend_from_slice(&buf[..read]);
            if let Some(pos) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        socket.write_all(response).await.unwrap();

        let mut after = received.split_off(header_end);
        if echo && !after.is_empty() {
            socket.write_all(&after).await.unwrap();
        }
        let deadline = tokio::time::Instant::now() + linger;
        while let Ok(Ok(read)) = tokio::time::timeout_at(deadline, socket.read(&mut buf)).await {
            if read == 0 {
                break;
            }
            if echo {
                socket.write_all(&buf[..read]).await.unwrap();
            }
            after.extend_from_slice(&buf[..read]);
        }

        let mut all = received;
        all.extend_from_slice(&after);
        let _ = tx.send(all);
    });

    (url, rx)
}

/// Regression: a handshake the upstream refuses must not turn into a tunnel.
/// actix hands every byte after an `Upgrade: websocket` request to the handler
/// as its payload; forwarding them after a non-101 answer lets the upstream
/// parse them as a second request that skipped authorization, the allow-list
/// and identity-header stripping.
#[actix_web::test]
async fn a_refused_upgrade_never_forwards_the_client_bytes() {
    let pool = redis_or_skip!();
    let (upstream_url, received) = start_upstream(
        b"HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: 6\r\n\r\ndenied",
        false,
        Duration::from_millis(500),
    )
    .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream_url)).await;
    let (addr, server) = start_proxy();

    let smuggled = format!(
        "GET /clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods/web/exec?command=ls HTTP/1.1\r\n\
         Host: proxy\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         \r\n\
         GET /api/v1/namespaces/kube-system/secrets?{SMUGGLED_MARKER} HTTP/1.1\r\n\
         Host: apiserver\r\n\
         X-Remote-User: system:admin\r\n\
         X-Remote-Group: system:masters\r\n\
         \r\n"
    );
    let mut client = TcpStream::connect(addr).await.unwrap();
    client.write_all(smuggled.as_bytes()).await.unwrap();

    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut response))
        .await
        .expect("the proxy should close the connection after a refused upgrade")
        .unwrap();
    let response = String::from_utf8_lossy(&response);
    assert!(response.starts_with("HTTP/1.1 403"), "got: {response}");
    assert!(response.ends_with("denied"), "got: {response}");

    let upstream_saw = String::from_utf8(received.await.unwrap()).unwrap();
    assert!(
        upstream_saw.contains("/api/v1/namespaces/dev/pods/web/exec"),
        "the handshake itself should reach the upstream: {upstream_saw}"
    );
    assert!(
        !upstream_saw.contains(SMUGGLED_MARKER),
        "client bytes were forwarded after a refused upgrade: {upstream_saw}"
    );

    server.stop(false).await;
    delete_proxy(&pool, &ns, &cluster).await;
}

/// A request that merely carries an `Upgrade` header, on a path the apiserver
/// never upgrades, goes through the standard path instead of a raw socket.
#[actix_web::test]
async fn an_upgrade_header_on_a_plain_path_does_not_open_a_raw_socket() {
    let pool = redis_or_skip!();
    let (upstream_url, received) = start_upstream(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
        false,
        Duration::from_millis(500),
    )
    .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream_url)).await;
    let (addr, server) = start_proxy();

    let request = format!(
        "GET /clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods HTTP/1.1\r\n\
         Host: proxy\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         \r\n"
    );
    let mut client = TcpStream::connect(addr).await.unwrap();
    client.write_all(request.as_bytes()).await.unwrap();
    // Nothing else will follow on this connection: end the websocket payload.
    client.shutdown().await.unwrap();

    let mut response = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut response)).await;

    let upstream_saw = String::from_utf8_lossy(&received.await.unwrap()).to_lowercase();
    assert!(
        upstream_saw.starts_with("get /api/v1/namespaces/dev/pods"),
        "got: {upstream_saw}"
    );
    assert!(
        !upstream_saw.contains("upgrade: websocket"),
        "the plain path must not forward an upgrade handshake: {upstream_saw}"
    );

    server.stop(false).await;
    delete_proxy(&pool, &ns, &cluster).await;
}

/// The fix must not break real upgrades: after a `101`, bytes flow both ways.
#[actix_web::test]
async fn an_accepted_upgrade_tunnels_both_ways() {
    let pool = redis_or_skip!();
    let (upstream_url, received) = start_upstream(
        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
        true,
        Duration::from_secs(2),
    )
    .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream_url)).await;
    let (addr, server) = start_proxy();

    let handshake = format!(
        "GET /clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods/web/exec?command=sh HTTP/1.1\r\n\
         Host: proxy\r\n\
         Upgrade: websocket\r\n\
         Connection: keep-alive, Upgrade\r\n\
         \r\n"
    );
    let mut client = TcpStream::connect(addr).await.unwrap();
    client.write_all(handshake.as_bytes()).await.unwrap();

    let mut response = Vec::new();
    let mut buf = [0u8; 4096];
    while !response.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = tokio::time::timeout(Duration::from_secs(5), client.read(&mut buf))
            .await
            .expect("the 101 should arrive")
            .unwrap();
        assert!(read > 0, "proxy closed the connection instead of upgrading");
        response.extend_from_slice(&buf[..read]);
    }
    assert!(
        response.starts_with(b"HTTP/1.1 101"),
        "got: {}",
        String::from_utf8_lossy(&response)
    );

    client.write_all(b"ping").await.unwrap();
    let mut echoed = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut echoed))
        .await
        .expect("the tunnelled bytes should come back")
        .unwrap();
    assert_eq!(&echoed, b"ping");
    drop(client);

    let upstream_saw = String::from_utf8_lossy(&received.await.unwrap()).to_lowercase();
    assert!(
        upstream_saw.contains("connection: upgrade\r\n"),
        "got: {upstream_saw}"
    );
    assert!(!upstream_saw.contains("keep-alive"), "got: {upstream_saw}");

    server.stop(false).await;
    delete_proxy(&pool, &ns, &cluster).await;
}

/// Bind an upstream that must never be contacted; resolves to whether a
/// connection came in within `wait`.
async fn start_unused_upstream(wait: Duration) -> (String, tokio::task::JoinHandle<bool>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let contacted =
        tokio::spawn(async move { tokio::time::timeout(wait, listener.accept()).await.is_ok() });
    (url, contacted)
}

/// Write `request` and read the whole response until the proxy closes.
async fn exchange(addr: std::net::SocketAddr, request: &[u8]) -> String {
    let mut client = TcpStream::connect(addr).await.unwrap();
    client.write_all(request).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut response))
        .await
        .expect("the proxy should answer and close")
        .unwrap();
    String::from_utf8_lossy(&response).into_owned()
}

/// Only websocket is tunnelled: actix-http cannot hand a SPDY session's bytes
/// to the handler, so a SPDY handshake is refused before reaching the cluster.
#[actix_web::test]
async fn a_spdy_upgrade_is_refused_before_reaching_the_cluster() {
    let pool = redis_or_skip!();
    let (upstream_url, contacted) = start_unused_upstream(Duration::from_millis(500)).await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream_url)).await;
    let (addr, server) = start_proxy();

    let handshake = format!(
        "POST /clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods/web/portforward HTTP/1.1\r\n\
         Host: proxy\r\n\
         Upgrade: SPDY/3.1\r\n\
         Connection: Upgrade\r\n\
         X-Stream-Protocol-Version: portforward.k8s.io\r\n\
         Content-Length: 0\r\n\
         Connection: close\r\n\
         \r\n"
    );
    let response = exchange(addr, handshake.as_bytes()).await;
    assert!(response.starts_with("HTTP/1.1 400"), "got: {response}");
    assert!(response.contains("websocket"), "got: {response}");
    assert!(!contacted.await.unwrap(), "the cluster was contacted");

    server.stop(false).await;
    delete_proxy(&pool, &ns, &cluster).await;
}

/// A proxy whose only rule allows port-forwarding pods in `dev` to `ports`.
fn port_forward_proxy(
    ns: &str,
    cluster: &str,
    upstream_url: &str,
    ports: &[&str],
) -> crd::ProxyKubeApi {
    let mut proxy = proxy_fixture(ns, cluster, upstream_url);
    proxy.spec.security_config = Some(SecurityConfiguration {
        enabled: true,
        allowed_resources: vec![AllowedPathConfigurationEnum::Path(
            AllowedPathConfiguration {
                path: "/api/v1/namespaces/dev/pods/*/portforward".to_string(),
                parametised: true,
                allowed_ports: Some(
                    ports
                        .iter()
                        .map(|port| PortSpec((*port).to_string()))
                        .collect(),
                ),
            },
        )],
        ..SecurityConfiguration::default()
    });
    proxy
}

/// With the channel protocols the ports are in the query string: a port
/// outside the rule is refused before the cluster is contacted.
#[actix_web::test]
async fn a_port_forward_query_outside_the_allowed_ports_is_refused() {
    let pool = redis_or_skip!();
    let (upstream_url, contacted) = start_unused_upstream(Duration::from_millis(500)).await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(
        &pool,
        &port_forward_proxy(&ns, &cluster, &upstream_url, &["8080"]),
    )
    .await;
    let (addr, server) = start_proxy();

    let handshake = format!(
        "GET /clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods/web/portforward?ports=8080&ports=22 HTTP/1.1\r\n\
         Host: proxy\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Protocol: v4.channel.k8s.io\r\n\
         \r\n"
    );
    let mut client = TcpStream::connect(addr).await.unwrap();
    client.write_all(handshake.as_bytes()).await.unwrap();
    client.shutdown().await.unwrap();
    let mut response = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut response)).await;
    let response = String::from_utf8_lossy(&response);

    assert!(response.starts_with("HTTP/1.1 403"), "got: {response}");
    assert!(response.contains("port 22"), "got: {response}");
    assert!(!contacted.await.unwrap(), "the cluster was contacted");

    server.stop(false).await;
    delete_proxy(&pool, &ns, &cluster).await;
}

mod spdy_dictionary {
    include!("../src/cluster/redirect/kube_redirect/spdy_dictionary.rs");
}

/// SPDY header compressor matching spdystream's (zlib with the SPDY/3
/// dictionary, sync-flushed per block).
struct SpdyHeaders(Compress);

impl SpdyHeaders {
    fn new() -> Self {
        let mut compress = Compress::new(Compression::best(), true);
        compress
            .set_dictionary(&spdy_dictionary::SPDY3_HEADER_DICTIONARY)
            .expect("dictionary");
        Self(compress)
    }

    fn syn_stream(&mut self, stream_id: u32, port: &str) -> Vec<u8> {
        let headers = [("port", port), ("streamtype", "data"), ("requestid", "0")];
        let mut block = u32::try_from(headers.len()).unwrap().to_be_bytes().to_vec();
        for (name, value) in headers {
            block.extend_from_slice(&u32::try_from(name.len()).unwrap().to_be_bytes());
            block.extend_from_slice(name.as_bytes());
            block.extend_from_slice(&u32::try_from(value.len()).unwrap().to_be_bytes());
            block.extend_from_slice(value.as_bytes());
        }
        let mut compressed = Vec::with_capacity(block.len() + 64);
        self.0
            .compress_vec(&block, &mut compressed, FlushCompress::Sync)
            .expect("compress");

        let mut body = stream_id.to_be_bytes().to_vec();
        body.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        body.extend(compressed);
        let mut frame = vec![0x80, 0x03, 0x00, 0x01, 0x00];
        frame.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes()[1..]);
        frame.extend(body);
        frame
    }
}

/// A masked binary websocket frame, as a client sends it.
fn ws_binary(payload: &[u8]) -> Vec<u8> {
    let mask = [0x0a, 0x0b, 0x0c, 0x0d];
    assert!(payload.len() <= 125);
    let mut frame = vec![0x82, 0x80 | u8::try_from(payload.len()).unwrap()];
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    frame
}

/// kubectl's transport: SPDY tunnelled in websocket. Streams to an allowed port
/// go through; the first one to another port closes the session before its
/// frame reaches the cluster.
#[actix_web::test]
async fn a_tunnelled_spdy_port_forward_is_filtered_per_stream() {
    let pool = redis_or_skip!();
    let (upstream_url, received) = start_upstream(
        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Protocol: SPDY/3.1+portforward.k8s.io\r\n\r\n",
        false,
        Duration::from_secs(3),
    )
    .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(
        &pool,
        &port_forward_proxy(&ns, &cluster, &upstream_url, &["8080"]),
    )
    .await;
    let (addr, server) = start_proxy();

    let handshake = format!(
        "GET /clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods/web/portforward HTTP/1.1\r\n\
         Host: proxy\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Protocol: SPDY/3.1+portforward.k8s.io\r\n\
         \r\n"
    );
    let mut client = TcpStream::connect(addr).await.unwrap();
    client.write_all(handshake.as_bytes()).await.unwrap();

    let mut response = Vec::new();
    let mut buf = [0u8; 4096];
    while !response.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = tokio::time::timeout(Duration::from_secs(5), client.read(&mut buf))
            .await
            .expect("the 101 should arrive")
            .unwrap();
        assert!(read > 0, "proxy closed the connection instead of upgrading");
        response.extend_from_slice(&buf[..read]);
    }
    assert!(
        response.starts_with(b"HTTP/1.1 101"),
        "got: {}",
        String::from_utf8_lossy(&response)
    );

    let mut spdy = SpdyHeaders::new();
    let allowed = ws_binary(&spdy.syn_stream(1, "8080"));
    let denied = ws_binary(&spdy.syn_stream(3, "22"));

    client.write_all(&allowed).await.unwrap();
    // Let the allowed frame through before the denied one arrives.
    tokio::time::sleep(Duration::from_millis(200)).await;
    client.write_all(&denied).await.unwrap();

    // The proxy closes the session.
    let mut rest = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut rest))
        .await
        .expect("the proxy should close a session that opens a denied port")
        .unwrap();

    let upstream_saw = received.await.unwrap();
    let contains = |needle: &[u8]| upstream_saw.windows(needle.len()).any(|w| w == needle);
    assert!(
        contains(&allowed),
        "the allowed stream should reach the cluster"
    );
    assert!(!contains(&denied), "the denied stream reached the cluster");

    server.stop(false).await;
    delete_proxy(&pool, &ns, &cluster).await;
}
