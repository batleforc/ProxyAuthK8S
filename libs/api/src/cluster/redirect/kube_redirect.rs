use actix_web::{dev::PeerAddr, http, web, HttpRequest, HttpResponse, Responder};
use common::State;
use crd::ProxyKubeApi;
use deadpool_redis::redis::AsyncTypedCommands;
use futures_util::stream::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{error, info, instrument};

#[instrument(name = "main_redirect",fields(http.method= ?method, http.response.status_code) ,skip(req, data, payload))]
pub async fn redirect(
    req: HttpRequest,
    data: web::Data<State>,
    mut payload: web::Payload,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
) -> impl Responder {
    let ns: String = req.match_info().get("ns").unwrap().parse().unwrap();
    let cluster: String = req.match_info().get("cluster").unwrap().parse().unwrap();
    let mut conn = match data.get_redis_conn().await {
        Ok(conn) => conn,
        Err(e) => {
            error!(error = %e, " couldn't get redis connection");
            return HttpResponse::ServiceUnavailable().body(e.to_string());
        }
    };
    let proxy_json = match conn.get(format!("proxyk8sauth:{}/{}", ns, cluster)).await {
        Ok(Some(proxy)) => proxy,
        Ok(None) => return HttpResponse::NotFound().finish(),
        Err(e) => {
            error!(error = %e, " couldn't get proxy from redis");
            return HttpResponse::ServiceUnavailable().body(e.to_string());
        }
    };
    // TODO: MTLS ? https://github.com/actix/examples/blob/master/https-tls/rustls-client-cert/src/main.rs
    let proxy = match ProxyKubeApi::from_json(&proxy_json) {
        Some(proxy) => proxy,
        None => {
            error!("Couldn't parse object");
            return HttpResponse::NotFound().finish();
        }
    };

    if !proxy.spec.enabled {
        return HttpResponse::NotFound().finish();
    }

    let url_to_call = match proxy
        .spec
        .service
        .url_to_call(data.client.clone(), "default".to_string())
        .await
    {
        Ok(url) => {
            let base_url = req
                .path()
                .to_string()
                .replace(&format!("/clusters/{}/{}", ns, cluster), &url);
            let query_string = req.query_string();
            if !query_string.is_empty() {
                format!("{}?{}", base_url, query_string)
            } else {
                base_url
            }
        }
        Err(err) => {
            error!(err, " couldn't get url to call");
            return HttpResponse::NotFound().finish();
        }
    };
    info!(from = %req.uri().to_string(), to = %url_to_call, method = %method.as_str(),
        "Forwarding request from {} to {} with method {}",
        req.uri().to_string(),
        url_to_call,
        method.as_str()
    );
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
    let client = match proxy.get_client(data.into_inner()).await {
        Ok(c) => c,
        Err(err) => {
            error!(err, " couldn't get client");
            return HttpResponse::ServiceUnavailable().body(err);
        }
    };
    let mut forwarded_req = client
        .request(
            reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap(),
            url_to_call.clone(),
        )
        // Convert the UnboundedReceiverStream<Bytes> into a stream of Result<Bytes, _>
        // which reqwest::Body::wrap_stream expects.
        .body(reqwest::Body::wrap_stream(
            UnboundedReceiverStream::new(rx).map(Ok::<web::Bytes, std::io::Error>),
        ));
    // Forward headers but skip hop-by-hop headers and content-length/transfer-encoding
    for (h, v) in req.headers().iter() {
        let name = h.as_str();
        // Skip headers that must not be forwarded or are managed by reqwest when streaming
        if name.eq_ignore_ascii_case("connection")
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
    // https://github.com/actix/examples/blob/master/http-proxy/src/main.rs#L56
}
