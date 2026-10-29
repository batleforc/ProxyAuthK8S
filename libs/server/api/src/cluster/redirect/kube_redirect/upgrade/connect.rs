//! Opening the upstream connection of an upgrade, and the handshake request
//! hyper sends on it (RFC 0007: no hand-written HTTP).

use std::time::Duration;

use std::net::IpAddr;

use axum::http::{self, HeaderMap, HeaderValue, Request};
use bytes::Bytes;
use common::State;
use crd::ProxyKubeApi;
use crd::service::Service;
use http_body_util::Empty;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};
use tokio_rustls::TlsConnector;

use super::super::tls::cached_tls_config;
use crate::cluster::redirect::forwarded::{
    forwarded_for_value, is_proxy_owned_header, is_upstream_auth_header,
};
use crate::cluster::redirect::upstream_identity::Upstream;

/// Bound on each of the TCP connect and the TLS handshake to the upstream, so a
/// black-holed apiserver does not pin the upgrade handler. The established
/// stream itself is not time-limited (exec/attach/port-forward are long-lived).
const UPGRADE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) trait AsyncIo: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> AsyncIo for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub(super) type BoxedAsyncIo = Box<dyn AsyncIo>;

/// The host (IPv6 brackets stripped) and port to dial for `upstream_url`.
fn upstream_host_port(upstream_url: &reqwest::Url) -> Result<(&str, u16), String> {
    let host = upstream_url
        .host_str()
        .ok_or_else(|| "missing upstream host".to_string())?;
    // `host_str` keeps the brackets of an IPv6 literal (`[::1]`), which neither
    // `TcpStream::connect` nor `ServerName` accept.
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let port = upstream_url
        .port_or_known_default()
        .ok_or_else(|| "missing upstream port".to_string())?;
    Ok((host, port))
}

/// Connect to the upstream, over TLS unless the URL is plain `http`.
pub(super) async fn connect_upgrade_stream(
    proxy: &ProxyKubeApi,
    state: &State,
    upstream_url: &reqwest::Url,
) -> Result<BoxedAsyncIo, String> {
    let (host, port) = upstream_host_port(upstream_url)?;

    let stream: BoxedAsyncIo = if matches!(proxy.spec.service, Service::Agent(_)) {
        // RFC 0012: never a socket to `host`, only the gateway's tunnel.
        Box::new(
            tokio::time::timeout(
                UPGRADE_CONNECT_TIMEOUT,
                connect_through_gateway(proxy, state, host, port),
            )
            .await
            .map_err(|_| "agent gateway connect timed out".to_string())??,
        )
    } else {
        Box::new(
            tokio::time::timeout(UPGRADE_CONNECT_TIMEOUT, TcpStream::connect((host, port)))
                .await
                .map_err(|_| "upstream connect timed out".to_string())?
                .map_err(|e| e.to_string())?,
        )
    };

    if upstream_url.scheme().eq_ignore_ascii_case("http") {
        return Ok(stream);
    }

    let tls_config = cached_tls_config(proxy, state).await?;
    let server_name = ServerName::try_from(host.to_string()).map_err(|e| e.to_string())?;
    let connector = TlsConnector::from(tls_config);
    let tls_stream = tokio::time::timeout(
        UPGRADE_CONNECT_TIMEOUT,
        connector.connect(server_name, stream),
    )
    .await
    .map_err(|_| "upstream TLS handshake timed out".to_string())?
    .map_err(|e| e.to_string())?;

    Ok(Box::new(tls_stream) as BoxedAsyncIo)
}

