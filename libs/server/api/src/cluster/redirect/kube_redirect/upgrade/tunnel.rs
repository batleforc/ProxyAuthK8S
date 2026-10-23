//! Piping an upgraded connection between the client and the upstream.
//!
//! Both handshakes are hyper's: the client's `101` is the response the
//! handler returns, and both connections are taken over with
//! `hyper::upgrade::on`, so no byte of either side is parsed here.
//!
//! When the session ends, one `session_closed` audit line (RFC 0008 phase 4)
//! records its duration, the bytes each way and who closed it.

use std::{
    io,
    pin::Pin,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::Instant,
};

use axum::{
    body::Body,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tracing::{Instrument as _, error, warn};

use std::collections::BTreeMap;

use super::super::exec::{ExecChannelCounter, ExecChannelFilter};
use super::super::port_forward::PortForwardFilter;
use crate::cluster::redirect::audit::{self, AuditContext, PortDecision};

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

/// One side of a tunnel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Client,
    Upstream,
}

/// What a session's two streams counted while it ran.
#[derive(Default)]
struct SessionCounters {
    /// Bytes read from the client, so sent towards the upstream.
    bytes_in: AtomicU64,
    /// Bytes read from the upstream, so sent towards the client.
    bytes_out: AtomicU64,
    /// The side whose stream ended first.
    first_eof: OnceLock<Side>,
}

/// A stream that counts the bytes read from it, and records on the shared
/// counters whether it ended before the other side.
struct Counted<T> {
    inner: T,
    side: Side,
    counters: Arc<SessionCounters>,
}

impl<T> Counted<T> {
    fn new(inner: T, side: Side, counters: &Arc<SessionCounters>) -> Self {
        Self {
            inner,
            side,
            counters: Arc::clone(counters),
        }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for Counted<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &poll {
            let read = buf.filled().len().saturating_sub(before) as u64;
            if read == 0 {
                // Only the first side to end is recorded; a later one is not.
                let _already_set = self.counters.first_eof.set(self.side);
            }
            let total = match self.side {
                Side::Client => &self.counters.bytes_in,
                Side::Upstream => &self.counters.bytes_out,
            };
            total.fetch_add(read, Ordering::Relaxed);
        }
        poll
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Counted<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// How a session ended, the `close_reason` of its audit line.
fn close_reason(refused: bool, failed: bool, first_eof: Option<Side>) -> &'static str {
    match (refused, failed, first_eof) {
        (true, _, _) => "filter",
        (_, true, _) => "error",
        (_, _, Some(Side::Client)) => "client",
        (_, _, Some(Side::Upstream)) => "upstream",
        (_, _, None) => "error",
    }
}

/// What reads a restricted session.
pub(in super::super) enum SessionFilter {
    /// A port-forward under a port restriction (RFC 0003).
    Port(PortForwardFilter),
    /// An exec or attach under an exec restriction (RFC 0004): the client
    /// half refuses forbidden channels, the upstream half only counts.
    Exec(ExecChannelFilter, ExecChannelCounter),
}

/// How a filtered session ended.
#[derive(Default)]
struct FilteredEnd {
    refused: bool,
    /// The forbidden channel that ended an exec.
    channel: Option<u8>,
    /// The bytes of each channel of an exec, both ways.
    channel_bytes: Option<BTreeMap<u8, u64>>,
}

/// Answer the client with the upstream's `101`, and pipe both directions once
/// both connections are upgraded.
///
/// `filter`, when set, inspects every client chunk of a restricted session and
/// ends it on a port or a channel outside the policy; each port it opens, and
/// the closing, are recorded on `audit`. The session's end is recorded there
/// too, with the bytes of each channel of an exec.
pub(super) fn tunnel(
    client: OnUpgrade,
    upstream: OnUpgrade,
    upstream_headers: &HeaderMap,
    upgrade_protocol: &str,
    audit: AuditContext,
    filter: Option<SessionFilter>,
) -> Response {
    let Ok(upgrade_protocol) = header::HeaderValue::from_str(upgrade_protocol) else {
        return (StatusCode::BAD_GATEWAY, "bad gateway").into_response();
    };
    // The task outlives the handler: it takes the request scope (request id,
    // client address) and the span along, for its audit lines.
    let request = audit::current_request().unwrap_or_default();
    let session = async move {
        let started = Instant::now();
        let counters = Arc::new(SessionCounters::default());
        let (end, failed) = match tokio::try_join!(client, upstream) {
            Err(err) => {
                error!(%err, "an upgraded connection could not be taken over");
                (FilteredEnd::default(), true)
            }
            Ok((client, upstream)) => {
                let client = Counted::new(TokioIo::new(client), Side::Client, &counters);
                let upstream = Counted::new(TokioIo::new(upstream), Side::Upstream, &counters);
                match filter {
                    Some(filter) => (pipe_filtered(client, upstream, filter, &audit).await, false),
                    None => {
                        let (mut client, mut upstream) = (client, upstream);
                        match tokio::io::copy_bidirectional(&mut client, &mut upstream).await {
                            Ok(_) => (FilteredEnd::default(), false),
                            Err(err) => {
                                warn!(%err, "upgraded connection ended with an error");
                                (FilteredEnd::default(), true)
                            }
                        }
                    }
                }
            }
        };
        crate::metrics::record_session_duration(
            super::super::exec::session_kind(&audit.path),
            started.elapsed().as_secs_f64(),
        );
        audit.emit_session_closed(
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            counters.bytes_in.load(Ordering::Relaxed),
            counters.bytes_out.load(Ordering::Relaxed),
            close_reason(end.refused, failed, counters.first_eof.get().copied()),
            end.channel_bytes.as_ref(),
            end.channel,
        );
    };
    tokio::spawn(audit::scope(request, session).instrument(tracing::Span::current()));

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
    mut filter: SessionFilter,
    audit: &AuditContext,
) -> FilteredEnd
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: AsyncRead + AsyncWrite + Unpin,
{
    let (mut client_reader, mut client_writer) = tokio::io::split(client);
    let (mut upstream_reader, mut upstream_writer) = tokio::io::split(upstream);
    // Each direction borrows its own half of the filter.
    let (port, exec, counter) = match &mut filter {
        SessionFilter::Port(port) => (Some(port), None, None),
        SessionFilter::Exec(exec, counter) => (None, Some(exec), Some(counter)),
    };
    let mut refused_channel = None;

    let refused = {
        // `true` when the filter refused the session.
        let client_to_upstream = async {
            let mut buffer = vec![0u8; FILTER_READ_BUFFER];
            let (mut port, mut exec) = (port, exec);
            loop {
                let read = match client_reader.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(err) => {
                        error!(%err, "error reading the upgraded client connection");
                        break;
                    }
                };
                let Some(chunk) = buffer.get(..read) else {
                    break;
                };
                if let Some(filter) = port.as_deref_mut() {
                    if let Err(reason) = filter.inspect(chunk) {
                        warn!(%reason, "closing a port-forward session outside the allowed ports");
                        // A refused port is a `port_denied` with its port; only
                        // traffic the filter cannot read is `closed_by_filter`.
                        match filter.refused_port() {
                            Some(port) => {
                                audit.emit_port_decision(PortDecision::Denied, Some(port))
                            }
                            None => audit.emit_port_decision(PortDecision::ClosedByFilter, None),
                        }
                        return true;
                    }
                    for port in filter.take_opened() {
                        audit.emit_port_decision(PortDecision::Allowed, Some(port));
                    }
                }
                if let Some(filter) = exec.as_deref_mut()
                    && let Err(reason) = filter.inspect(chunk)
                {
                    warn!(%reason, "closing an exec session on a forbidden channel");
                    refused_channel = filter.refused_channel();
                    return true;
                }
                if upstream_writer.write_all(chunk).await.is_err() {
                    break;
                }
            }
            // The session ends either way; a failed shutdown changes nothing.
            let _shutdown = upstream_writer.shutdown().await;
            false
        };
        let upstream_to_client = async {
            let mut counter = counter;
            let mut buffer = vec![0u8; FILTER_READ_BUFFER];
            loop {
                let read = match upstream_reader.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                let Some(chunk) = buffer.get(..read) else {
                    break;
                };
                if let Some(counter) = counter.as_deref_mut() {
                    counter.count(chunk);
                }
                if client_writer.write_all(chunk).await.is_err() {
                    break;
                }
            }
            // The session ends either way; a failed flush changes nothing.
            let _flush = client_writer.flush().await;
        };
        tokio::pin!(client_to_upstream, upstream_to_client);

        tokio::select! {
            refused = &mut client_to_upstream => {
                // The client finished sending: keep relaying the upstream's
                // answer until it ends. A refusal drops both connections.
                if !refused {
                    upstream_to_client.await;
                }
                refused
            }
            // The upstream closed: the session is over.
            () = &mut upstream_to_client => false,
        }
    };

