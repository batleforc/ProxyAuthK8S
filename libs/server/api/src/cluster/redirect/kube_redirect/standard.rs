use axum::{
    body::{Body, Bytes},
    http::{self, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::stream::StreamExt;
use tracing::{debug, error, instrument, warn};

use super::context::RedirectContext;
use super::upstream::{apply_forward_headers, upstream_client};
use crate::cluster::redirect::forwarded::is_hop_by_hop;
use crate::cluster::redirect::upstream_identity::Upstream;
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

/// Content-Length of a request/response, when the peer announced one.
fn announced_content_length(headers: &http::header::HeaderMap) -> Option<usize> {
    headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
}

fn body_for_debug_log(body: &[u8]) -> String {
    let Some((head, rest)) = body
        .split_at_checked(DEBUG_BODY_LOG_LIMIT)
        .filter(|(_, rest)| !rest.is_empty())
    else {
        return String::from_utf8_lossy(body).to_string();
    };

    format!(
        "{}... [truncated {} bytes]",
        String::from_utf8_lossy(head),
        rest.len()
    )
}

// `proxy` is deliberately `Empty` rather than `?ctx.proxy`: an `#[instrument]`
// field is formatted at span creation whatever the subscriber's level, so a
// direct binding dumped the whole `ProxyKubeApi` into the trace backend on every
// proxied request. It is filled in below only under DEBUG. The span still
// identifies the target cluster at any level through `url_to_call`.
#[instrument(
    skip_all,
    fields(method = ?ctx.method, peer_addr = ?ctx.peer_ip, proxy = tracing::field::Empty, url_to_call)
)]
pub(super) async fn standard_redirect(ctx: RedirectContext) -> Response {
    let url_to_call = ctx.url_to_call();
    let query_string = ctx.query_string().to_string();
    tracing::Span::current().record("url_to_call", ctx.url_without_query().as_str());
    let is_debug_enabled = tracing::enabled!(tracing::Level::DEBUG);
    if is_debug_enabled {
        tracing::Span::current().record("proxy", tracing::field::debug(&ctx.proxy));
    }
    let RedirectContext {
        req,
        body: payload,
        data,
        method,
        peer_ip,
        proxy,
        user,
        upstream_identity,
        audit,
        ..
    } = ctx;
    // watch=true/1 and follow=true/1 produce infinite streaming responses; treat them specially
    let is_streaming_request = query_string
        .split('&')
        .any(|p| p.starts_with("watch=") || p.starts_with("follow="));

    let debug_body_limit = debug_body_max_bytes();
    // Only buffer the request body for debug logging when the client announced a
    // size we are willing to hold in memory. Unknown or oversized bodies are
    // streamed through as usual and simply not logged.
    let buffer_request_body = is_debug_enabled
        && match announced_content_length(&req.headers) {
            Some(len) => len <= debug_body_limit,
            None => false,
        };

    if is_debug_enabled && !buffer_request_body {
        debug!(
            debug_body_limit,
            "request body not buffered for debug logging (unknown or oversized content-length)"
        );
    }

    let mut payload = payload.into_data_stream();
    let request_body = if buffer_request_body {
        let mut body = Vec::new();
        let mut overflowed = false;
        while let Some(item) = payload.next().await {
            match item {
                Ok(chunk) => {
                    if body.len().saturating_add(chunk.len()) > debug_body_limit {
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
            audit.emit_refusal(413);
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                "request body exceeds the configured proxy buffering limit",
            )
                .into_response();
        }

        let body = Bytes::from(body);
        debug!(
            request_body_len = body.len(),
            request_body = %body_for_debug_log(body.as_ref()),
            "standard redirect input body"
        );
        Some(body)
    } else {
        None
    };

    // Streamed straight into the upstream request body: hyper's own flow
    // control back-pressures the client when the upstream is slow.
    let request_stream = request_body.is_none().then_some(payload);

    let client = match upstream_client(&proxy, &data).await {
        Ok(client) => client,
        Err(err) => {
            error!(err, "couldn't build the upstream client");
            audit.emit_refusal(503);
            return (StatusCode::SERVICE_UNAVAILABLE, "upstream unavailable").into_response();
        }
    };

    // Same `http` crate on both sides: the method is forwarded as is.
    let mut forwarded_req = client.request(method, url_to_call);

    forwarded_req = match (request_body, request_stream) {
        (Some(request_body), _) => forwarded_req.body(request_body),
        // Convert the ReceiverStream<Bytes> into a stream of Result<Bytes, _>
        // which reqwest::Body::wrap_stream expects.
        (None, Some(request_stream)) => forwarded_req
            .body(reqwest::Body::wrap_stream(request_stream.map(|chunk| {
                chunk.inspect_err(|e| error!(%e, "error reading request payload"))
            }))),
        // Unreachable: exactly one of the two is always set above.
        (None, None) => forwarded_req,
    };

    forwarded_req = apply_forward_headers(
        forwarded_req,
        &req.headers,
        peer_ip,
        Upstream::new(user.as_ref(), upstream_identity.as_ref()),
    );

    // Kubernetes sends durations in Go format (`timeout=32s`, `1m30s`), so the
    // value has to be parsed as such and not as a bare number of seconds.
    // One extra second of slack so the upstream timeout fires first.
    if let Some(timeout) = extract_timeout_from_query(&query_string) {
        let capped = timeout.min(MAX_UPSTREAM_TIMEOUT);
        forwarded_req =
            forwarded_req.timeout(capped.saturating_add(std::time::Duration::from_secs(1)));
    }

    let res = match forwarded_req.send().await {
        Ok(res) => res,
        Err(e) => {
            tracing::error!(error = %e, "error forwarding request to cluster");
            audit.emit_refusal(503);
            return (StatusCode::SERVICE_UNAVAILABLE, "upstream unavailable").into_response();
        }
    };

    let response_status = res.status();
    if upstream_identity.is_some() && response_status == StatusCode::UNAUTHORIZED {
        audit.emit_refusal(502);
        return crate::cluster::redirect::status_response::untrusted_upstream(
            &proxy.to_path(),
            proxy.upstream_mode().as_str(),
        );
    }
    // Emitted as soon as the upstream answered: a watch stream stays open for
    // minutes and waiting for it to end would delay the audit record forever.
    audit.emit(response_status.as_u16());
    let response_headers = res.headers().clone();

    tracing::Span::current().record("http.response.status_code", response_status.as_u16());
    // Same `http` crate on both sides: status and headers are copied as they
    // are, minus the hop-by-hop ones.
    let mut client_resp = Response::builder().status(response_status);
    let mut has_content_encoding = false;
    if let Some(headers) = client_resp.headers_mut() {
        for (name, value) in &response_headers {
            if name == http::header::CONTENT_ENCODING {
                has_content_encoding = true;
            }
            if !is_hop_by_hop(name.as_str()) {
                headers.append(name, value.clone());
            }
        }
        // Keep the compression layer from buffering the proxied stream: it
        // leaves alone a response that already names its encoding. Without
        // one (common for k8s watch/logs), say "identity" so each chunk
        // reaches the client immediately.
        if !has_content_encoding {
            headers.insert(
                http::header::CONTENT_ENCODING,
                HeaderValue::from_static("identity"),
            );
        }
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
                client_resp
                    .body(Body::from(response_body))
                    .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
            }
            Err(e) => {
                error!(%e, "error reading response body for debug logging");
                (StatusCode::SERVICE_UNAVAILABLE, "upstream unavailable").into_response()
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
        client_resp
            .body(Body::from_stream(res.bytes_stream()))
            .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
    }
}
