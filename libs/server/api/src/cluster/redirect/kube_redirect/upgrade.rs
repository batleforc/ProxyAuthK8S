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

use super::context::RedirectContext;
use super::tls::build_tls_config;
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
    let port = upstream_url
        .port_or_known_default()
        .ok_or_else(|| "missing upstream port".to_string())?;

    let tcp_stream = TcpStream::connect((host, port))
        .await
        .map_err(|e| e.to_string())?;

    if upstream_url.scheme().eq_ignore_ascii_case("http") {
        return Ok(Box::new(tcp_stream) as BoxedAsyncIo);
    }

    let tls_config = build_tls_config(proxy, state, true).await?;
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

// See `standard_redirect`: `proxy` is `Empty` so the whole `ProxyKubeApi` is not
// formatted into the span on every proxied request, and is recorded only under
// DEBUG.
#[instrument(
    skip_all,
    fields(method = ?ctx.method, peer_addr = ?ctx.peer_addr, proxy = tracing::field::Empty, url_to_call)
)]
pub(super) async fn upgrade_redirect(ctx: RedirectContext) -> HttpResponse {
    let url_to_call = ctx.url_to_call();
    tracing::Span::current().record("url_to_call", url_to_call.as_str());
    if tracing::enabled!(tracing::Level::DEBUG) {
        tracing::Span::current().record("proxy", tracing::field::debug(&ctx.proxy));
    }
    let RedirectContext {
        req,
        data,
        payload,
        method,
        peer_addr,
        proxy,
        user,
        audit,
        ..
    } = ctx;
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

#[cfg(test)]
mod tests {
    //! Characterization tests for the upgrade path's security contract.
    //!
    //! Written deliberately BEFORE the planned merge of the response-header
    //! logic into a shared `copy_upstream_response_headers`. Tests added after a
    //! refactor can only pin whatever the refactor produced; these pin what the
    //! code is required to do, so they can still fail if the merge changes it.
    //!
    //! Scope is the two functions that encode a security contract rather than an
    //! implementation shape: the request-smuggling guard, and the serializer that
    //! strips client-supplied identity and stamps the proxy's own.

    use super::{is_upgrade_request, serialize_upgrade_request, upgrade_request_declares_body};
    use crate::model::user::User;
    use actix_web::dev::PeerAddr;
    use actix_web::test::TestRequest;
    use actix_web::{HttpRequest, http};

    fn request(headers: &[(&str, &str)]) -> HttpRequest {
        let mut req = TestRequest::get().uri("/api/v1/pods");
        for (name, value) in headers {
            req = req.insert_header((*name, *value));
        }
        req.to_http_request()
    }

    fn user() -> User {
        User {
            username: "alice".to_string(),
            email: "alice@example.com".to_string(),
            groups: vec!["dev".to_string(), "platform".to_string()],
        }
    }

    fn serialize(headers: &[(&str, &str)], user: Option<&User>) -> String {
        let req = request(headers);
        let url = reqwest::Url::parse("https://cluster.example.com:6443/api/v1/pods?watch=true")
            .expect("test url should parse");
        String::from_utf8(serialize_upgrade_request(
            &req,
            &http::Method::GET,
            &url,
            Some(PeerAddr(
                "10.1.2.3:5555".parse().expect("addr should parse"),
            )),
            user,
        ))
        .expect("serialized request should be utf-8")
    }

    #[test]
    fn an_upgrade_is_recognised_from_either_header() {
        assert!(is_upgrade_request(&request(&[("upgrade", "websocket")])));
        assert!(is_upgrade_request(&request(&[("connection", "upgrade")])));
        // A multi-token Connection header is the common real-world shape.
        assert!(is_upgrade_request(&request(&[(
            "connection",
            "keep-alive, Upgrade"
        )])));
        // Case-insensitive, per RFC 7230.
        assert!(is_upgrade_request(&request(&[("connection", "UPGRADE")])));
        assert!(!is_upgrade_request(&request(&[(
            "connection",
            "keep-alive"
        )])));
        assert!(!is_upgrade_request(&request(&[])));
    }

    /// The smuggling guard. An upgrade handshake never carries a body, and this
    /// path hand-serializes onto a raw socket — a client-controlled framing
    /// header plus a body would let a second request ride past authorization.
    #[test]
    fn a_declared_body_is_refused_on_the_upgrade_path() {
        assert!(upgrade_request_declares_body(&request(&[(
            "content-length",
            "5"
        )])));
        assert!(upgrade_request_declares_body(&request(&[(
            "transfer-encoding",
            "chunked"
        )])));
        // Unparseable length: fail closed rather than guess.
        assert!(upgrade_request_declares_body(&request(&[(
            "content-length",
            "not-a-number"
        )])));
        // Transfer-Encoding wins even alongside a zero length — the pair is the
        // classic desync primitive.
        assert!(upgrade_request_declares_body(&request(&[
            ("content-length", "0"),
            ("transfer-encoding", "chunked"),
        ])));
    }

    #[test]
    fn a_bodyless_handshake_is_allowed() {
        assert!(!upgrade_request_declares_body(&request(&[])));
        assert!(!upgrade_request_declares_body(&request(&[(
            "content-length",
            "0"
        )])));
    }

    /// The framing headers must never reach the raw socket, or the guard above
    /// could be bypassed by anything that sets them later.
    #[test]
    fn framing_headers_are_stripped_from_the_serialized_request() {
        let wire = serialize(
            &[
                ("content-length", "0"),
                ("transfer-encoding", "chunked"),
                ("upgrade", "websocket"),
            ],
            Some(&user()),
        );
        let lower = wire.to_ascii_lowercase();
        assert!(!lower.contains("content-length"), "{wire}");
        assert!(!lower.contains("transfer-encoding"), "{wire}");
    }

    /// `connection`/`upgrade` are what make this an upgrade — stripping them
    /// would silently turn the handshake into an ordinary request.
    #[test]
    fn the_upgrade_headers_themselves_are_preserved() {
        let wire = serialize(
            &[("upgrade", "SPDY/3.1"), ("connection", "Upgrade")],
            Some(&user()),
        );
        assert!(wire.contains("upgrade: SPDY/3.1"), "{wire}");
        assert!(wire.contains("connection: Upgrade"), "{wire}");
    }

    /// The whole point of the proxy's identity stamping: a client must not be
    /// able to present itself as someone else by setting the headers the proxy
    /// owns.
    #[test]
    fn client_supplied_identity_headers_are_replaced_not_forwarded() {
        let wire = serialize(
            &[
                ("x-forwarded-user", "root"),
                ("x-forwarded-groups", "system:masters"),
                ("upgrade", "websocket"),
            ],
            Some(&user()),
        );

        assert!(
            !wire.contains("root"),
            "a spoofed x-forwarded-user reached the upstream: {wire}"
        );
        assert!(
            !wire.contains("system:masters"),
            "spoofed groups reached the upstream: {wire}"
        );
        assert!(wire.contains("x-forwarded-user: alice"), "{wire}");
        assert!(wire.contains("x-forwarded-groups: dev,platform"), "{wire}");
    }

    /// Impersonation headers are the upstream's own auth mechanism; a client
    /// that could set them would be talking to the apiserver as anyone.
    ///
    /// `Authorization` is deliberately NOT in that set: the caller's bearer
    /// token IS the cluster credential in this design, so it is forwarded. This
    /// test records that on purpose — the standard path applies the identical
    /// policy at `upstream.rs:68`, and the two must not drift apart.
    #[test]
    fn upstream_auth_headers_from_the_client_are_dropped() {
        let wire = serialize(
            &[
                ("impersonate-user", "system:admin"),
                ("impersonate-group", "system:masters"),
                ("x-remote-user", "root"),
                ("authorization", "Bearer the-callers-own-token"),
                ("upgrade", "websocket"),
            ],
            Some(&user()),
        );
        let lower = wire.to_ascii_lowercase();
        assert!(!lower.contains("impersonate-user"), "{wire}");
        assert!(!lower.contains("impersonate-group"), "{wire}");
        assert!(!lower.contains("x-remote-user"), "{wire}");
        assert!(
            lower.contains("authorization: bearer the-callers-own-token"),
            "the caller's own token is the upstream credential and must survive: {wire}"
        );
    }

    /// The Host must describe the upstream we actually opened a socket to, not
    /// whatever the client asked for.
    #[test]
    fn the_host_is_rewritten_to_the_upstream_authority() {
        let wire = serialize(
            &[("host", "evil.example.com"), ("upgrade", "websocket")],
            Some(&user()),
        );
        assert!(wire.contains("Host: cluster.example.com:6443"), "{wire}");
        assert!(!wire.contains("evil.example.com"), "{wire}");
    }

    /// The request line has to carry the query, or a `watch=true` upgrade
    /// silently becomes a non-watch request.
    #[test]
    fn the_request_line_keeps_method_path_and_query() {
        let wire = serialize(&[("upgrade", "websocket")], Some(&user()));
        assert!(
            wire.starts_with("GET /api/v1/pods?watch=true HTTP/1.1\r\n"),
            "{wire}"
        );
    }

    /// No resolved user means no identity headers at all — an absent header is
    /// safe, an empty one asserts an identity nobody holds.
    #[test]
    fn no_user_means_no_identity_headers() {
        let wire = serialize(&[("upgrade", "websocket")], None);
        let lower = wire.to_ascii_lowercase();
        assert!(!lower.contains("x-forwarded-user"), "{wire}");
        assert!(!lower.contains("x-forwarded-groups"), "{wire}");
    }

    #[test]
    fn the_forwarded_for_chain_is_appended_not_replaced() {
        let wire = serialize(
            &[("x-forwarded-for", "203.0.113.9"), ("upgrade", "websocket")],
            Some(&user()),
        );
        assert!(
            wire.contains("x-forwarded-for: 203.0.113.9, 10.1.2.3"),
            "the original client hop must survive: {wire}"
        );
    }

    /// Headers end with a blank line; without it the upstream waits forever.
    #[test]
    fn the_header_block_is_terminated() {
        let wire = serialize(&[("upgrade", "websocket")], Some(&user()));
        assert!(wire.ends_with("\r\n\r\n"), "{wire:?}");
    }
}