    let channel_bytes = match &filter {
        SessionFilter::Port(_) => None,
        SessionFilter::Exec(exec, counter) => {
            let mut bytes = exec.bytes().clone();
            for (channel, count) in counter.bytes() {
                let total = bytes.entry(*channel).or_default();
                *total = total.saturating_add(*count);
            }
            Some(bytes)
        }
    };
    FilteredEnd {
        refused,
        channel: refused_channel,
        channel_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_close_reason_names_who_ended_the_session() {
        assert_eq!(close_reason(true, false, Some(Side::Client)), "filter");
        assert_eq!(close_reason(false, true, Some(Side::Client)), "error");
        assert_eq!(close_reason(false, false, Some(Side::Client)), "client");
        assert_eq!(close_reason(false, false, Some(Side::Upstream)), "upstream");
        assert_eq!(close_reason(false, false, None), "error");
    }

    #[tokio::test]
    async fn counted_streams_count_each_way_and_the_first_end() {
        let counters = Arc::new(SessionCounters::default());
        let (client_side, mut client_peer) = tokio::io::duplex(64);
        let (upstream_side, mut upstream_peer) = tokio::io::duplex(64);
        let mut client = Counted::new(client_side, Side::Client, &counters);
        let mut upstream = Counted::new(upstream_side, Side::Upstream, &counters);

        let peers = async {
            client_peer.write_all(b"hello").await.unwrap();
            let mut got = [0u8; 5];
            upstream_peer.read_exact(&mut got).await.unwrap();
            upstream_peer.write_all(b"answer!").await.unwrap();
            let mut back = [0u8; 7];
            client_peer.read_exact(&mut back).await.unwrap();
            // The client leaves first.
            drop(client_peer);
            upstream_peer
        };
        let (copied, _upstream_peer) = tokio::join!(
            tokio::io::copy_bidirectional(&mut client, &mut upstream),
            async {
                let peer = peers.await;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                drop(peer);
            }
        );
        copied.unwrap();
        assert_eq!(counters.bytes_in.load(Ordering::Relaxed), 5);
        assert_eq!(counters.bytes_out.load(Ordering::Relaxed), 7);
        assert_eq!(counters.first_eof.get(), Some(&Side::Client));
    }
}
