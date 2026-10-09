//! Reading the upstream answer to an upgrade handshake, and relaying it when the
//! upstream refused to switch protocols.

use actix_web::{HttpResponse, http};
use tokio::io::{AsyncRead, AsyncReadExt};
use tracing::error;

/// Upper bound on the upstream response header block. A misbehaving or
/// compromised upstream that never terminates its headers must not be able to
/// grow per-connection memory without limit.
const MAX_UPGRADE_HEADER_BYTES: usize = 64 * 1024;

/// Upper bound on a non-upgraded upstream response read on the upgrade path.
/// Such a response is a refused handshake (an apiserver `Status`, a 4xx from a
/// proxied pod), buffered so the upstream connection can be dropped right after.
const MAX_REFUSED_RESPONSE_BYTES: usize = 1024 * 1024;

/// Upstream response headers, in order, as raw `(name, value)` pairs.
type ResponseHeaders = Vec<(String, Vec<u8>)>;

/// The status line and headers of the upstream answer, plus whatever bytes
/// already followed them on the socket.
pub(super) struct UpstreamHead {
    pub(super) status: http::StatusCode,
    pub(super) headers: ResponseHeaders,
    pub(super) leftover: Vec<u8>,
}

impl UpstreamHead {
    /// The first header named `wanted` (case-insensitive), when it is UTF-8.
    pub(super) fn header(&self, wanted: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .and_then(|(_, value)| std::str::from_utf8(value).ok())
    }

    /// The protocol the connection switched to, only for a completed
    /// `101 Switching Protocols` that names one.
    pub(super) fn switched_protocol(&self) -> Option<String> {
        self.header("upgrade")
            .map(str::to_string)
            .filter(|_| self.status == http::StatusCode::SWITCHING_PROTOCOLS)
    }
}

/// Parse a response header block (without its terminating blank line).
fn parse_response_head(header_bytes: &[u8]) -> Result<(http::StatusCode, ResponseHeaders), String> {
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

    let status = http::StatusCode::from_u16(status_code).map_err(|e| e.to_string())?;
    Ok((status, headers))
}

/// Read the upstream response head, bounded by [`MAX_UPGRADE_HEADER_BYTES`].
pub(super) async fn read_upgrade_response_headers(
    upstream: &mut (impl AsyncRead + Unpin),
) -> Result<UpstreamHead, String> {
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
            let leftover = buffer[header_end + 4..].to_vec();
            let (status, headers) = parse_response_head(&buffer[..header_end])?;
            return Ok(UpstreamHead {
                status,
                headers,
                leftover,
            });
        }
    }
}

