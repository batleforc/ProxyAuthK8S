//! Opening the raw upstream connection of an upgrade and writing its handshake.

use std::time::Duration;

use actix_web::{HttpRequest, dev::PeerAddr, http, web};
use common::State;
use crd::ProxyKubeApi;
use rustls::pki_types::ServerName;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};
use tokio_rustls::TlsConnector;

use super::super::tls::cached_tls_config;
use crate::cluster::redirect::forwarded::{
    forwarded_for_value, identity_headers, is_proxy_owned_header, is_upstream_auth_header,
};
use crate::model::user::User;

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
    state: &web::Data<State>,
    upstream_url: &reqwest::Url,
) -> Result<BoxedAsyncIo, String> {
    let (host, port) = upstream_host_port(upstream_url)?;

    let tcp_stream =
        tokio::time::timeout(UPGRADE_CONNECT_TIMEOUT, TcpStream::connect((host, port)))
            .await
            .map_err(|_| "upstream connect timed out".to_string())?
            .map_err(|e| e.to_string())?;

    if upstream_url.scheme().eq_ignore_ascii_case("http") {
        return Ok(Box::new(tcp_stream) as BoxedAsyncIo);
    }

    let tls_config = cached_tls_config(proxy, state).await?;
    let server_name = ServerName::try_from(host.to_string()).map_err(|e| e.to_string())?;
    let connector = TlsConnector::from(tls_config);
    let tls_stream = tokio::time::timeout(
        UPGRADE_CONNECT_TIMEOUT,
        connector.connect(server_name, tcp_stream),
    )
    .await
    .map_err(|_| "upstream TLS handshake timed out".to_string())?
    .map_err(|e| e.to_string())?;

    Ok(Box::new(tls_stream) as BoxedAsyncIo)
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

/// `METHOD path?query HTTP/1.1` plus the `Host` header for `upstream_url`.
fn request_line_and_host(method: &http::Method, upstream_url: &reqwest::Url) -> Vec<u8> {
    let path = match upstream_url.query() {
        Some(query) => format!("{}?{}", upstream_url.path(), query),
        None => upstream_url.path().to_string(),
    };
    let authority = upstream_url.port().map_or_else(
        || upstream_url.host_str().unwrap_or_default().to_string(),
        |port| format!("{}:{}", upstream_url.host_str().unwrap_or_default(), port),
    );

    let mut request_bytes = format!("{} {} HTTP/1.1\r\n", method.as_str(), path).into_bytes();
    request_bytes.extend_from_slice(format!("Host: {authority}\r\n").as_bytes());
    request_bytes
}

/// Hand-serialize the upgrade handshake sent on the raw upstream socket.
pub(super) fn serialize_upgrade_request(
    req: &HttpRequest,
    method: &http::Method,
    upstream_url: &reqwest::Url,
    peer_addr: Option<PeerAddr>,
    user: Option<&User>,
) -> Vec<u8> {
    let mut request_bytes = request_line_and_host(method, upstream_url);

    // The connection-management headers are rebuilt rather than copied, so the
    // client cannot steer how the upstream treats the connection.
    request_bytes.extend_from_slice(b"Connection: Upgrade\r\n");
    if let Some(upgrade) = req.headers().get(http::header::UPGRADE) {
        request_bytes.extend_from_slice(b"Upgrade: ");
        request_bytes.extend_from_slice(upgrade.as_bytes());
        request_bytes.extend_from_slice(b"\r\n");
    }

    for (header_name, header_value) in req.headers() {
        if is_dropped_request_header(header_name) {
            continue;
        }
        request_bytes.extend_from_slice(header_name.as_str().as_bytes());
        request_bytes.extend_from_slice(b": ");
        request_bytes.extend_from_slice(header_value.as_bytes());
        request_bytes.extend_from_slice(b"\r\n");
    }

    let incoming_forwarded_for = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok());
    let peer_ip = peer_addr.map(|PeerAddr(addr)| addr.ip());
    if let Some(forwarded_for) = forwarded_for_value(incoming_forwarded_for, peer_ip) {
        request_bytes.extend_from_slice(format!("x-forwarded-for: {forwarded_for}\r\n").as_bytes());
    }

    for (name, value) in identity_headers(user) {
        request_bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }

    request_bytes.extend_from_slice(b"\r\n");
    request_bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test::TestRequest;

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
        let bytes = request_line_and_host(
            &http::Method::GET,
            &url("https://api.example:6443/api/v1/pods/p/exec?command=ls"),
        );
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "GET /api/v1/pods/p/exec?command=ls HTTP/1.1\r\nHost: api.example:6443\r\n"
        );
        let bytes = request_line_and_host(&http::Method::POST, &url("https://api.example/x"));
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "POST /x HTTP/1.1\r\nHost: api.example\r\n"
        );
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
        let req = TestRequest::get()
            .insert_header(("upgrade", "websocket"))
            .insert_header(("connection", "keep-alive, Upgrade"))
            .insert_header(("transfer-encoding", "chunked"))
            .insert_header(("sec-websocket-protocol", "v5.channel.k8s.io"))
            .to_http_request();
        let bytes = serialize_upgrade_request(
            &req,
            &http::Method::GET,
            &url("https://api.example:6443/api/v1/namespaces/d/pods/p/exec"),
            None,
            None,
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with(
            "GET /api/v1/namespaces/d/pods/p/exec HTTP/1.1\r\nHost: api.example:6443\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n"
        ));
        assert!(text.contains("sec-websocket-protocol: v5.channel.k8s.io\r\n"));
        assert!(!text.contains("transfer-encoding"));
        assert!(!text.contains("keep-alive"));
        assert!(text.ends_with("\r\n\r\n"));
    }
}
