//! The tunnels this gateway pod holds (RFC 0012 §6.3, decision 26).
//!
//! Keyed by the `ProxyKubeApi` UID of the agent identity. A stream goes to the
//! tunnel of that UID carrying the fewest open streams. A tunnel leaves the
//! table when its [`TunnelGuard`] drops, which the connection task holds for
//! as long as the tunnel lives.

use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use agent_pki::AgentIdentity;
use agent_proto::mux::{MuxError, MuxHandle};
use futures_util::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("{0} tunnels are already open for this agent identity")]
    TooManyTunnels(usize),
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("no tunnel for this cluster")]
    NoTunnel,
    #[error("every tunnel of this cluster is at its stream limit")]
    Limited,
    #[error(transparent)]
    Mux(#[from] MuxError),
}

struct Tunnel {
    id: u64,
    identity: AgentIdentity,
    handle: MuxHandle,
    streams: Arc<AtomicUsize>,
    max_streams: usize,
    /// Set to `true` to ask the connection task to close the tunnel.
    close: watch::Sender<bool>,
}

#[derive(Default)]
pub struct TunnelRegistry {
    tunnels: Mutex<HashMap<String, Vec<Tunnel>>>,
    next_id: AtomicU64,
}

/// Keeps a tunnel registered; dropping it removes the tunnel.
#[must_use = "the tunnel is unregistered as soon as the guard drops"]
pub struct TunnelGuard {
    registry: Arc<TunnelRegistry>,
    uid: String,
    id: u64,
    /// Becomes `true` when the registry wants this tunnel closed (`epoch`
    /// raised, CR deleted, gateway shutting down).
    pub closed: watch::Receiver<bool>,
}

impl Drop for TunnelGuard {
    fn drop(&mut self) {
        let mut tunnels = self
            .registry
            .tunnels
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(list) = tunnels.get_mut(&self.uid) {
            list.retain(|tunnel| tunnel.id != self.id);
            if list.is_empty() {
                tunnels.remove(&self.uid);
            }
        }
    }
}

impl TunnelRegistry {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Add a tunnel for `identity`, refused past `max_tunnels` for its UID
    /// (T11). `max_streams` bounds the streams opened on it.
    pub fn register(
        self: &Arc<Self>,
        identity: AgentIdentity,
        handle: MuxHandle,
        max_tunnels: usize,
        max_streams: usize,
    ) -> Result<TunnelGuard, RegistryError> {
        let mut tunnels = self.tunnels.lock().unwrap_or_else(PoisonError::into_inner);
        let list = tunnels.entry(identity.uid.clone()).or_default();
        if list.len() >= max_tunnels {
            return Err(RegistryError::TooManyTunnels(list.len()));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (close, closed) = watch::channel(false);
        let uid = identity.uid.clone();
        list.push(Tunnel {
            id,
            identity,
            handle,
            streams: Arc::new(AtomicUsize::new(0)),
            max_streams,
            close,
        });
        Ok(TunnelGuard {
            registry: Arc::clone(self),
            uid,
            id,
            closed,
        })
    }

    /// A new stream to the agent of `uid`, on its least loaded tunnel.
    pub async fn open_stream(&self, uid: &str) -> Result<CountedStream, OpenError> {
        let (handle, streams) = {
            let tunnels = self.tunnels.lock().unwrap_or_else(PoisonError::into_inner);
            let list = tunnels.get(uid).ok_or(OpenError::NoTunnel)?;
            let tunnel = list
                .iter()
                .filter(|tunnel| !tunnel.handle.is_closed())
                .min_by_key(|tunnel| tunnel.streams.load(Ordering::Relaxed))
                .ok_or(OpenError::NoTunnel)?;
            // Reserved under the lock, so two opens cannot both take the last slot.
            let reserved = tunnel
                .streams
                .try_update(Ordering::AcqRel, Ordering::Acquire, |open| {
                    (open < tunnel.max_streams).then(|| open.saturating_add(1))
                });
            if reserved.is_err() {
                return Err(OpenError::Limited);
            }
            (tunnel.handle.clone(), Arc::clone(&tunnel.streams))
        };
        let slot = StreamSlot(streams);
        let stream = handle.open().await?;
        Ok(CountedStream {
            inner: stream,
            _slot: slot,
        })
    }

    /// Every identity with its number of tunnels, for the presence records.
    #[must_use]
    pub fn identities(&self) -> Vec<(AgentIdentity, usize)> {
        let tunnels = self.tunnels.lock().unwrap_or_else(PoisonError::into_inner);
        tunnels
            .values()
            .filter_map(|list| {
                list.first()
                    .map(|first| (first.identity.clone(), list.len()))
            })
            .collect()
    }

    /// Ask every tunnel whose identity `revoked` returns `true` for to close
    /// (T8); returns how many were asked.
    pub fn close_where(&self, revoked: impl Fn(&AgentIdentity) -> bool) -> usize {
        let tunnels = self.tunnels.lock().unwrap_or_else(PoisonError::into_inner);
        let mut count = 0usize;
        for tunnel in tunnels.values().flatten() {
            if revoked(&tunnel.identity) {
                // An error means the connection task is already gone.
                let _already_closed = tunnel.close.send(true);
                count = count.saturating_add(1);
            }
        }
        count
    }
}

/// One stream slot of a tunnel, given back on drop.
struct StreamSlot(Arc<AtomicUsize>);

impl Drop for StreamSlot {
    fn drop(&mut self) {
        // Never below zero: the slot was taken before this value existed.
        let _released = self
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |open| {
                open.checked_sub(1)
            });
    }
}

