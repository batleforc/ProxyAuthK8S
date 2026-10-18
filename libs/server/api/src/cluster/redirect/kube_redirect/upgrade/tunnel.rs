//! Piping an upgraded connection between the client and the upstream.
//!
//! Both handshakes are hyper's: the client's `101` is the response the
//! handler returns, and both connections are taken over with
//! `hyper::upgrade::on`, so no byte of either side is parsed here.

use axum::{
    body::Body,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{error, warn};

use super::super::port_forward::PortForwardFilter;
use crate::cluster::redirect::audit::{AuditContext, PortDecision};

/// Bytes read from the client at a time on a filtered session.
const FILTER_READ_BUFFER: usize = 8 * 1024;

/// Whether an upstream response header is not relayed to the client: framing
/// and connection management belong to each hop.
fn is_dropped_response_header(header_name: &str) -> bool {
    [
        "transfer-encoding",
        "content-length",
        "host",
        "connection",
        "keep-alive",
        "upgrade",
    ]
    .iter()
    .any(|dropped| header_name.eq_ignore_ascii_case(dropped))
}

/// The client response for an upstream answer: its status and relayable
/// headers, and `body`.
pub(super) fn relayed_response(status: StatusCode, upstream: &HeaderMap, body: Body) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    for (name, value) in upstream {
        if !is_dropped_response_header(name.as_str()) {
            headers.append(name, value.clone());
        }
    }
    response
}

/// Answer the client with the upstream's `101`, and pipe both directions once
/// both connections are upgraded.
///
/// `filter`, when set, inspects every client chunk of a restricted
/// port-forward and ends the session on a port outside the policy; each port
/// it opens, and the closing, are recorded on the audit context with it.
pub(super) fn tunnel(
    client: OnUpgrade,
    upstream: OnUpgrade,
    upstream_headers: &HeaderMap,
    upgrade_protocol: &str,
    filter: Option<(PortForwardFilter, AuditContext)>,
) -> Response {
    let Ok(upgrade_protocol) = header::HeaderValue::from_str(upgrade_protocol) else {
        return (StatusCode::BAD_GATEWAY, "bad gateway").into_response();
    };
    tokio::spawn(async move {
        let (client, upstream) = match tokio::try_join!(client, upstream) {
            Ok(both) => both,
            Err(err) => {
                error!(%err, "an upgraded connection could not be taken over");
                return;
            }
        };
        let (mut client, mut upstream) = (TokioIo::new(client), TokioIo::new(upstream));
        match filter {
            Some(filter) => pipe_filtered(client, upstream, filter).await,
            None => {
                if let Err(err) = tokio::io::copy_bidirectional(&mut client, &mut upstream).await {
                    warn!(%err, "upgraded connection ended with an error");
                }
            }
        }
    });

    let mut response = relayed_response(
        StatusCode::SWITCHING_PROTOCOLS,
        upstream_headers,
        Body::empty(),
    );
    let headers = response.headers_mut();
    headers.insert(
        header::CONNECTION,
        header::HeaderValue::from_static("upgrade"),
    );
    headers.insert(header::UPGRADE, upgrade_protocol);
    response
}

/// Copy both directions, the client's through `filter`; a refused chunk is
/// never forwarded and ends the whole session.
async fn pipe_filtered<C, U>(
    client: C,
    upstream: U,
    (mut filter, audit): (PortForwardFilter, AuditContext),
) where
    C: AsyncRead + AsyncWrite + Unpin,
    U: AsyncRead + AsyncWrite + Unpin,
{
    let (mut client_reader, mut client_writer) = tokio::io::split(client);
    let (mut upstream_reader, mut upstream_writer) = tokio::io::split(upstream);

    // `true` when the filter refused the session.
    let client_to_upstream = async {
        let mut buffer = vec![0u8; FILTER_READ_BUFFER];
        loop {
            let read = match client_reader.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => read,
                Err(err) => {
                    error!(%err, "error reading the upgraded client connection");
                    break;
                }
            };
            let chunk = &buffer[..read];
            if let Err(reason) = filter.inspect(chunk) {
                warn!(%reason, "closing a port-forward session outside the allowed ports");
                // A refused port is a `port_denied` with its port; only traffic
                // the filter cannot read is reported as `session_closed_by_filter`.
                match filter.refused_port() {
                    Some(port) => audit.emit_port_decision(PortDecision::Denied, Some(port)),
                    None => audit.emit_port_decision(PortDecision::ClosedByFilter, None),
                }
                return true;
            }
            for port in filter.take_opened() {
                audit.emit_port_decision(PortDecision::Allowed, Some(port));
            }
            if upstream_writer.write_all(chunk).await.is_err() {
                break;
            }
        }
        let _ = upstream_writer.shutdown().await;
        false
    };
    let upstream_to_client = tokio::io::copy(&mut upstream_reader, &mut client_writer);
    tokio::pin!(client_to_upstream, upstream_to_client);

    tokio::select! {
        refused = &mut client_to_upstream => {
            // The client finished sending: keep relaying the upstream's answer
            // until it ends. A refusal drops both connections right away.
            if !refused {
                let _ = upstream_to_client.await;
            }
        }
        // The upstream closed: the session is over.
        _ = &mut upstream_to_client => {}
    }
}
