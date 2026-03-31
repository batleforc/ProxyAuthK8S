use std::sync::Arc;

use actix_web::{dev::PeerAddr, http, web, HttpRequest, HttpResponse};
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
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{error, instrument};

use super::tls::build_tls_config;

pub(super) fn is_upgrade_request(req: &HttpRequest) -> bool {
    let has_upgrade_header = req.headers().contains_key(http::header::UPGRADE);
    let connection_has_upgrade_token = req
        .headers()
        .get(http::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',').any(|token| {
                token
                    .trim()
                    .eq_ignore_ascii_case(http::header::UPGRADE.as_str())
            })
        })
        .unwrap_or(false);

    has_upgrade_header || connection_has_upgrade_token
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
) -> Vec<u8> {
    let path = match upstream_url.query() {
        Some(query) => format!("{}?{}", upstream_url.path(), query),
        None => upstream_url.path().to_string(),
    };
    let authority = upstream_url
        .port()
        .map(|port| format!("{}:{}", upstream_url.host_str().unwrap_or_default(), port))
        .unwrap_or_else(|| upstream_url.host_str().unwrap_or_default().to_string());

    let mut request_bytes = format!("{} {} HTTP/1.1\r\n", method.as_str(), path).into_bytes();
    request_bytes.extend_from_slice(format!("Host: {}\r\n", authority).as_bytes());

    for (header_name, header_value) in req.headers() {
        if header_name == http::header::HOST {
            continue;
        }

        request_bytes.extend_from_slice(header_name.as_str().as_bytes());
        request_bytes.extend_from_slice(b": ");
        request_bytes.extend_from_slice(header_value.as_bytes());
        request_bytes.extend_from_slice(b"\r\n");
    }

    if let Some(PeerAddr(addr)) = peer_addr {
        request_bytes.extend_from_slice(format!("x-forwarded-for: {}\r\n", addr.ip()).as_bytes());
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

#[instrument(skip(req, data, payload))]
pub(super) async fn upgrade_redirect(
    req: HttpRequest,
    data: web::Data<State>,
    payload: web::Payload,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
    proxy: ProxyKubeApi,
    url_to_call: String,
) -> HttpResponse {
    let upstream_url = match reqwest::Url::parse(&url_to_call) {
        Ok(url) => url,
        Err(err) => return HttpResponse::BadGateway().body(err.to_string()),
    };

    let mut upstream = match connect_upgrade_stream(&proxy, &data, &upstream_url).await {
        Ok(stream) => stream,
        Err(err) => return HttpResponse::ServiceUnavailable().body(err),
    };

    let request_bytes = serialize_upgrade_request(&req, &method, &upstream_url, peer_addr);
    if let Err(err) = upstream.write_all(&request_bytes).await {
        return HttpResponse::ServiceUnavailable().body(err.to_string());
    }
    if let Err(err) = upstream.flush().await {
        return HttpResponse::ServiceUnavailable().body(err.to_string());
    }

    let (status, headers, leftover) = match read_upgrade_response_headers(&mut upstream).await {
        Ok(response) => response,
        Err(err) => return HttpResponse::BadGateway().body(err),
    };

    tracing::Span::current().record("http.response.status_code", status.as_u16());

    let (mut upstream_reader, mut upstream_writer) = tokio::io::split(upstream);
    let (tx, rx) = mpsc::unbounded_channel::<web::Bytes>();

    if !leftover.is_empty() {
        let _ = tx.send(web::Bytes::from(leftover));
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
    if status == http::StatusCode::SWITCHING_PROTOCOLS {
        if let Some((_, upgrade_value)) = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("upgrade"))
        {
            if let Ok(upgrade_value) = std::str::from_utf8(upgrade_value) {
                client_resp.upgrade(upgrade_value);
            }
        }
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

    client_resp.streaming(UnboundedReceiverStream::new(rx).map(Ok::<web::Bytes, actix_web::Error>))
}
