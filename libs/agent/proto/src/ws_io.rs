//! A WebSocket as a byte stream, for yamux.
//!
//! Binary frames carry the bytes; ping and pong are left to tungstenite; a
//! close frame, or the end of the stream, is the end of the bytes. A text
//! frame has no place in the tunnel and fails the read.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use bytes::{Buf, Bytes};
use futures_util::io::{AsyncRead, AsyncWrite};
use futures_util::{Sink, Stream};
pub use tokio_tungstenite::tungstenite::Message;

/// Largest payload one frame carries; a longer write is split.
pub const MAX_FRAME: usize = 16 * 1024;

pub struct WsIo<S> {
    inner: S,
    unread: Bytes,
    ended: bool,
}

impl<S> WsIo<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            unread: Bytes::new(),
            ended: false,
        }
    }
}

impl<S, E> AsyncRead for WsIo<S>
where
    S: Stream<Item = Result<Message, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            if !self.unread.is_empty() {
                let n = buf.len().min(self.unread.len());
                let (dst, _) = buf.split_at_mut(n);
                dst.copy_from_slice(self.unread.split_to(n).chunk());
                return Poll::Ready(Ok(n));
            }
            if self.ended {
                return Poll::Ready(Ok(0));
            }
            match ready!(Pin::new(&mut self.inner).poll_next(cx)) {
                Some(Ok(Message::Binary(data))) => self.unread = data,
                Some(Ok(Message::Close(_))) | None => self.ended = true,
                Some(Ok(Message::Text(_))) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "text frame in the agent tunnel",
                    )));
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
                Some(Err(err)) => return Poll::Ready(Err(io::Error::other(err))),
            }
        }
    }
}

impl<S, E> AsyncWrite for WsIo<S>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        ready!(Pin::new(&mut self.inner).poll_ready(cx)).map_err(io::Error::other)?;
        let (chunk, _) = buf.split_at(buf.len().min(MAX_FRAME));
        Pin::new(&mut self.inner)
            .start_send(Message::Binary(Bytes::copy_from_slice(chunk)))
            .map_err(io::Error::other)?;
        Poll::Ready(Ok(chunk.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner)
            .poll_flush(cx)
            .map_err(io::Error::other)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner)
            .poll_close(cx)
            .map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc;
    use futures::{SinkExt, StreamExt};
    use futures_util::io::{AsyncReadExt, AsyncWriteExt};

    /// A WebSocket stand-in: what is written comes out of `rx`, what is
    /// pushed into `tx` is read.
    type Fake = futures::stream::Map<
        mpsc::UnboundedReceiver<Message>,
        fn(Message) -> Result<Message, std::io::Error>,
    >;

    fn reader(messages: Vec<Message>) -> WsIo<Fake> {
        let (tx, rx) = mpsc::unbounded();
        for message in messages {
            tx.unbounded_send(message).unwrap();
        }
        tx.close_channel();
        WsIo::new(rx.map(Ok as fn(Message) -> Result<Message, std::io::Error>))
    }

    #[tokio::test]
    async fn binary_frames_are_read_as_one_byte_stream_until_close() {
        let mut io = reader(vec![
            Message::Binary(Bytes::from_static(b"hel")),
            Message::Ping(Bytes::new()),
            Message::Binary(Bytes::from_static(b"lo")),
            Message::Close(None),
            Message::Binary(Bytes::from_static(b"after close")),
        ]);
        let mut out = Vec::new();
        io.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, b"hello");
    }

    #[tokio::test]
    async fn a_text_frame_fails_the_read() {
        let mut io = reader(vec![Message::text("hi")]);
        let err = io.read_to_end(&mut Vec::new()).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn a_long_write_is_split_into_bounded_binary_frames() {
        let (tx, mut rx) = mpsc::unbounded::<Message>();
        let mut io = WsIo::new(tx.sink_map_err(std::io::Error::other));
        let payload = vec![7u8; MAX_FRAME * 2 + 10];
        io.write_all(&payload).await.unwrap();
        io.close().await.unwrap();

        let mut sizes = Vec::new();
        let mut joined = Vec::new();
        while let Some(message) = rx.next().await {
            let Message::Binary(data) = message else {
                panic!("only binary frames are written: {message:?}");
            };
            sizes.push(data.len());
            joined.extend_from_slice(&data);
        }
        assert_eq!(sizes, vec![MAX_FRAME, MAX_FRAME, 10]);
        assert_eq!(joined, payload);
    }
}