/// A byte stream to the apiserver of an `Agent` cluster: a `CONNECT`, sent
/// by hyper, to the gateway pod holding its tunnel (RFC 0012, decision 25).
async fn connect_through_gateway(
    proxy: &ProxyKubeApi,
    state: &State,
    host: &str,
    port: u16,
) -> Result<TokioIo<hyper::upgrade::Upgraded>, String> {
    let uid = proxy
        .metadata
        .uid
        .as_deref()
        .ok_or_else(|| "agent cluster without a uid".to_string())?;
    let route = state
        .agent_gateway
        .route(state, uid)
        .await
        .map_err(|err| err.to_string())?;
    let tcp = TcpStream::connect(route.address.as_str())
        .await
        .map_err(|err| {
            // That pod may be gone: read the presences again next time.
            state.agent_gateway.forget(uid);
            format!("agent gateway unreachable: {err}")
        })?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
        .await
        .map_err(|err| err.to_string())?;
    tokio::spawn(async move {
        if let Err(err) = connection.with_upgrades().await {
            tracing::debug!(error = %err, "agent gateway CONNECT connection ended");
        }
    });
    let mut request = Request::builder()
        .method(http::Method::CONNECT)
        .uri(format!("{host}:{port}"))
        .body(Empty::<Bytes>::new())
        .map_err(|err| err.to_string())?;
    request.headers_mut().extend(route.headers);
    let response = sender
        .send_request(request)
        .await
        .map_err(|err| err.to_string())?;
    if response.status() != http::StatusCode::OK {
        state.agent_gateway.forget(uid);
        return Err(format!("agent gateway refused: {}", response.status()));
    }
    let upgraded = hyper::upgrade::on(response)
        .await
        .map_err(|err| err.to_string())?;
    Ok(TokioIo::new(upgraded))
}

/// Whether a client header is left out of the forwarded handshake: the
/// rewritten host, the connection-management and framing headers (rebuilt, or
/// meaningless without a body) and the identity headers the proxy owns.
fn is_dropped_request_header(header_name: &http::header::HeaderName) -> bool {
    *header_name == http::header::HOST
        || *header_name == http::header::CONNECTION
        || *header_name == http::header::UPGRADE
        || header_name.as_str().eq_ignore_ascii_case("keep-alive")
        || header_name
            .as_str()
            .eq_ignore_ascii_case("proxy-connection")
        || *header_name == http::header::CONTENT_LENGTH
        || *header_name == http::header::TRANSFER_ENCODING
        || is_proxy_owned_header(header_name.as_str())
        || is_upstream_auth_header(header_name.as_str())
}

/// The bearer-token entries of `Sec-WebSocket-Protocol`, the apiserver's
/// `base64url.bearer.authorization.k8s.io.<token>` (RFC 0014, decision 39).
const BEARER_SUBPROTOCOL_PREFIX: &str = "base64url.bearer.authorization.k8s.io.";

/// `Sec-WebSocket-Protocol` without its bearer-token entries, in every mode:
/// the proxy validates `Authorization` only, so a second token there was
/// never checked, and in a trusted mode it would carry the caller's IdP token
/// upstream (or let the apiserver fall back to it, its union of
/// authenticators going on after a refusal). `None` when nothing is left.
fn strip_bearer_subprotocols(value: &HeaderValue) -> Option<HeaderValue> {
    let kept: Vec<&str> = value
        .to_str()
        .ok()?
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && !entry.starts_with(BEARER_SUBPROTOCOL_PREFIX))
        .collect();
    if kept.is_empty() {
        return None;
    }
    HeaderValue::from_str(&kept.join(", ")).ok()
}

