use actix_web::{dev::PeerAddr, http, web, HttpRequest, HttpResponse};
use common::State;
use crd::ProxyKubeApi;
use futures_util::stream::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{error, info};

use super::tls::build_tls_config;

pub(super) async fn standard_redirect(
    req: HttpRequest,
    data: web::Data<State>,
    mut payload: web::Payload,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
    proxy: ProxyKubeApi,
    url_to_call: String,
) -> HttpResponse {
    // Stream the request payload into a tokio unbounded channel as raw bytes.
    // Only forward successful chunks; on a payload error log and stop.
    let (tx, rx) = mpsc::unbounded_channel::<web::Bytes>();
    actix_web::rt::spawn(async move {
        while let Some(item) = payload.next().await {
            match item {
                Ok(chunk) => {
                    // send bytes, but stop if receiver was dropped
                    if tx.send(chunk).is_err() {
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

    let tls_config = match build_tls_config(&proxy, &data).await {
        Ok(config) => config,
        Err(err) => {
            error!(err, " couldn't build TLS config");
            return HttpResponse::ServiceUnavailable().body(err);
        }
    };

    let client = match reqwest::ClientBuilder::new()
        .use_preconfigured_tls(tls_config)
        .build()
    {
        Ok(c) => c,
        Err(err) => {
            error!(error = %err, " couldn't get client");
            return HttpResponse::ServiceUnavailable().body(err.to_string());
        }
    };

    let mut forwarded_req = client
        .request(
            reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap(),
            url_to_call,
        )
        // Convert the UnboundedReceiverStream<Bytes> into a stream of Result<Bytes, _>
        // which reqwest::Body::wrap_stream expects.
        .body(reqwest::Body::wrap_stream(
            UnboundedReceiverStream::new(rx).map(Ok::<web::Bytes, std::io::Error>),
        ));

    for (h, v) in req.headers().iter() {
        let name = h.as_str();
        // Skip headers that must not be forwarded or are managed by reqwest when streaming
        if name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("upgrade")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("host")
        {
            continue;
        }

        // Only forward header values that are valid UTF-8 strings. If not valid, skip them.
        if let Ok(value_str) = v.to_str() {
            forwarded_req = forwarded_req.header(name, value_str);
        } else {
            // non-utf8 header; skip it to avoid conversion issues
            info!(header = %name, "skipping non-utf8 header");
        }
    }

    if let Some(PeerAddr(addr)) = peer_addr {
        forwarded_req = forwarded_req.header("x-forwarded-for", addr.ip().to_string());
    }

    if req.query_string().contains("timeout=") {
        if let Some(timeout_val) = req.query_string().split('&').find_map(|param| {
            if param.starts_with("timeout=") {
                Some(param.replace("timeout=", ""))
            } else {
                None
            }
        }) {
            if let Ok(timeout_secs) = timeout_val.parse::<u64>() {
                forwarded_req =
                    forwarded_req.timeout(std::time::Duration::from_secs(timeout_secs + 1));
            }
        }
    }

    let res = match forwarded_req.send().await {
        Ok(res) => res,
        Err(e) => {
            tracing::error!(error = %e, " error forwarding request to cluster");
            return HttpResponse::ServiceUnavailable().body(e.to_string());
        }
    };

    tracing::Span::current().record("http.response.status_code", res.status().as_u16());
    let mut client_resp =
        HttpResponse::build(actix_web::http::StatusCode::from_u16(res.status().as_u16()).unwrap());

    for (header_name, header_value) in res.headers().iter() {
        let name = header_name.as_str();
        // Skip headers that must not be forwarded or are managed by reqwest when streaming
        if name.eq_ignore_ascii_case("connection")
            || name.eq_ignore_ascii_case("upgrade")
            || name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("host")
        {
            continue;
        }

        // Only forward header values that are valid UTF-8 strings. If not valid, skip them.
        if let Ok(value_str) = header_value.to_str() {
            client_resp.insert_header((
                actix_web::http::header::HeaderName::from_bytes(name.as_ref()).unwrap(),
                actix_web::http::header::HeaderValue::from_bytes(value_str.as_ref()).unwrap(),
            ));
        } else {
            // non-utf8 header; skip it to avoid conversion issues
            info!(header = %name, "skipping non-utf8 header");
        }
    }

    // Copy the response body stream directly to the client response
    client_resp.streaming(res.bytes_stream())
}
