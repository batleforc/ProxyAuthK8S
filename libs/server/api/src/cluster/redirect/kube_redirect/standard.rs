use super::RedirectContext;
use actix_web::{HttpResponse, http, web};
use futures_util::stream::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info, instrument, warn};

use super::upstream::{apply_forward_headers, upstream_client};
use crate::cluster::redirect::forwarded::is_hop_by_hop;
use crate::duration::extract_timeout_from_query;

const DEBUG_BODY_LOG_LIMIT: usize = 8 * 1024;

/// Upper bound on the upstream request timeout the client may ask for via
/// `?timeout=`. The value is client-controlled and feeds our own `reqwest`
/// timeout, so an unbounded `timeout=100000h` would otherwise let a caller pin a
/// proxy worker/connection open indefinitely. One hour comfortably covers a
/// legitimate long-lived watch while capping abuse.
const MAX_UPSTREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_hours(1);

/// Upper bound on the memory a single request may buffer when debug logging is
/// enabled (`PROXY_DEBUG_BODY_MAX_BYTES`, see `common::config`). Beyond that the
/// body is streamed through untouched and simply not logged.
fn debug_body_max_bytes() -> usize {
    common::config::get().proxy.debug_body_max_bytes
}

/// In-flight chunks between the client payload reader and the upstream request
/// body (`PROXY_STREAM_CHANNEL_CAPACITY`): bounded so a slow upstream applies
/// back-pressure instead of accumulating the whole body in memory.
fn stream_channel_capacity() -> usize {
    common::config::get().proxy.stream_channel_capacity
}

/// Content-Length of a request/response, when the peer announced one.
fn announced_content_length(headers: &http::header::HeaderMap) -> Option<usize> {
    headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
}

fn body_for_debug_log(body: &[u8]) -> String {
    if body.len() <= DEBUG_BODY_LOG_LIMIT {
        return String::from_utf8_lossy(body).to_string();
    }

    format!(
        "{}... [truncated {} bytes]",
        String::from_utf8_lossy(&body[..DEBUG_BODY_LOG_LIMIT]),
        body.len() - DEBUG_BODY_LOG_LIMIT
    )
}

