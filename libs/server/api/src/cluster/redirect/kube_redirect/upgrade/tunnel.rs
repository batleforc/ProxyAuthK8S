//! Piping an upgraded connection between the client and the upstream.

use actix_web::{HttpResponse, web};
use futures_util::stream::StreamExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf},
    sync::{mpsc, oneshot},
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, warn};

use super::super::port_forward::PortForwardFilter;
use super::connect::BoxedAsyncIo;
use super::response::{UpstreamHead, copy_upstream_headers};

/// In-flight chunks buffered between the upgraded upstream connection and the
/// client. Bounded to keep exec/attach/port-forward sessions from growing
/// without limit when one side reads slower than the other.
const UPGRADE_CHANNEL_CAPACITY: usize = 32;

/// Answer the client with the upstream's `101` and pipe both directions.
///
/// `filter`, when set, inspects every client chunk of a restricted
/// port-forward and ends the session on a port outside the policy.
pub(super) async fn tunnel(
    upstream: BoxedAsyncIo,
    head: UpstreamHead,
    upgrade_protocol: String,
    payload: web::Payload,
    filter: Option<PortForwardFilter>,
) -> HttpResponse {
    let UpstreamHead {
        status,
        headers,
        leftover,
    } = head;
    let (upstream_reader, upstream_writer) = tokio::io::split(upstream);
    // Bounded so a slow client back-pressures the upstream reader instead of
    // letting the upgraded stream accumulate in memory.
    let (tx, rx) = mpsc::channel::<web::Bytes>(UPGRADE_CHANNEL_CAPACITY);

    if !leftover.is_empty() && tx.send(web::Bytes::from(leftover)).await.is_err() {
        return HttpResponse::ServiceUnavailable().body("client stream closed");
    }

    // Fired when the filter closes the session, to stop the upstream reader
    // too: both halves dropped close the upstream connection, and the client
    // stream ends with the reader.
    let (close_tx, close_rx) = oneshot::channel::<()>();

    actix_web::rt::spawn(client_to_upstream(
        payload,
        upstream_writer,
        filter,
        close_tx,
    ));
    actix_web::rt::spawn(upstream_to_client(upstream_reader, tx, close_rx));

    let mut client_resp = HttpResponse::build(status);
    client_resp.upgrade(upgrade_protocol);
    copy_upstream_headers(&mut client_resp, headers);

    client_resp.streaming(ReceiverStream::new(rx).map(Ok::<web::Bytes, actix_web::Error>))
}

/// Copy the client's bytes upstream until either side ends, or the filter
/// refuses a chunk.
async fn client_to_upstream(
    payload: web::Payload,
    mut upstream_writer: WriteHalf<BoxedAsyncIo>,
    mut filter: Option<PortForwardFilter>,
    close_tx: oneshot::Sender<()>,
) {
    let mut client_payload = payload.into_inner();
    while let Some(item) = client_payload.next().await {
        match item {
            Ok(chunk) => {
                if let Some(filter) = filter.as_mut()
                    && let Err(reason) = filter.inspect(&chunk)
                {
                    warn!(%reason, "closing a port-forward session outside the allowed ports");
                    let _ = close_tx.send(());
                    return;
                }
                if upstream_writer.write_all(&chunk).await.is_err() {
                    break;
                }
            }
            Err(err) => {
                error!(%err, "error reading upgraded client payload");
                break;
            }
        }
    }

    let _ = upstream_writer.shutdown().await;
}

/// Copy the upstream's bytes to the client stream until either side ends or
/// the session is closed by the filter.
async fn upstream_to_client(
    mut upstream_reader: ReadHalf<BoxedAsyncIo>,
    tx: mpsc::Sender<web::Bytes>,
    mut close_rx: oneshot::Receiver<()>,
) {
    let mut buffer = [0u8; 8192];
    loop {
        let read = tokio::select! {
            read = upstream_reader.read(&mut buffer) => read,
            _ = &mut close_rx => break,
        };
        match read {
            Ok(0) => break,
            Ok(read) => {
                if tx
                    .send(web::Bytes::copy_from_slice(&buffer[..read]))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(err) => {
                error!(%err, "error reading upgraded upstream payload");
                break;
            }
        }
    }
}