/// The handshake sent upstream, serialized by hyper: the request line of
/// `upstream_url`, its `Host`, the rebuilt `Connection`/`Upgrade`, the client's
/// headers minus the dropped ones, the forwarding chain and the proxy's
/// identity headers. No body: the handshake never carries one.
pub(super) fn build_upgrade_request(
    headers: &HeaderMap,
    method: &http::Method,
    upstream_url: &reqwest::Url,
    peer_ip: Option<IpAddr>,
    upstream: Upstream<'_>,
) -> Result<Request<Empty<Bytes>>, String> {
    let path = match upstream_url.query() {
        Some(query) => format!("{}?{}", upstream_url.path(), query),
        None => upstream_url.path().to_string(),
    };
    let authority = upstream_url.port().map_or_else(
        || upstream_url.host_str().unwrap_or_default().to_string(),
        |port| format!("{}:{}", upstream_url.host_str().unwrap_or_default(), port),
    );
    let mut request = Request::builder()
        .method(method.clone())
        .uri(path)
        .body(Empty::new())
        .map_err(|e| e.to_string())?;
    let upstream_headers = request.headers_mut();
    upstream_headers.insert(
        http::header::HOST,
        HeaderValue::from_str(&authority).map_err(|e| e.to_string())?,
    );
    // The connection-management headers are rebuilt rather than copied, so the
    // client cannot steer how the upstream treats the connection.
    upstream_headers.insert(
        http::header::CONNECTION,
        HeaderValue::from_static("Upgrade"),
    );
    if let Some(upgrade) = headers.get(http::header::UPGRADE) {
        upstream_headers.insert(http::header::UPGRADE, upgrade.clone());
    }
    for (name, value) in headers {
        if is_dropped_request_header(name)
            || (upstream.drops_authorization() && *name == http::header::AUTHORIZATION)
        {
            continue;
        }
        if *name == http::header::SEC_WEBSOCKET_PROTOCOL {
            if let Some(value) = strip_bearer_subprotocols(value) {
                upstream_headers.append(name, value);
            }
            continue;
        }
        upstream_headers.append(name, value.clone());
    }

    let incoming_forwarded_for = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok());
    if let Some(forwarded_for) = forwarded_for_value(incoming_forwarded_for, peer_ip) {
        upstream_headers.insert(
            "x-forwarded-for",
            HeaderValue::from_str(&forwarded_for).map_err(|e| e.to_string())?,
        );
    }
    // Appended, never inserted: one `X-Remote-Group` per group.
    for (name, value) in upstream.headers() {
        let mut value = HeaderValue::from_str(&value).map_err(|e| e.to_string())?;
        // The `Issuer` JWT never shows in a debug print of the request (T10).
        value.set_sensitive(name.eq_ignore_ascii_case("authorization"));
        upstream_headers.append(
            http::header::HeaderName::from_bytes(name.as_bytes()).map_err(|e| e.to_string())?,
            value,
        );
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> reqwest::Url {
        reqwest::Url::parse(raw).expect("valid test url")
    }

    #[test]
    fn host_and_port_default_from_the_scheme() {
        let https = url("https://api.example:6443/x");
        assert_eq!(upstream_host_port(&https).unwrap(), ("api.example", 6443));
        let http = url("http://api.example/x");
        assert_eq!(upstream_host_port(&http).unwrap(), ("api.example", 80));
        let tls = url("https://api.example/x");
        assert_eq!(upstream_host_port(&tls).unwrap(), ("api.example", 443));
    }

    #[test]
    fn ipv6_hosts_lose_their_brackets() {
        let v6 = url("https://[::1]:8443/x");
        assert_eq!(upstream_host_port(&v6).unwrap(), ("::1", 8443));
    }

    #[test]
    fn a_url_without_a_host_is_refused() {
        let no_host = url("unix:/run/sock");
        assert_eq!(
            upstream_host_port(&no_host).unwrap_err(),
            "missing upstream host"
        );
    }

    #[test]
    fn the_request_line_keeps_the_query_and_explicit_port() {
        let req = build_upgrade_request(
            &HeaderMap::new(),
            &http::Method::GET,
            &url("https://api.example:6443/api/v1/pods/p/exec?command=ls"),
            None,
            crate::cluster::redirect::upstream_identity::Upstream::Passthrough(None),
        )
        .unwrap();
        assert_eq!(req.method(), http::Method::GET);
        assert_eq!(req.uri(), "/api/v1/pods/p/exec?command=ls");
        assert_eq!(req.headers()["host"], "api.example:6443");
        let req = build_upgrade_request(
            &HeaderMap::new(),
            &http::Method::POST,
            &url("https://api.example/x"),
            None,
            crate::cluster::redirect::upstream_identity::Upstream::Passthrough(None),
        )
        .unwrap();
        assert_eq!(req.method(), http::Method::POST);
        assert_eq!(req.uri(), "/x");
        assert_eq!(req.headers()["host"], "api.example");
    }

    #[test]
    fn framing_and_connection_headers_are_dropped() {
        for name in [
            "host",
            "connection",
            "upgrade",
            "keep-alive",
            "proxy-connection",
            "content-length",
            "transfer-encoding",
            "x-forwarded-user",
            "impersonate-user",
        ] {
            let name = http::header::HeaderName::from_static(name);
            assert!(is_dropped_request_header(&name), "{name} must be dropped");
        }
        let kept = http::header::HeaderName::from_static("sec-websocket-protocol");
        assert!(!is_dropped_request_header(&kept));
    }

    #[test]
    fn the_handshake_rebuilds_connection_headers_and_drops_smuggling_vectors() {
        let mut headers = HeaderMap::new();
        headers.insert("upgrade", "websocket".parse().unwrap());
        headers.insert("connection", "keep-alive, Upgrade".parse().unwrap());
        headers.insert("transfer-encoding", "chunked".parse().unwrap());
        headers.insert(
            "sec-websocket-protocol",
            "v5.channel.k8s.io".parse().unwrap(),
        );
        let req = build_upgrade_request(
            &headers,
            &http::Method::GET,
            &url("https://api.example:6443/api/v1/namespaces/d/pods/p/exec"),
            None,
            crate::cluster::redirect::upstream_identity::Upstream::Passthrough(None),
        )
        .unwrap();
        assert_eq!(req.headers()["connection"], "Upgrade");
        assert_eq!(req.headers()["upgrade"], "websocket");
        assert_eq!(req.headers()["sec-websocket-protocol"], "v5.channel.k8s.io");
        assert!(!req.headers().contains_key("transfer-encoding"));
    }

    /// Decision 39: a bearer token in the sub-protocol never goes upstream,
    /// the channel protocols do, in their order.
    #[test]
    fn bearer_subprotocol_entries_are_stripped_in_every_mode() {
        let token = "base64url.bearer.authorization.k8s.io.ZXlKaGJHY2lPaUpTVXpJMU5pSjk";
        let mut headers = HeaderMap::new();
        headers.insert("upgrade", "websocket".parse().unwrap());
        headers.append(
            "sec-websocket-protocol",
            format!("v5.channel.k8s.io, {token}").parse().unwrap(),
        );
        headers.append(
            "sec-websocket-protocol",
            "v4.channel.k8s.io".parse().unwrap(),
        );
        let req = build_upgrade_request(
            &headers,
            &http::Method::GET,
            &url("https://api.example:6443/api/v1/namespaces/d/pods/p/exec"),
            None,
            crate::cluster::redirect::upstream_identity::Upstream::Passthrough(None),
        )
        .unwrap();
        let sent: Vec<_> = req
            .headers()
            .get_all("sec-websocket-protocol")
            .iter()
            .map(|value| value.to_str().unwrap().to_string())
            .collect();
        assert_eq!(sent, ["v5.channel.k8s.io", "v4.channel.k8s.io"]);

        // A header holding only the token is dropped altogether.
        let mut headers = HeaderMap::new();
        headers.insert("sec-websocket-protocol", token.parse().unwrap());
        let req = build_upgrade_request(
            &headers,
            &http::Method::GET,
            &url("https://api.example:6443/api/v1/namespaces/d/pods/p/exec"),
            None,
            crate::cluster::redirect::upstream_identity::Upstream::Passthrough(None),
        )
        .unwrap();
        assert!(!req.headers().contains_key("sec-websocket-protocol"));
    }

    /// T5 and decision 23 on the upgrade path: in FrontProxy the caller's
    /// token never goes, each group is its own header, nothing
    /// `x-forwarded-*` is set.
    #[test]
    fn front_proxy_headers_replace_the_token_on_the_upgrade_path() {
        use crate::cluster::redirect::upstream_identity::{Upstream, UpstreamIdentity};
        let identity = UpstreamIdentity {
            username: "proxyauth:alice".into(),
            groups: vec!["proxyauth:a,b".into(), "proxyauth:c".into()],
            extra: std::collections::BTreeMap::new(),
        };
        let mut headers = HeaderMap::new();
        headers.insert("upgrade", "websocket".parse().unwrap());
        headers.append("authorization", "Bearer idp-token".parse().unwrap());
        headers.append("Authorization", "Bearer second".parse().unwrap());
        let req = build_upgrade_request(
            &headers,
            &http::Method::GET,
            &url("https://api.example:6443/api/v1/namespaces/d/pods/p/exec"),
            None,
            Upstream::Headers(&identity),
        )
        .unwrap();
        assert!(!req.headers().contains_key("authorization"));
        assert!(!req.headers().contains_key("x-forwarded-user"));
        assert_eq!(req.headers()["x-remote-user"], "proxyauth:alice");
        let groups: Vec<_> = req.headers().get_all("x-remote-group").iter().collect();
        assert_eq!(groups, ["proxyauth:a,b", "proxyauth:c"]);
    }
}
