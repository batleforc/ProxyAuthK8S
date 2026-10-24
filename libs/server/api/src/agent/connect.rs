//! The gateway's `CONNECT` listener (RFC 0012 decision 25, T18).
//!
//! The proxy asks for a connection to the apiserver of one `Agent` cluster:
//! `CONNECT <server_name>:443` with `Proxy-Authorization: Bearer <token>` and
//! `X-Proxyauth-Agent: <uid>`. The target in the request line is ignored: the
//! agent only knows its one upstream (§5.1), so the request cannot name
//! another. A stream to that agent is opened before the `200`, so a refusal
//! carries its reason: `407` without the token, `503` without a tunnel, `429`
//! at the stream limit.

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http::{HeaderMap, Method, Request, Response, StatusCode, header};
use http_body_util::Empty;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tracing::{debug, warn};

use super::registry::{OpenError, TunnelRegistry};

/// Names the cluster, by the UID of its `ProxyKubeApi`.
pub const AGENT_HEADER: &str = "x-proxyauth-agent";
/// Bound on a `CONNECT` request head: it carries a few headers only.
const MAX_HEAD_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub struct ConnectService {
    registry: Arc<TunnelRegistry>,
    /// `None`: every `CONNECT` is refused (no token configured).
    token: Option<Arc<str>>,
}

impl ConnectService {
    #[must_use]
    pub fn new(registry: Arc<TunnelRegistry>, token: Option<String>) -> Self {
        Self {
            registry,
            token: token.map(Arc::from),
        }
    }

    fn authorized(&self, headers: &HeaderMap) -> bool {
        let Some(expected) = &self.token else {
            return false;
        };
        let presented = headers
            .get(header::PROXY_AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .unwrap_or_default();
        presented.as_bytes().ct_eq(expected.as_bytes()).into()
    }

    async fn respond(&self, mut req: Request<Incoming>) -> Response<Empty<Bytes>> {
        if req.method() != Method::CONNECT {
            return status(StatusCode::METHOD_NOT_ALLOWED);
        }
        if !self.authorized(req.headers()) {
            warn!("CONNECT refused: missing or wrong proxy token");
            let mut response = status(StatusCode::PROXY_AUTHENTICATION_REQUIRED);
            response.headers_mut().insert(
                header::PROXY_AUTHENTICATE,
                http::HeaderValue::from_static("Bearer"),
            );
            return response;
        }
        let Some(uid) = req
            .headers()
            .get(AGENT_HEADER)
            .and_then(|value| value.to_str().ok())
            .filter(|uid| is_uid(uid))
            .map(ToString::to_string)
        else {
            return status(StatusCode::BAD_REQUEST);
        };
        let stream = match self.registry.open_stream(&uid).await {
            Ok(stream) => stream,
            Err(OpenError::Limited) => return status(StatusCode::TOO_MANY_REQUESTS),
            Err(err) => {
                debug!(%uid, error = %err, "CONNECT refused: no usable tunnel");
                return status(StatusCode::SERVICE_UNAVAILABLE);
            }
        };
        let on_upgrade = hyper::upgrade::on(&mut req);
        tokio::spawn(async move {
            match on_upgrade.await {
                Ok(upgraded) => {
                    let mut client = TokioIo::new(upgraded);
                    let mut agent = stream.compat();
                    if let Err(err) = tokio::io::copy_bidirectional(&mut client, &mut agent).await {
                        debug!(error = %err, "CONNECT relay ended");
                    }
                }
                Err(err) => debug!(error = %err, "CONNECT upgrade failed"),
            }
        });
        status(StatusCode::OK)
    }
}

fn status(code: StatusCode) -> Response<Empty<Bytes>> {
    let mut response = Response::new(Empty::new());
    *response.status_mut() = code;
    response
}

fn is_uid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// Serve `CONNECT` on `listener` until the task is dropped.
pub async fn serve(listener: TcpListener, service: ConnectService) {
    loop {
        let (io, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(err) => {
                warn!(error = %err, "CONNECT listener: accept failed");
                continue;
            }
        };
        let service = service.clone();
        tokio::spawn(async move {
            let handler = service_fn(move |req| {
                let service = service.clone();
                async move { Ok::<_, Infallible>(service.respond(req).await) }
            });
            if let Err(err) = http1::Builder::new()
                .max_buf_size(MAX_HEAD_BYTES)
                .serve_connection(TokioIo::new(io), handler)
                .with_upgrades()
                .await
            {
                debug!(%peer, error = %err, "CONNECT connection ended");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::registry::tests::{echo_tunnel, identity};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn gateway(token: Option<&str>) -> (std::net::SocketAddr, Vec<impl Drop>) {
        let registry = TunnelRegistry::new();
        let guard = registry
            .register(identity("aa"), echo_tunnel(), 4, 8)
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(serve(
            listener,
            ConnectService::new(registry, token.map(ToString::to_string)),
        ));
        (addr, vec![guard])
    }

    async fn connect(addr: std::net::SocketAddr, headers: &str) -> (String, tokio::net::TcpStream) {
        let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        tcp.write_all(
            format!(
                "CONNECT kubernetes.default.svc:443 HTTP/1.1\r\nHost: kubernetes.default.svc:443\r\n{headers}\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            tcp.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        (String::from_utf8(head).unwrap(), tcp)
    }

    #[tokio::test]
    async fn an_authorized_connect_is_spliced_to_the_agent() {
        let (addr, _guards) = gateway(Some("t0ken")).await;
        let (head, mut tcp) = connect(
            addr,
            "Proxy-Authorization: Bearer t0ken\r\nX-Proxyauth-Agent: aa\r\n",
        )
        .await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        tcp.write_all(b"tls records").await.unwrap();
        let mut echoed = [0u8; 11];
        tcp.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"tls records");
    }

    /// T18: another pod of the relay cluster, without the token.
    #[tokio::test]
    async fn a_connect_without_the_proxy_token_is_refused() {
        let (addr, _guards) = gateway(Some("t0ken")).await;
        for headers in [
            "X-Proxyauth-Agent: aa\r\n",
            "Proxy-Authorization: Bearer wrong\r\nX-Proxyauth-Agent: aa\r\n",
            "Proxy-Authorization: Basic dDBrZW4=\r\nX-Proxyauth-Agent: aa\r\n",
        ] {
            let (head, _) = connect(addr, headers).await;
            assert!(head.starts_with("HTTP/1.1 407"), "{headers}: {head}");
        }
        // No token configured on the gateway: nothing gets through.
        let (addr, _guards) = gateway(None).await;
        let (head, _) = connect(
            addr,
            "Proxy-Authorization: Bearer \r\nX-Proxyauth-Agent: aa\r\n",
        )
        .await;
        assert!(head.starts_with("HTTP/1.1 407"), "{head}");
    }

    #[tokio::test]
    async fn an_unknown_cluster_or_a_malformed_uid_is_refused() {
        let (addr, _guards) = gateway(Some("t0ken")).await;
        let (head, _) = connect(
            addr,
            "Proxy-Authorization: Bearer t0ken\r\nX-Proxyauth-Agent: bb\r\n",
        )
        .await;
        assert!(head.starts_with("HTTP/1.1 503"), "{head}");
        let (head, _) = connect(
            addr,
            "Proxy-Authorization: Bearer t0ken\r\nX-Proxyauth-Agent: ../etc\r\n",
        )
        .await;
        assert!(head.starts_with("HTTP/1.1 400"), "{head}");
    }
}