/// A yamux stream that gives its tunnel's slot back when it drops.
pub struct CountedStream {
    inner: agent_proto::mux::Stream,
    _slot: StreamSlot,
}

impl AsyncRead for CountedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for CountedStream {
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

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use agent_proto::mux;
    use futures_util::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_util::compat::TokioAsyncReadCompatExt;

    pub(crate) fn identity(uid: &str) -> AgentIdentity {
        AgentIdentity {
            namespace: "team-a".into(),
            name: "prod".into(),
            uid: uid.into(),
            epoch: 0,
        }
    }

    /// A tunnel whose agent end echoes every stream back.
    pub(crate) fn echo_tunnel() -> MuxHandle {
        let (gateway_io, agent_io) = tokio::io::duplex(1 << 20);
        let (handle, _inbound, driver) = mux::new(gateway_io.compat(), mux::Mode::Client, 8);
        let (_agent, mut agent_in, agent_driver) =
            mux::new(agent_io.compat(), mux::Mode::Server, 8);
        tokio::spawn(driver);
        tokio::spawn(agent_driver);
        tokio::spawn(async move {
            while let Some(stream) = agent_in.recv().await {
                tokio::spawn(async move {
                    let (mut read, mut write) = stream.split();
                    drop(futures_util::io::copy(&mut read, &mut write).await);
                });
            }
        });
        handle
    }

    #[tokio::test]
    async fn a_stream_reaches_the_agent_of_its_uid() {
        let registry = TunnelRegistry::new();
        let _guard = registry
            .register(identity("aa"), echo_tunnel(), 4, 8)
            .unwrap();
        let mut stream = registry.open_stream("aa").await.unwrap();
        stream.write_all(b"ping").await.unwrap();
        let mut got = [0u8; 4];
        stream.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"ping");
        assert!(matches!(
            registry.open_stream("bb").await,
            Err(OpenError::NoTunnel)
        ));
    }

    #[tokio::test]
    async fn the_tunnel_and_stream_limits_hold() {
        let registry = TunnelRegistry::new();
        let _one = registry
            .register(identity("aa"), echo_tunnel(), 1, 2)
            .unwrap();
        assert_eq!(
            registry.register(identity("aa"), echo_tunnel(), 1, 2).err(),
            Some(RegistryError::TooManyTunnels(1))
        );
        let first = registry.open_stream("aa").await.unwrap();
        let _second = registry.open_stream("aa").await.unwrap();
        assert!(matches!(
            registry.open_stream("aa").await,
            Err(OpenError::Limited)
        ));
        // A closed stream gives its slot back.
        drop(first);
        registry.open_stream("aa").await.unwrap();
    }

    #[tokio::test]
    async fn streams_go_to_the_least_loaded_tunnel() {
        let registry = TunnelRegistry::new();
        let _a = registry
            .register(identity("aa"), echo_tunnel(), 4, 8)
            .unwrap();
        let _b = registry
            .register(identity("aa"), echo_tunnel(), 4, 8)
            .unwrap();
        let _s1 = registry.open_stream("aa").await.unwrap();
        let _s2 = registry.open_stream("aa").await.unwrap();
        let loads: Vec<usize> = registry
            .tunnels
            .lock()
            .unwrap()
            .get("aa")
            .unwrap()
            .iter()
            .map(|tunnel| tunnel.streams.load(Ordering::Relaxed))
            .collect();
        assert_eq!(loads, vec![1, 1]);
    }

    #[tokio::test]
    async fn a_dropped_guard_unregisters_and_close_where_signals() {
        let registry = TunnelRegistry::new();
        let guard = registry
            .register(identity("aa"), echo_tunnel(), 4, 8)
            .unwrap();
        assert_eq!(registry.identities().len(), 1);
        assert_eq!(registry.close_where(|id| id.uid == "aa"), 1);
        assert!(*guard.closed.borrow());
        drop(guard);
        assert!(registry.identities().is_empty());
        assert!(matches!(
            registry.open_stream("aa").await,
            Err(OpenError::NoTunnel)
        ));
    }
}
