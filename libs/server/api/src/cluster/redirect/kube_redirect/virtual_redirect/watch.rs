//! Watch streams: translated event by event, because a watch never ends and so
//! can never be buffered the way an ordinary response is.

use axum::{
    body::Body,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use serde_json::Value;
use tracing::warn;
use virtual_api::MapperRegistry;

use crate::cluster::redirect::kube_redirect::virtual_redirect::body::max_buffered_bytes;

/// Translate a newline-delimited watch stream event by event.
pub(super) fn stream_watch(
    res: reqwest::Response,
    status: StatusCode,
    registry: MapperRegistry,
    upstream_path: String,
) -> Response {
    let mut buffer = BytesMut::new();
    // Cap a single unterminated line: a watch that never emits a newline (a
    // hostile or stuck upstream) would otherwise grow `buffer` without bound.
    let line_limit = max_buffered_bytes();
    let translated = res.bytes_stream().map(move |chunk| {
        let chunk = chunk.map_err(std::io::Error::other)?;
        buffer.extend_from_slice(&chunk);

        let mut out = BytesMut::new();
        // Only whole lines can be parsed; a partial one stays buffered until
        // the rest of it arrives.
        while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            let line = buffer.split_to(newline.saturating_add(1));
            let trimmed = line.strip_suffix(b"\n").unwrap_or(&line);
            if trimmed.is_empty() {
                continue;
            }

            match serde_json::from_slice::<Value>(trimmed) {
                Ok(event) => {
                    let event = match registry.resolve(&upstream_path) {
                        Some((mapper, _)) => mapper.map_watch_event(event),
                        None => event,
                    };
                    out.extend_from_slice(event.to_string().as_bytes());
                    out.extend_from_slice(b"\n");
                }
                Err(err) => {
                    // Pass unparseable lines through untouched rather than
                    // silently dropping part of the stream.
                    warn!(error = %err, "watch event is not JSON, forwarding it untouched");
                    out.extend_from_slice(&line);
                }
            }
        }

        // Whatever is left is an as-yet-unterminated line; refuse to let it grow
        // past the limit.
        if buffer.len() > line_limit {
            return Err(std::io::Error::other("watch line exceeds the size limit"));
        }

        Ok::<Bytes, std::io::Error>(out.freeze())
    });

    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            // Keep the compression layer from buffering the stream.
            (header::CONTENT_ENCODING, "identity"),
        ],
        Body::from_stream(translated),
    )
        .into_response()
}
