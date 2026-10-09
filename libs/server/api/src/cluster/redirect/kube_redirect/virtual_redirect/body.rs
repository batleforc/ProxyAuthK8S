//! Reading request and response bodies for a translated virtual request.
//!
//! A rewritten response has to be held in memory in full, so every read here is
//! capped: [`read_response_capped`] aborts as soon as the limit is passed
//! instead of buffering everything and checking the size afterwards.

use actix_web::{HttpResponse, http, web};
use futures_util::StreamExt;
use serde_json::Value;

/// Upper bound on a buffered virtual response (`PROXY_VIRTUAL_MAX_BODY_BYTES`,
/// default 32 MiB, see `common::config`).
///
/// A translated response must be held in memory in full; a `NamespaceList` on a
/// very large cluster is the realistic worst case, and past this the request is
/// refused rather than allowed to grow without bound.
pub(super) fn max_buffered_bytes() -> usize {
    common::config::get().proxy.virtual_max_body_bytes
}

pub(super) fn json_response(status: http::StatusCode, body: &Value) -> HttpResponse {
    HttpResponse::build(status)
        .content_type("application/json")
        .body(body.to_string())
}

/// Read the client body, capped, so a mapper can rewrite it.
pub(super) async fn read_client_body(
    payload: &mut web::Payload,
    limit: usize,
) -> Result<web::Bytes, ReadCapError> {
    let mut body = web::BytesMut::new();
    while let Some(chunk) = payload.next().await {
        let chunk = chunk.map_err(|err| ReadCapError::Upstream(err.to_string()))?;
        if body.len() + chunk.len() > limit {
            return Err(ReadCapError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

/// Why a capped upstream read stopped short.
pub(super) enum ReadCapError {
    /// The body exceeded `limit`; aborted without buffering the rest.
    TooLarge,
    /// The upstream connection failed mid-read.
    Upstream(String),
}

/// Read an upstream response into memory, aborting as soon as it exceeds `limit`
/// instead of buffering the whole body first. A translated response must be held
/// in full, so without an incremental cap a multi-GB `NamespaceList` (or a
/// hostile upstream) would be read entirely into RAM before the size was checked.
pub(super) async fn read_response_capped(
    res: reqwest::Response,
    limit: usize,
) -> Result<web::Bytes, ReadCapError> {
    // Reject early when the upstream announced an oversized body.
    if let Some(len) = res.content_length()
        && len > limit as u64
    {
        return Err(ReadCapError::TooLarge);
    }
    let mut body = web::BytesMut::new();
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|err| ReadCapError::Upstream(err.to_string()))?;
        if body.len() + chunk.len() > limit {
            return Err(ReadCapError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}
