//! A yamux connection driven by one future, opened from any number of
//! handles.
//!
//! yamux makes progress only while `poll_next_inbound` is polled, so a single
//! driver owns the connection: it accepts inbound streams into a channel and
//! serves the outbound requests of every [`MuxHandle`].

use std::collections::VecDeque;
use std::future::{Future, poll_fn};
use std::task::Poll;

use futures_util::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use yamux::{Config, Connection, ConnectionError};
pub use yamux::{Mode, Stream};

/// Per-stream receive window: the yamux default (RFC 0012 §7.5).
pub const STREAM_WINDOW: usize = yamux::DEFAULT_CREDIT as usize;
/// Inbound streams waiting for their consumer; past it, they are dropped.
const INBOUND_BACKLOG: usize = 64;

type OpenRequest = oneshot::Sender<Result<Stream, ConnectionError>>;

/// Opens outbound streams on a tunnel. Cheap to clone.
#[derive(Clone, Debug)]
pub struct MuxHandle {
    open: mpsc::Sender<OpenRequest>,
}

#[derive(Debug, thiserror::Error)]
pub enum MuxError {
    #[error("the tunnel is closed")]
    Closed,
    #[error("the tunnel refused a new stream: {0}")]
    Connection(#[from] ConnectionError),
}

impl MuxHandle {
    pub async fn open(&self) -> Result<Stream, MuxError> {
        let (tx, rx) = oneshot::channel();
        self.open.send(tx).await.map_err(|_| MuxError::Closed)?;
        Ok(rx.await.map_err(|_| MuxError::Closed)??)
    }

    /// Whether the driver has stopped.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.open.is_closed()
    }
}

/// The yamux configuration of both ends: `max_streams` streams of
/// [`STREAM_WINDOW`] each (`streamsPerTunnel`, RFC 0012 §4.1).
#[must_use]
pub fn config(max_streams: usize) -> Config {
    let mut config = Config::default();
    // Each setter asserts window >= streams * 256 KiB against the other's
    // current value: lift the window, set the count, then bound the window.
    config.set_max_connection_receive_window(None);
    config.set_max_num_streams(max_streams);
    config.set_max_connection_receive_window(Some(max_streams.saturating_mul(STREAM_WINDOW)));
    config
}

/// Wrap `io` in a yamux connection. The returned future drives it until the
/// peer closes or the connection fails; spawn it.
pub fn new<T>(
    io: T,
    mode: Mode,
    max_streams: usize,
) -> (
    MuxHandle,
    mpsc::Receiver<Stream>,
    impl Future<Output = Result<(), ConnectionError>>,
)
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let (open_tx, mut open_rx) = mpsc::channel::<OpenRequest>(16);
    let (inbound_tx, inbound_rx) = mpsc::channel(INBOUND_BACKLOG);
    let mut connection = Connection::new(io, config(max_streams), mode);
    let mut pending: VecDeque<OpenRequest> = VecDeque::new();

    let driver = poll_fn(move |cx| {
        while let Poll::Ready(Some(request)) = open_rx.poll_recv(cx) {
            pending.push_back(request);
        }
        while !pending.is_empty() {
            match connection.poll_new_outbound(cx) {
                Poll::Ready(result) => {
                    if let Some(request) = pending.pop_front() {
                        // The requester may have given up: the stream drops.
                        drop(request.send(result));
                    }
                }
                Poll::Pending => break,
            }
        }
        loop {
            match connection.poll_next_inbound(cx) {
                Poll::Ready(Some(Ok(stream))) => {
                    // A full backlog or a gone consumer resets the stream.
                    drop(inbound_tx.try_send(stream));
                }
                Poll::Ready(Some(Err(err))) => return Poll::Ready(Err(err)),
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
    });
    (MuxHandle { open: open_tx }, inbound_rx, driver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_util::compat::TokioAsyncReadCompatExt;

    fn pair() -> (
        (MuxHandle, mpsc::Receiver<Stream>),
        (MuxHandle, mpsc::Receiver<Stream>),
    ) {
        let (a, b) = tokio::io::duplex(1 << 20);
        let (proxy, proxy_in, proxy_driver) = new(a.compat(), Mode::Client, 8);
        let (agent, agent_in, agent_driver) = new(b.compat(), Mode::Server, 8);
        tokio::spawn(proxy_driver);
        tokio::spawn(agent_driver);
        ((proxy, proxy_in), (agent, agent_in))
    }

    #[tokio::test]
    async fn a_stream_opened_on_one_end_is_accepted_on_the_other() {
        let ((proxy, _), (_, mut agent_in)) = pair();
        let mut out = proxy.open().await.unwrap();
        out.write_all(b"ping").await.unwrap();
        out.close().await.unwrap();

        let mut inbound = agent_in.recv().await.unwrap();
        let mut got = Vec::new();
        inbound.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, b"ping");
    }

    #[tokio::test]
    async fn both_ends_open_streams_and_they_stay_apart() {
        let ((proxy, mut proxy_in), (agent, mut agent_in)) = pair();
        // The agent opens the control stream first, then the proxy data.
        let mut control = agent.open().await.unwrap();
        control.write_all(b"control").await.unwrap();
        control.close().await.unwrap();
        let mut data = proxy.open().await.unwrap();
        data.write_all(b"data").await.unwrap();
        data.close().await.unwrap();

        let mut got = Vec::new();
        proxy_in
            .recv()
            .await
            .unwrap()
            .read_to_end(&mut got)
            .await
            .unwrap();
        assert_eq!(got, b"control");
        got.clear();
        agent_in
            .recv()
            .await
            .unwrap()
            .read_to_end(&mut got)
            .await
            .unwrap();
        assert_eq!(got, b"data");
    }

    #[tokio::test]
    async fn a_closed_peer_closes_the_handles() {
        let (a, b) = tokio::io::duplex(1 << 16);
        let (proxy, _proxy_in, proxy_driver) = new(a.compat(), Mode::Client, 8);
        let driver = tokio::spawn(proxy_driver);
        drop(b);
        driver.await.unwrap().ok();
        assert!(proxy.is_closed());
        assert!(matches!(proxy.open().await, Err(MuxError::Closed)));
    }

    #[test]
    fn the_receive_window_covers_every_stream() {
        // `Config` asserts it; a wrong order of the setters would panic here.
        drop(config(512));
    }
}
