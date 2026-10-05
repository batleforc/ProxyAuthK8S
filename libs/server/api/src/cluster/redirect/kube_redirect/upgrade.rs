use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, dev::PeerAddr, http, web};
use common::State;
use crd::ProxyKubeApi;
use crd::security::PortPolicy;
use crd::security::path_matcher::percent_decode_once;
use futures_util::stream::StreamExt;
use rustls::pki_types::ServerName;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
};
use tokio_rustls::TlsConnector;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, instrument, warn};

use super::port_forward::{self, PortForwardFilter};
use super::tls::build_tls_config;
use crate::cluster::redirect::audit::AuditContext;
use crate::cluster::redirect::forwarded::{
    forwarded_for_value, identity_headers, is_proxy_owned_header, is_upstream_auth_header,
};
use crate::cluster::redirect::status_response::{bad_request, forbidden};
use crate::model::user::User;

/// In-flight chunks buffered between the upgraded upstream connection and the
/// client. Bounded to keep exec/attach/port-forward sessions from growing
/// without limit when one side reads slower than the other.
const UPGRADE_CHANNEL_CAPACITY: usize = 32;

/// Upper bound on the upstream response header block. A misbehaving or
/// compromised upstream that never terminates its headers must not be able to
/// grow per-connection memory without limit.
const MAX_UPGRADE_HEADER_BYTES: usize = 64 * 1024;

/// Upper bound on a non-upgraded upstream response read on the upgrade path.
/// Such a response is a refused handshake (an apiserver `Status`, a 4xx from a
/// proxied pod), buffered so the upstream connection can be dropped right after.
const MAX_REFUSED_RESPONSE_BYTES: usize = 1024 * 1024;

/// Subresources the apiserver serves over an upgraded connection.
const STREAMING_SUBRESOURCES: [&str; 4] = ["exec", "attach", "portforward", "proxy"];

/// Whether the request is an upgrade handshake that must be tunnelled.
///
/// Both `Connection: upgrade` and an `Upgrade` protocol are required (RFC 9110
/// §7.8), on a method and path the apiserver actually upgrades: the streaming
/// subresources and watches. Anything else goes through the standard path, where
/// `reqwest` owns the request framing, so an `Upgrade` header on an arbitrary
/// request cannot open a raw socket to the cluster.
pub(super) fn is_upgrade_request(req: &HttpRequest, upstream_path: &str) -> bool {
    let has_upgrade_header = req
        .headers()
        .get(http::header::UPGRADE)
        .is_some_and(|value| !value.is_empty());
    let connection_has_upgrade_token = req
        .headers()
        .get_all(http::header::CONNECTION)
        .filter_map(|v| v.to_str().ok())
        .any(|v| {
            v.split(',').any(|token| {
                token
                    .trim()
                    .eq_ignore_ascii_case(http::header::UPGRADE.as_str())
            })
        });

    has_upgrade_header
        && connection_has_upgrade_token
        && matches!(*req.method(), http::Method::GET | http::Method::POST)
        && is_upgrade_target(upstream_path, req.query_string())
}

/// Whether the apiserver can upgrade a request to `upstream_path`.
fn is_upgrade_target(upstream_path: &str, query: &str) -> bool {
    // Decoded as the apiserver decodes them, so `exe%63` is still `exec`.
    let is_streaming_path = upstream_path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(percent_decode_once)
        .any(|segment| segment == "watch" || STREAMING_SUBRESOURCES.contains(&segment.as_str()));
    let is_watch_query = query
        .split('&')
        .any(|param| param == "watch=true" || param == "watch=1");

    is_streaming_path || is_watch_query
}

/// Whether the client declares a request body on the upgrade path.
///
/// Upgrade handshakes (websocket / SPDY exec, attach, port-forward, watch) never
/// carry a request body. The upgrade path hand-serializes the request onto a raw
/// upstream socket, so a client-supplied `Content-Length`/`Transfer-Encoding`
/// plus a body would let an attacker control request framing and smuggle a
/// second request past the proxy's authorization and identity stamping. Refusing
/// any declared body closes that vector.
fn upgrade_request_declares_body(req: &HttpRequest) -> bool {
    if req.headers().contains_key(http::header::TRANSFER_ENCODING) {
        return true;
    }
    match req
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
        }) {
        // No Content-Length header at all.
        None => false,
        // Present and parses to zero.
        Some(Some(0)) => false,
        // Present with a non-zero or unparseable value: treat as a body.
        Some(_) => true,
    }
}