/// Relay an upstream response that did not upgrade the connection, then close.
///
/// The body is read according to its own framing so the response is complete,
/// and the client connection is closed with it: whatever the client sent after
/// its handshake is discarded rather than forwarded.
pub(super) async fn refused_upgrade_response(
    upstream: &mut (impl AsyncRead + Unpin),
    head: UpstreamHead,
) -> HttpResponse {
    let UpstreamHead {
        status,
        headers,
        leftover,
    } = head;
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

/// Whether an upstream response header is not relayed to the client: framing
/// and connection management belong to each hop.
fn is_dropped_response_header(header_name: &str) -> bool {
    [
        "transfer-encoding",
        "content-length",
        "host",
        "connection",
        "keep-alive",
        "upgrade",
    ]
    .iter()
    .any(|dropped| header_name.eq_ignore_ascii_case(dropped))
}

/// Copy the relayable upstream headers onto the client response; headers that
/// are not valid HTTP are skipped.
pub(super) fn copy_upstream_headers(
    client_resp: &mut actix_web::HttpResponseBuilder,
    headers: ResponseHeaders,
) {
    for (header_name, header_value) in headers {
        if is_dropped_response_header(&header_name) {
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

#[derive(Debug, thiserror::Error)]
enum ReadMoreError {
    #[error("upstream closed before the response was complete")]
    Closed,
    #[error("upstream response body exceeded the allowed size")]
    TooLarge,
    #[error(transparent)]
    Io(std::io::Error),
}

// The refused-response readers report errors as `String`; keep `?` working.
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

#[cfg(test)]
mod read_more_error_tests {
    use super::ReadMoreError;

    #[test]
    fn read_more_errors_keep_their_messages_when_turned_into_strings() {
        assert_eq!(
            String::from(ReadMoreError::Closed),
            "upstream closed before the response was complete"
        );
        assert_eq!(
            String::from(ReadMoreError::TooLarge),
            "upstream response body exceeded the allowed size"
        );
        let io = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe broke");
        assert_eq!(String::from(ReadMoreError::Io(io)), "pipe broke");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(status: u16, headers: &[(&str, &str)]) -> UpstreamHead {
        UpstreamHead {
            status: http::StatusCode::from_u16(status).unwrap(),
            headers: headers
                .iter()
                .map(|(n, v)| (n.to_string(), v.as_bytes().to_vec()))
                .collect(),
            leftover: Vec::new(),
        }
    }

    #[test]
    fn the_status_and_headers_are_parsed() {
        let (status, headers) = parse_response_head(
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nX:  y ",
        )
        .unwrap();
        assert_eq!(status, http::StatusCode::SWITCHING_PROTOCOLS);
        assert_eq!(
            headers,
            vec![
                ("Upgrade".to_string(), b"websocket".to_vec()),
                ("X".to_string(), b"y".to_vec()),
            ]
        );
    }

    #[test]
    fn a_malformed_status_line_is_refused() {
        assert_eq!(
            parse_response_head(b"garbage").unwrap_err(),
            "invalid upstream status line"
        );
        assert!(parse_response_head(b"HTTP/1.1 abc OK").is_err());
        assert!(parse_response_head(b"HTTP/1.1 1000 Nope").is_err());
    }

    #[tokio::test]
    async fn the_head_is_split_from_the_bytes_that_follow_it() {
        let mut upstream: &[u8] =
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\nfirst";
        let head = read_upgrade_response_headers(&mut upstream).await.unwrap();
        assert_eq!(head.status, http::StatusCode::SWITCHING_PROTOCOLS);
        assert_eq!(head.leftover, b"first");
    }

    #[tokio::test]
    async fn an_unterminated_head_is_an_error() {
        let mut upstream: &[u8] = b"HTTP/1.1 101 Switching Protocols\r\n";
        assert_eq!(
            read_upgrade_response_headers(&mut upstream)
                .await
                .err()
                .unwrap(),
            "upstream closed before sending response headers"
        );
    }

    #[test]
    fn only_a_101_with_an_upgrade_header_switches_protocols() {
        let switched = head(101, &[("upgrade", "websocket")]);
        assert_eq!(switched.switched_protocol().as_deref(), Some("websocket"));
        assert_eq!(head(101, &[]).switched_protocol(), None);
        assert_eq!(
            head(403, &[("Upgrade", "websocket")]).switched_protocol(),
            None
        );
    }

    #[test]
    fn header_lookup_is_case_insensitive() {
        let h = head(101, &[("Sec-WebSocket-Protocol", "v5.channel.k8s.io")]);
        assert_eq!(
            h.header("sec-websocket-protocol"),
            Some("v5.channel.k8s.io")
        );
        assert_eq!(h.header("missing"), None);
    }

    #[test]
    fn hop_headers_are_not_relayed() {
        for name in ["Transfer-Encoding", "content-length", "Host", "Connection"] {
            assert!(is_dropped_response_header(name));
        }
        assert!(is_dropped_response_header("keep-alive"));
        assert!(is_dropped_response_header("UPGRADE"));
        assert!(!is_dropped_response_header("content-type"));
    }

    #[tokio::test]
    async fn a_content_length_body_is_read_exactly() {
        let mut upstream: &[u8] = b"lo, ignored";
        let headers = vec![("Content-Length".to_string(), b"5".to_vec())];
        let body = read_response_body(
            &mut upstream,
            http::StatusCode::FORBIDDEN,
            &headers,
            b"hel".to_vec(),
        )
        .await
        .unwrap();
        assert_eq!(body, b"hello");
    }

    #[tokio::test]
    async fn a_chunked_body_is_reassembled() {
        let mut upstream: &[u8] = b"3\r\nlo!\r\n0\r\n\r\n";
        let headers = vec![("transfer-encoding".to_string(), b"chunked".to_vec())];
        let body = read_response_body(
            &mut upstream,
            http::StatusCode::FORBIDDEN,
            &headers,
            b"2;ext=1\r\nhe\r\n".to_vec(),
        )
        .await
        .unwrap();
        assert_eq!(body, b"helo!");
    }

    #[tokio::test]
    async fn an_unframed_body_runs_until_close_and_bodyless_statuses_are_empty() {
        let mut upstream: &[u8] = b"rest";
        let body = read_response_body(
            &mut upstream,
            http::StatusCode::BAD_REQUEST,
            &[],
            b"the ".to_vec(),
        )
        .await
        .unwrap();
        assert_eq!(body, b"the rest");

        let mut upstream: &[u8] = b"ignored";
        let body = read_response_body(
            &mut upstream,
            http::StatusCode::NO_CONTENT,
            &[],
            b"x".to_vec(),
        )
        .await
        .unwrap();
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn an_oversized_content_length_is_refused() {
        let mut upstream: &[u8] = b"";
        let headers = vec![(
            "content-length".to_string(),
            (MAX_REFUSED_RESPONSE_BYTES + 1).to_string().into_bytes(),
        )];
        let err = read_response_body(&mut upstream, http::StatusCode::OK, &headers, Vec::new())
            .await
            .unwrap_err();
        assert_eq!(err, "upstream response body exceeded the allowed size");
    }

    #[tokio::test]
    async fn a_refused_handshake_is_relayed_and_closes_the_connection() {
        let mut upstream: &[u8] = b"";
        let mut refused = head(
            403,
            &[
                ("Content-Type", "application/json"),
                ("Content-Length", "2"),
            ],
        );
        refused.leftover = b"{}".to_vec();
        let response = refused_upgrade_response(&mut upstream, refused).await;
        assert_eq!(response.status(), http::StatusCode::FORBIDDEN);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json"
        );
        assert!(response.head().connection_type() == actix_web::http::ConnectionType::Close);
    }
}
