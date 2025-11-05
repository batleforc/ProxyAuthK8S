use actix_web::{dev::PeerAddr, http, web, HttpRequest, HttpResponse, Responder};
use common::State;
use crd::ProxyKubeApi;
use deadpool_redis::redis::AsyncTypedCommands;
use futures_util::stream::StreamExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{error, info, instrument};

#[instrument(name = "main_redirect", level = "warn", skip(data, payload))]
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
        Err(e) => return HttpResponse::ServiceUnavailable().body(e.to_string()),
    };
    let proxy_json = match conn.get(format!("proxyk8sauth:{}/{}", ns, cluster)).await {
        Ok(Some(proxy)) => proxy,
        Ok(None) => return HttpResponse::NotFound().finish(),
        Err(e) => return HttpResponse::ServiceUnavailable().body(e.to_string()),
    };
    // TODO: MTLS ? https://github.com/actix/examples/blob/master/https-tls/rustls-client-cert/src/main.rs
    let proxy = match ProxyKubeApi::from_json(&proxy_json) {
        Some(proxy) => proxy,
        None => {
            error!("Couldn't parse object");
            return HttpResponse::NotFound().finish();
        }
    };

    let url_to_call = match proxy
        .spec
        .service
        .url_to_call(data.client.clone(), "default".to_string())
        .await
    {
        Ok(url) => req
            .uri()
            .to_string()
            .replace(&format!("/clusters/{}/{}", ns, cluster), &url),
        Err(err) => {
            error!(err);
            return HttpResponse::NotFound().finish();
        }
    };
    info!(
        "Forwarding request from {} to {} with method {}",
        req.uri().to_string(),
        url_to_call,
        method.as_str()
    );
    let (tx, rx) = mpsc::unbounded_channel();
    actix_web::rt::spawn(async move {
        while let Some(chunk) = payload.next().await {
            tx.send(chunk).unwrap();
        }
    });
    let client = match proxy.get_client(data.into_inner()).await {
        Ok(c) => c,
        Err(err) => {
            error!(err);
            return HttpResponse::ServiceUnavailable().body(err);
        }
    };
    let mut forwarded_req = client
        .request(
            reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap(),
            url_to_call.clone(),
        )
        .body(reqwest::Body::wrap_stream(UnboundedReceiverStream::new(rx)));
    for (h, v) in req.headers().iter() {
        forwarded_req = forwarded_req.header(h.as_str(), v.clone().to_str().unwrap());
        info!("Header: {}: {}", h.as_str(), v.clone().to_str().unwrap());
    }
    if let Some(PeerAddr(addr)) = peer_addr {
        forwarded_req = forwarded_req.header("x-forwarded-for", addr.ip().to_string());
    }

    let res = match forwarded_req.send().await {
        Ok(res) => res,
        Err(e) => {
            tracing::error!(error = %e);
            return HttpResponse::ServiceUnavailable().body(e.to_string());
        }
    };

    let mut client_resp =
        HttpResponse::build(actix_web::http::StatusCode::from_u16(res.status().as_u16()).unwrap());

    for (header_name, header_value) in res.headers().iter().filter(|(h, _)| *h != "connection") {
        client_resp.insert_header((
            actix_web::http::header::HeaderName::from_bytes(header_name.as_ref()).unwrap(),
            actix_web::http::header::HeaderValue::from_bytes(header_value.as_ref()).unwrap(),
        ));
    }

    client_resp.streaming(res.bytes_stream())
    // https://github.com/actix/examples/blob/master/http-proxy/src/main.rs#L56
}