trait AsyncIo: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> AsyncIo for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

type BoxedAsyncIo = Box<dyn AsyncIo>;

async fn connect_upgrade_stream(
    proxy: &ProxyKubeApi,
    state: &web::Data<State>,
    upstream_url: &reqwest::Url,
) -> Result<BoxedAsyncIo, String> {
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

    let tcp_stream = TcpStream::connect((host, port))
        .await
        .map_err(|e| e.to_string())?;

    if upstream_url.scheme().eq_ignore_ascii_case("http") {
        return Ok(Box::new(tcp_stream) as BoxedAsyncIo);
    }

    let tls_config = build_tls_config(proxy, state).await?;
    let server_name = ServerName::try_from(host.to_string()).map_err(|e| e.to_string())?;
    let connector = TlsConnector::from(Arc::new(tls_config));
    let tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .map_err(|e| e.to_string())?;

    Ok(Box::new(tls_stream) as BoxedAsyncIo)
}

fn serialize_upgrade_request(
    req: &HttpRequest,
    method: &http::Method,
    upstream_url: &reqwest::Url,
    peer_addr: Option<PeerAddr>,
    user: Option<&User>,
) -> Vec<u8> {
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

    // The connection-management headers are rebuilt rather than copied, so the
    // client cannot steer how the upstream treats the connection.
    request_bytes.extend_from_slice(b"Connection: Upgrade\r\n");
    if let Some(upgrade) = req.headers().get(http::header::UPGRADE) {
        request_bytes.extend_from_slice(b"Upgrade: ");
        request_bytes.extend_from_slice(upgrade.as_bytes());
        request_bytes.extend_from_slice(b"\r\n");
    }

    for (header_name, header_value) in req.headers() {
        // Drop the rewritten host, the connection-management and framing headers
        // (rebuilt above, or meaningless without a body) and the identity headers
        // the proxy owns.
        if header_name == http::header::HOST
            || header_name == http::header::CONNECTION
            || header_name == http::header::UPGRADE
            || header_name.as_str().eq_ignore_ascii_case("keep-alive")
            || header_name
                .as_str()
                .eq_ignore_ascii_case("proxy-connection")
            || header_name == http::header::CONTENT_LENGTH
            || header_name == http::header::TRANSFER_ENCODING
            || is_proxy_owned_header(header_name.as_str())
            || is_upstream_auth_header(header_name.as_str())
        {
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

async fn read_upgrade_response_headers(
    upstream: &mut (impl AsyncRead + Unpin),
) -> Result<(http::StatusCode, Vec<(String, Vec<u8>)>, Vec<u8>), String> {
    let mut buffer = Vec::with_capacity(4096);
    let mut temp = [0u8; 2048];

    loop {
        let read = upstream.read(&mut temp).await.map_err(|e| e.to_string())?;
        if read == 0 {
            return Err("upstream closed before sending response headers".to_string());
        }
        buffer.extend_from_slice(&temp[..read]);

        if buffer.len() > MAX_UPGRADE_HEADER_BYTES {
            return Err("upstream response headers exceeded the allowed size".to_string());
        }

        if let Some(header_end) = buffer.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let body_start = header_end + 4;
            let header_bytes = &buffer[..header_end];
            let leftover = buffer[body_start..].to_vec();
            let header_text = String::from_utf8_lossy(header_bytes);
            let mut lines = header_text.split("\r\n");
            let status_line = lines
                .next()
                .ok_or_else(|| "missing upstream status line".to_string())?;
            let status_code = status_line
                .split_whitespace()
                .nth(1)
                .ok_or_else(|| "invalid upstream status line".to_string())?
                .parse::<u16>()
                .map_err(|e| e.to_string())?;

            let mut headers = Vec::new();
            for line in lines {
                if let Some((name, value)) = line.split_once(':') {
                    headers.push((name.trim().to_string(), value.trim().as_bytes().to_vec()));
                }
            }

            return Ok((
                http::StatusCode::from_u16(status_code).map_err(|e| e.to_string())?,
                headers,
                leftover,
            ));
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[instrument(skip(req, data, payload, user, audit))]
pub(super) async fn upgrade_redirect(
    req: HttpRequest,
    data: web::Data<State>,
    payload: web::Payload,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
    proxy: ProxyKubeApi,
    url_to_call: String,
    user: Option<User>,
    audit: AuditContext,
    port_policy: PortPolicy,
) -> HttpResponse {
    // Only websocket is tunnelled: actix-http hands the bytes that follow the
    // handshake to the handler for `Upgrade: websocket` alone, so a SPDY
    // session would die right after its `101`. Refusing it up front gives the
    // client a readable error instead.
    let is_websocket = req
        .headers()
        .get(http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("websocket"));
    if !is_websocket {
        audit.emit(400);
        return bad_request(
            "this proxy only tunnels websocket upgrades; SPDY is not supported, use a kubectl recent enough to use websockets for exec, attach and port-forward",
        );
    }

    // The query fixes the ports of the channel protocols, and a SPDY tunnel
    // must not ask for others either.
    let is_restricted_port_forward =
        port_policy.is_restricted() && port_forward::is_port_forward_path(req.path());
    if is_restricted_port_forward
        && let Err(reason) = port_forward::check_query_ports(req.query_string(), &port_policy)
    {
        warn!(%reason, "refusing a port-forward to a port outside the allowed list");
        audit.emit(403);
        return forbidden(&reason);
    }

    // Upgrade handshakes never carry a body; a declared body here is an attempt
    // to smuggle a second request onto the raw upstream socket.
    if upgrade_request_declares_body(&req) {
        audit.emit(400);
        return HttpResponse::BadRequest().body("upgrade requests must not carry a body");
    }

    let upstream_url = match reqwest::Url::parse(&url_to_call) {
        Ok(url) => url,
        Err(err) => {
            error!(error = %err, "invalid upstream url for upgrade request");
            audit.emit(502);
            return HttpResponse::BadGateway().body("bad gateway");
        }
    };

    let mut upstream = match connect_upgrade_stream(&proxy, &data, &upstream_url).await {
        Ok(stream) => stream,
        Err(err) => {
            error!(error = %err, "could not open the upstream upgrade stream");
            audit.emit(503);
            return HttpResponse::ServiceUnavailable().body("upstream unavailable");
        }
    };

    let request_bytes =
        serialize_upgrade_request(&req, &method, &upstream_url, peer_addr, user.as_ref());
    if let Err(err) = upstream.write_all(&request_bytes).await {
        error!(error = %err, "could not write the upgrade request upstream");
        audit.emit(503);
        return HttpResponse::ServiceUnavailable().body("upstream unavailable");
    }
    if let Err(err) = upstream.flush().await {
        error!(error = %err, "could not flush the upgrade request upstream");
        audit.emit(503);
        return HttpResponse::ServiceUnavailable().body("upstream unavailable");
    }

    let (status, headers, leftover) = match read_upgrade_response_headers(&mut upstream).await {
        Ok(response) => response,
        Err(err) => {
            error!(error = %err, "could not read the upstream upgrade response headers");
            audit.emit(502);
            return HttpResponse::BadGateway().body("bad gateway");
        }
    };

    let upgrade_protocol = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("upgrade"))
        .and_then(|(_, value)| std::str::from_utf8(value).ok())
        .map(str::to_string);

    // Only a completed `101 Switching Protocols` turns the connection into a
    // tunnel. On any other answer the upstream keeps parsing HTTP on that
    // socket, so piping the client's remaining bytes would let them through as
    // a second request that skipped authorization, the allow-list and identity
    // stamping. Answer with the refusal and drop both connections instead.
    let Some(upgrade_protocol) =
        upgrade_protocol.filter(|_| status == http::StatusCode::SWITCHING_PROTOCOLS)
    else {
        tracing::Span::current().record("http.response.status_code", status.as_u16());
        audit.emit(status.as_u16());
        return refused_upgrade_response(&mut upstream, status, headers, leftover).await;
    };

    // A restricted port-forward is only tunnelled over a protocol whose ports
    // are known: fixed by the (checked) query, or read by the filter.
    let mut filter: Option<PortForwardFilter> = None;
    if is_restricted_port_forward {
        let header = |wanted: &str| {
            headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
                .and_then(|(_, value)| std::str::from_utf8(value).ok())
        };
        match port_forward::filter_for_accepted_protocol(
            header("sec-websocket-protocol"),
            header("sec-websocket-extensions"),
            &port_policy,
        ) {
            Ok(selected) => filter = selected,
            Err(reason) => {
                warn!(%reason, "refusing a port-forward the proxy cannot filter");
                audit.emit(403);
                return forbidden(&reason);
            }
        }
    }

    tracing::Span::current().record("http.response.status_code", status.as_u16());
    audit.emit(status.as_u16());

    let (mut upstream_reader, mut upstream_writer) = tokio::io::split(upstream);
    // Bounded so a slow client back-pressures the upstream reader instead of
    // letting the upgraded stream accumulate in memory.
    let (tx, rx) = mpsc::channel::<web::Bytes>(UPGRADE_CHANNEL_CAPACITY);

    if !leftover.is_empty() && tx.send(web::Bytes::from(leftover)).await.is_err() {
        return HttpResponse::ServiceUnavailable().body("client stream closed");
    }

    // Fired when the filter closes the session, to stop the upstream reader
    // too: both halves dropped close the upstream connection, and the client
    // stream ends with the reader.
    let (close_tx, mut close_rx) = tokio::sync::oneshot::channel::<()>();

    let mut client_payload = payload.into_inner();
    actix_web::rt::spawn(async move {
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
    });

    let tx_reader = tx.clone();
    actix_web::rt::spawn(async move {
        let mut buffer = [0u8; 8192];
        loop {
            let read = tokio::select! {
                read = upstream_reader.read(&mut buffer) => read,
                _ = &mut close_rx => break,
            };
            match read {
                Ok(0) => break,
                Ok(read) => {
                    if tx_reader
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
    });

    let mut client_resp = HttpResponse::build(status);
    client_resp.upgrade(upgrade_protocol);
    copy_upstream_headers(&mut client_resp, headers);

    client_resp.streaming(ReceiverStream::new(rx).map(Ok::<web::Bytes, actix_web::Error>))
}

/// Relay an upstream response that did not upgrade the connection, then close.
///
/// The body is read according to its own framing so the response is complete,
/// and the client connection is closed with it: whatever the client sent after
/// its handshake is discarded rather than forwarded.
async fn refused_upgrade_response(
    upstream: &mut (impl AsyncRead + Unpin),
    status: http::StatusCode,
    headers: Vec<(String, Vec<u8>)>,
    leftover: Vec<u8>,
) -> HttpResponse {
    let body = match read_response_body(upstream, status, &headers, leftover).await {
        Ok(body) => body,
        Err(err) => {
            error!(error = %err, %status, "could not read the refused upgrade response");
            return HttpResponse::BadGateway().force_close().body("bad gateway");
        }
    };

    let mut client_resp = HttpResponse::build(status);
    client_resp.force_close();
    copy_upstream_headers(&mut client_resp, headers);
    client_resp.body(body)
}

fn copy_upstream_headers(
    client_resp: &mut actix_web::HttpResponseBuilder,
    headers: Vec<(String, Vec<u8>)>,
) {
    for (header_name, header_value) in headers {
        if header_name.eq_ignore_ascii_case("transfer-encoding")
            || header_name.eq_ignore_ascii_case("content-length")
            || header_name.eq_ignore_ascii_case("host")
            || header_name.eq_ignore_ascii_case("connection")
            || header_name.eq_ignore_ascii_case("keep-alive")
            || header_name.eq_ignore_ascii_case("upgrade")
        {
            continue;
        }

        if let (Ok(name), Ok(value)) = (
            actix_web::http::header::HeaderName::from_bytes(header_name.as_bytes()),
            actix_web::http::header::HeaderValue::from_bytes(&header_value),
        ) {
            client_resp.insert_header((name, value));
        }
    }
}

/// Read one HTTP/1.1 response body following its framing (RFC 9112 §6.3):
/// none for 1xx/204/304, chunked, `Content-Length`, or until the upstream
/// closes. Bounded by [`MAX_REFUSED_RESPONSE_BYTES`].
async fn read_response_body(
    upstream: &mut (impl AsyncRead + Unpin),
    status: http::StatusCode,
    headers: &[(String, Vec<u8>)],
    mut buffer: Vec<u8>,
) -> Result<Vec<u8>, String> {
    if status.is_informational()
        || status == http::StatusCode::NO_CONTENT
        || status == http::StatusCode::NOT_MODIFIED
    {
        return Ok(Vec::new());
    }

    let header = |wanted: &str| {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| String::from_utf8_lossy(value).to_ascii_lowercase())
    };

    if header("transfer-encoding").is_some_and(|te| te.contains("chunked")) {
        return read_chunked_body(upstream, buffer).await;
    }

    if let Some(length) = header("content-length") {
        let length = length
            .trim()
            .parse::<usize>()
            .map_err(|e| format!("invalid upstream content-length: {e}"))?;
        if length > MAX_REFUSED_RESPONSE_BYTES {
            return Err("upstream response body exceeded the allowed size".to_string());
        }
        while buffer.len() < length {
            read_more(upstream, &mut buffer).await?;
        }
        buffer.truncate(length);
        return Ok(buffer);
    }

    loop {
        match read_more(upstream, &mut buffer).await {
            Ok(()) => {}
            Err(ReadMoreError::Closed) => return Ok(buffer),
            Err(err) => return Err(err.to_string()),
        }
    }
}

async fn read_chunked_body(
    upstream: &mut (impl AsyncRead + Unpin),
    mut buffer: Vec<u8>,
) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    let mut pos = 0;

    loop {
        let line_end = loop {
            if let Some(offset) = buffer[pos..].windows(2).position(|w| w == b"\r\n") {
                break pos + offset;
            }
            read_more(upstream, &mut buffer).await?;
        };
        let size_line = std::str::from_utf8(&buffer[pos..line_end])
            .map_err(|_| "invalid upstream chunk size".to_string())?;
        let size_hex = size_line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| "invalid upstream chunk size".to_string())?;
        pos = line_end + 2;

        // The last chunk; trailers are not needed since the connection is dropped.
        if size == 0 {
            return Ok(body);
        }

        let chunk_end = pos
            .checked_add(size)
            .filter(|end| *end <= MAX_REFUSED_RESPONSE_BYTES)
            .ok_or_else(|| "upstream response body exceeded the allowed size".to_string())?;
        while buffer.len() < chunk_end + 2 {
            read_more(upstream, &mut buffer).await?;
        }
        body.extend_from_slice(&buffer[pos..chunk_end]);
        pos = chunk_end + 2;
    }
}

#[derive(Debug)]
enum ReadMoreError {
    Closed,
    TooLarge,
    Io(std::io::Error),
}

impl std::fmt::Display for ReadMoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => f.write_str("upstream closed before the response was complete"),
            Self::TooLarge => f.write_str("upstream response body exceeded the allowed size"),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

impl From<ReadMoreError> for String {
    fn from(err: ReadMoreError) -> Self {
        err.to_string()
    }
}

async fn read_more(
    upstream: &mut (impl AsyncRead + Unpin),
    buffer: &mut Vec<u8>,
) -> Result<(), ReadMoreError> {
    let mut temp = [0u8; 8192];
    let read = upstream.read(&mut temp).await.map_err(ReadMoreError::Io)?;
    if read == 0 {
        return Err(ReadMoreError::Closed);
    }
    buffer.extend_from_slice(&temp[..read]);
    if buffer.len() > MAX_REFUSED_RESPONSE_BYTES {
        return Err(ReadMoreError::TooLarge);
    }
    Ok(())
}