#[instrument(skip(ctx), fields(http.method = %ctx.method))]
pub(super) async fn standard_redirect(ctx: RedirectContext, url_to_call: String) -> HttpResponse {
    let RedirectContext {
        req,
        data,
        mut payload,
        method,
        peer_addr,
        proxy,
        user,
        audit,
    } = ctx;
    let is_debug_enabled = tracing::enabled!(tracing::Level::DEBUG);
    // watch=true/1 and follow=true/1 produce infinite streaming responses; treat them specially
    let is_streaming_request = req
        .query_string()
        .split('&')
        .any(|p| p.starts_with("watch=") || p.starts_with("follow="));

    let debug_body_limit = debug_body_max_bytes();
    // Only buffer the request body for debug logging when the client announced a
    // size we are willing to hold in memory. Unknown or oversized bodies are
    // streamed through as usual and simply not logged.
    let buffer_request_body = is_debug_enabled
        && match announced_content_length(req.headers()) {
            Some(len) => len <= debug_body_limit,
            None => false,
        };

    if is_debug_enabled && !buffer_request_body {
        debug!(
            debug_body_limit,
            "request body not buffered for debug logging (unknown or oversized content-length)"
        );
    }

    let request_body = if buffer_request_body {
        let mut body = web::BytesMut::new();
        let mut overflowed = false;
        while let Some(item) = payload.next().await {
            match item {
                Ok(chunk) => {
                    if body.len() + chunk.len() > debug_body_limit {
                        overflowed = true;
                        break;
                    }
                    body.extend_from_slice(&chunk);
                }
                Err(e) => {
                    error!(%e, "error reading request payload");
                    break;
                }
            }
        }

        if overflowed {
            // The body was larger than the announced content-length; it has been
            // partially consumed and can no longer be forwarded faithfully.
            warn!(
                debug_body_limit,
                "request body exceeded the debug buffering limit"
            );
            audit.emit(413);
            return HttpResponse::PayloadTooLarge()
                .body("request body exceeds the configured proxy buffering limit");
        }

        let body = body.freeze();
        debug!(
            request_body_len = body.len(),
            request_body = %body_for_debug_log(body.as_ref()),
            "standard redirect input body"
        );
        Some(body)
    } else {
        None
    };

    let request_stream = if request_body.is_none() {
        // Stream the request payload into a bounded tokio channel as raw bytes.
        // The bound is what makes a slow upstream back-pressure the client
        // instead of letting the whole body pile up in memory.
        // Only forward successful chunks; on a payload error log and stop.
        let (tx, rx) = mpsc::channel::<web::Bytes>(stream_channel_capacity());
        actix_web::rt::spawn(async move {
            while let Some(item) = payload.next().await {
                match item {
                    Ok(chunk) => {
                        // send bytes, but stop if receiver was dropped
                        if tx.send(chunk).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        error!(%e, "error reading request payload");
                        break;
                    }
                }
            }

            // tx is dropped here when the task ends which closes the stream for the receiver
        });
        Some(rx)
    } else {
        None
    };

    let client = match upstream_client(&proxy, &data).await {
        Ok(client) => client,
        Err(err) => {
            error!(err, "couldn't build the upstream client");
            audit.emit(503);
            return HttpResponse::ServiceUnavailable().body("upstream unavailable");
        }
    };

    let upstream_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(m) => m,
        Err(err) => {
            error!(error = %err, method = %method.as_str(), "unsupported HTTP method");
            audit.emit(405);
            return HttpResponse::MethodNotAllowed().body("unsupported HTTP method");
        }
    };

    let mut forwarded_req = client.request(upstream_method, url_to_call);

    forwarded_req = match (request_body, request_stream) {
        (Some(request_body), _) => forwarded_req.body(request_body),
        // Convert the ReceiverStream<Bytes> into a stream of Result<Bytes, _>
        // which reqwest::Body::wrap_stream expects.
        (None, Some(request_stream)) => forwarded_req.body(reqwest::Body::wrap_stream(
            ReceiverStream::new(request_stream).map(Ok::<web::Bytes, std::io::Error>),
        )),
        // Unreachable: exactly one of the two is always set above.
        (None, None) => forwarded_req,
    };

    forwarded_req = apply_forward_headers(forwarded_req, &req, peer_addr, user.as_ref());

    // Kubernetes sends durations in Go format (`timeout=32s`, `1m30s`), so the
    // value has to be parsed as such and not as a bare number of seconds.
    // One extra second of slack so the upstream timeout fires first.
    if let Some(timeout) = extract_timeout_from_query(req.query_string()) {
        let capped = timeout.min(MAX_UPSTREAM_TIMEOUT);
        forwarded_req = forwarded_req.timeout(capped + std::time::Duration::from_secs(1));
    }

    let res = match forwarded_req.send().await {
        Ok(res) => res,
        Err(e) => {
            tracing::error!(error = %e, "error forwarding request to cluster");
            audit.emit(503);
            return HttpResponse::ServiceUnavailable().body("upstream unavailable");
        }
    };

    let response_status = res.status();
    // Emitted as soon as the upstream answered: a watch stream stays open for
    // minutes and waiting for it to end would delay the audit record forever.
    audit.emit(response_status.as_u16());
    let response_headers = res.headers().clone();

    tracing::Span::current().record("http.response.status_code", response_status.as_u16());
    // An upstream status we cannot represent is a broken gateway, not a reason
    // to panic the worker.
    let client_status = match actix_web::http::StatusCode::from_u16(response_status.as_u16()) {
        Ok(status) => status,
        Err(err) => {
            error!(error = %err, upstream_status = response_status.as_u16(), "invalid upstream status code");
            actix_web::http::StatusCode::BAD_GATEWAY
        }
    };
    let mut client_resp = HttpResponse::build(client_status);

    // Track whether the upstream already sent a Content-Encoding so we know whether to
    // add "identity" ourselves to stop actix-web's Compress middleware from buffering the stream.
    let mut has_content_encoding = false;
    for (header_name, header_value) in &response_headers {
        let name = header_name.as_str();
        if name.eq_ignore_ascii_case("content-encoding") {
            has_content_encoding = true;
        }
        // Skip headers that must not be forwarded or are managed by reqwest when streaming
        if is_hop_by_hop(name) {
            continue;
        }

        // Only forward header values that are valid UTF-8 strings. If not valid, skip them.
        let Ok(value_str) = header_value.to_str() else {
            // non-utf8 header; skip it to avoid conversion issues
            info!(header = %name, "skipping non-utf8 header");
            continue;
        };

        // An upstream may send a header actix cannot represent; drop it rather
        // than panicking the worker.
        match (
            actix_web::http::header::HeaderName::from_bytes(name.as_bytes()),
            actix_web::http::header::HeaderValue::from_bytes(value_str.as_bytes()),
        ) {
            (Ok(header_name), Ok(header_value)) => {
                client_resp.insert_header((header_name, header_value));
            }
            _ => {
                warn!(header = %name, "skipping response header actix cannot represent");
            }
        }
    }

    // Prevent actix-web's Compress middleware from gzip-buffering the proxied stream.
    // Compress skips compression whenever Content-Encoding is already present in the response.
    // If the upstream didn't send one (common for k8s watch/logs), inject "identity" so
    // Compress leaves the byte stream untouched and each chunk reaches the client immediately.
    if !has_content_encoding {
        client_resp.insert_header(("content-encoding", "identity"));
    }

    // Same rule as for the request: only hold the response in memory when the
    // upstream announced a size we accept to buffer.
    let buffer_response_body = is_debug_enabled
        && !is_streaming_request
        && match res.content_length() {
            Some(len) => len <= debug_body_limit as u64,
            None => false,
        };

    if buffer_response_body {
        match res.bytes().await {
            Ok(response_body) => {
                debug!(
                    response_body_len = response_body.len(),
                    response_body = %body_for_debug_log(response_body.as_ref()),
                    "standard redirect output body"
                );
                client_resp.body(response_body)
            }
            Err(e) => {
                error!(%e, "error reading response body for debug logging");
                HttpResponse::ServiceUnavailable().body("upstream unavailable")
            }
        }
    } else {
        if is_debug_enabled {
            debug!(
                is_streaming_request,
                debug_body_limit, "skipping response body debug log, streaming it through instead"
            );
        }
        // Copy the response body stream directly to the client response
        client_resp.streaming(res.bytes_stream())
    }
}
