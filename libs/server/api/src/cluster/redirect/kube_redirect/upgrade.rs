use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, dev::PeerAddr, http, web};
use common::State;
use crd::ProxyKubeApi;
use futures_util::stream::StreamExt;
use rustls::pki_types::ServerName;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
};
use tokio_rustls::TlsConnector;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, instrument};

use super::tls::build_tls_config;
use crate::cluster::redirect::audit::AuditContext;
use crate::cluster::redirect::forwarded::{
    forwarded_for_value, identity_headers, is_proxy_owned_header, is_upstream_auth_header,
};
use crate::model::user::User;

/// In-flight chunks buffered between the upgraded upstream connection and the
/// client. Bounded to keep exec/attach/port-forward sessions from growing
/// without limit when one side reads slower than the other.
const UPGRADE_CHANNEL_CAPACITY: usize = 32;

/// Upper bound on the upstream response header block. A misbehaving or
/// compromised upstream that never terminates its headers must not be able to
/// grow per-connection memory without limit.
const MAX_UPGRADE_HEADER_BYTES: usize = 64 * 1024;

pub(super) fn is_upgrade_request(req: &HttpRequest) -> bool {
    let has_upgrade_header = req.headers().contains_key(http::header::UPGRADE);
    let connection_has_upgrade_token = req
        .headers()
        .get(http::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',').any(|token| {
                token
                    .trim()
                    .eq_ignore_ascii_case(http::header::UPGRADE.as_str())
            })
        });

    has_upgrade_header || connection_has_upgrade_token
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

    for (header_name, header_value) in req.headers() {
        // `connection`/`upgrade` are exactly what makes this an upgrade, so they
        // are forwarded; only the rewritten host and the proxy-owned headers go.
        // Drop the rewritten host, proxy-owned identity headers, and the framing
        // headers (`Content-Length`/`Transfer-Encoding`). `connection`/`upgrade`
        // are intentionally kept — they are what makes this an upgrade.
        if header_name == http::header::HOST
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
) -> HttpResponse {
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

    tracing::Span::current().record("http.response.status_code", status.as_u16());
    audit.emit(status.as_u16());

    let (mut upstream_reader, mut upstream_writer) = tokio::io::split(upstream);
    // Bounded so a slow client back-pressures the upstream reader instead of
    // letting the upgraded stream accumulate in memory.
    let (tx, rx) = mpsc::channel::<web::Bytes>(UPGRADE_CHANNEL_CAPACITY);

    if !leftover.is_empty() && tx.send(web::Bytes::from(leftover)).await.is_err() {
        return HttpResponse::ServiceUnavailable().body("client stream closed");
    }

    let mut client_payload = payload.into_inner();
    actix_web::rt::spawn(async move {
        while let Some(item) = client_payload.next().await {
            match item {
                Ok(chunk) => {
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
            match upstream_reader.read(&mut buffer).await {
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
    if status == http::StatusCode::SWITCHING_PROTOCOLS
        && let Some((_, upgrade_value)) = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("upgrade"))
        && let Ok(upgrade_value) = std::str::from_utf8(upgrade_value)
    {
        client_resp.upgrade(upgrade_value);
    }

    for (header_name, header_value) in headers {
        if header_name.eq_ignore_ascii_case("transfer-encoding")
            || header_name.eq_ignore_ascii_case("content-length")
            || header_name.eq_ignore_ascii_case("host")
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

    client_resp.streaming(ReceiverStream::new(rx).map(Ok::<web::Bytes, actix_web::Error>))
}
