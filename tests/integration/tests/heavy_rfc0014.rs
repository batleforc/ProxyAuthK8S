//! RFC 0014 §6.P points 16 and 17, on the heavy bench (RFC 0016): what only
//! the upstream echo and the previous release's server can show. `task
//! it:heavy` runs them; `it:test` leaves every `heavy_*` binary out.

use std::time::Duration;

use integration::{
    fixture::{self, CLUSTER},
    heavy::{self, Previous},
};

/// The subprotocols a client using the token-in-subprotocol scheme offers:
/// the token base64url-encoded without padding, as client-go writes it.
fn bearer_protocols(token: &str) -> String {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(token);
    format!("base64url.bearer.authorization.k8s.io.{encoded}, v5.channel.k8s.io")
}

/// Point 16: an exec WebSocket offering its token in a subprotocol, in
/// `Passthrough` then in `FrontProxy`: the handshake the echo receives keeps
/// `v5.channel.k8s.io` alone (decision 39). In `FrontProxy`, the user's token
/// does not reach it either, and the identity comes as `X-Remote-User`.
/// `kubectl exec`, which never offers that subprotocol, is point 6 of the
/// integration suite (`rfc0014::p6_…`).
#[tokio::test]
async fn p16_the_bearer_subprotocol_never_reaches_upstream() {
    let p = heavy::echoed("heavy-0014-p16").await;
    let ns = p.ns.name.clone();
    let token = p.reader.cluster_token(&ns, CLUSTER).await;
    let exec = |marker: &str| {
        format!(
            "/api/v1/namespaces/{ns}/pods/web/exec?command=true&stdout=true&container=app&marker={marker}"
        )
    };

    let status =
        heavy::websocket_handshake(&ns, &exec("passthrough"), &token, &bearer_protocols(&token))
            .await;
    assert_eq!(status, 101, "Passthrough handshake");
    let seen = heavy::relayed(&ns, "marker=passthrough");
    assert_eq!(seen.len(), 1, "one handshake relayed: {seen:?}");
    assert_eq!(
        seen[0].header("sec-websocket-protocol"),
        vec!["v5.channel.k8s.io"],
        "Passthrough relays the channel protocol alone"
    );

    fixture::use_front_proxy(&p, &format!("external:{}", heavy::echo_url(&ns)));
    fixture::wait_trust(&ns, "Trusted").await;
    let status =
        heavy::websocket_handshake(&ns, &exec("frontproxy"), &token, &bearer_protocols(&token))
            .await;
    assert_eq!(status, 101, "FrontProxy handshake");
    let seen = heavy::relayed(&ns, "marker=frontproxy");
    assert_eq!(seen.len(), 1, "one handshake relayed: {seen:?}");
    let handshake = &seen[0];
    assert_eq!(
        handshake.header("sec-websocket-protocol"),
        vec!["v5.channel.k8s.io"],
        "FrontProxy relays the channel protocol alone"
    );
    assert!(
        handshake.header("authorization").is_empty(),
        "the user's token reached upstream: {handshake:?}"
    );
    assert_eq!(
        handshake.header("x-remote-user"),
        vec!["proxyauth:reader@it.local"],
        "the asserted identity"
    );
}

/// Point 17: the previous release's server, beside the current one on the
/// same Redis, serves a `Passthrough` cluster (the control) and answers a
/// cluster switched to `FrontProxy` as unknown: the echo receives nothing
/// from it, while the current proxy still serves it (decision 42).
#[tokio::test]
async fn p17_a_previous_server_never_serves_a_trusted_cluster() {
    let p = heavy::echoed("heavy-0014-p17").await;
    let ns = p.ns.name.clone();
    let token = p.reader.cluster_token(&ns, CLUSTER).await;
    let previous = Previous::start().await;
    let configmaps = |marker: &str| format!("/api/v1/namespaces/{ns}/configmaps?marker={marker}");

    let (status, body) = previous
        .get(&ns, &configmaps("old-passthrough"), &token)
        .await;
    assert_eq!(
        status, 200,
        "the previous server serves Passthrough: {body}"
    );
    assert_eq!(heavy::relayed(&ns, "marker=old-passthrough").len(), 1);

    fixture::use_front_proxy(&p, &format!("external:{}", heavy::echo_url(&ns)));
    fixture::wait_trust(&ns, "Trusted").await;
    let (status, body) = previous
        .get(&ns, &configmaps("old-frontproxy"), &token)
        .await;
    assert_eq!(
        status, 404,
        "the previous server must not know the cluster: {body}"
    );

    let response = integration::Bench::get()
        .proxied(
            reqwest::Method::GET,
            &ns,
            CLUSTER,
            &configmaps("new-frontproxy"),
            &token,
        )
        .send()
        .await
        .expect("request through the current proxy");
    assert_eq!(
        response.status(),
        200,
        "the current proxy serves FrontProxy"
    );
    assert!(
        heavy::eventually(Duration::from_secs(10), || {
            !heavy::relayed(&ns, "marker=new-frontproxy").is_empty()
        })
        .await,
        "the current proxy's request reached the echo"
    );
    assert!(
        heavy::relayed(&ns, "marker=old-frontproxy").is_empty(),
        "the previous server relayed a FrontProxy request"
    );
}
