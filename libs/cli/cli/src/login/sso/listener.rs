//! The local loopback listener that catches the provider's redirect.
//!
//! Both loopback families are bound because the redirect URI is registered as
//! `localhost`, which may resolve to `127.0.0.1` or `::1` depending on the host
//! and on whichever opener the provider hands the URL to.

use std::future::poll_fn;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::task::Poll;
use std::time::Duration;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing::debug;

use crate::{
    error::ProxyAuthK8sError,
    login::sso::callback::{CallbackResult, parse_callback},
};

/// How long to wait for the browser round-trip before giving up.
const CALLBACK_TIMEOUT: Duration = Duration::from_mins(3);

/// Minimal HTML shown in the browser once the callback has been received.
const SUCCESS_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>ProxyAuthK8S</title></head><body style=\"font-family:sans-serif\">\
<h2>Login complete</h2><p>You can close this tab and return to your terminal.</p>\
</body></html>";

/// Minimal HTML shown in the browser when the provider redirected back with an
/// error. The details are surfaced on the terminal, not the page.
const ERROR_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>ProxyAuthK8S</title></head><body style=\"font-family:sans-serif\">\
<h2>Login failed</h2><p>The sign-in was not completed. Check your terminal for details.</p>\
</body></html>";

/// Bind a loopback listener on the IPv4 (`127.0.0.1`) and IPv6 (`::1`) loopbacks
/// for `port`. Both are attempted; only one needs to succeed (IPv6 may be
/// disabled, and vice-versa). Errors only when neither family can be bound.
pub(super) async fn bind_loopback_listeners(
    port: u16,
) -> Result<Vec<TcpListener>, ProxyAuthK8sError> {
    let mut listeners = Vec::with_capacity(2);
    let mut last_err = None;
    for addr in [
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ] {
        match TcpListener::bind((addr, port)).await {
            Ok(listener) => listeners.push(listener),
            Err(e) => {
                debug!("could not bind the callback listener on {addr}:{port}: {e}");
                last_err = Some(e);
            }
        }
    }
    if listeners.is_empty() {
        let detail = last_err.map_or_else(
            || "no loopback address available".to_string(),
            |e| e.to_string(),
        );
        return Err(ProxyAuthK8sError::SsoLoginError(format!(
            "could not bind the local callback listener on 127.0.0.1:{port} or [::1]:{port} \
             ({detail}). Set PROXYAUTH_CALLBACK_PORT to a free port registered at your IdP."
        )));
    }
    Ok(listeners)
}

/// Accept the next connection ready on any of `listeners`.
async fn accept_any(listeners: &[TcpListener]) -> io::Result<TcpStream> {
    poll_fn(|cx| {
        for listener in listeners {
            if let Poll::Ready(res) = listener.poll_accept(cx) {
                return Poll::Ready(res.map(|(stream, _)| stream));
            }
        }
        Poll::Pending
    })
    .await
}

/// Accept loopback connections until one carries an OIDC `code`+`state`, then
/// answer the browser with a small success page. Times out after
/// [`CALLBACK_TIMEOUT`] so the CLI never hangs forever.
pub(super) async fn wait_for_callback(
    listeners: &[TcpListener],
) -> Result<(String, String), ProxyAuthK8sError> {
    let accept = async {
        loop {
            let mut stream = accept_any(listeners).await.map_err(|e| {
                ProxyAuthK8sError::SsoLoginError(format!("callback listener failed: {e}"))
            })?;

            // The request line (`GET /...?code=..&state=.. HTTP/1.1`) is all we
            // need; a single read of the first packet holds it.
            let mut buffer = [0u8; 8192];
            let read = stream.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]);
            let target = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("");

            match parse_callback(target) {
                Some(CallbackResult::Success { code, state }) => {
                    respond(&mut stream, "200 OK", SUCCESS_PAGE).await;
                    return Ok((code, state));
                }
                Some(CallbackResult::ProviderError { error, description }) => {
                    // The IdP redirected back with an error (e.g. the user
                    // declined consent, or the OIDC client is misconfigured).
                    // Surface it now instead of waiting out the whole timeout.
                    respond(&mut stream, "200 OK", ERROR_PAGE).await;
                    let detail = match description {
                        Some(desc) => format!("{error}: {desc}"),
                        None => error,
                    };
                    return Err(ProxyAuthK8sError::SsoLoginError(format!(
                        "the identity provider returned an error ({detail})"
                    )));
                }
                None => {
                    // Browsers also fetch /favicon.ico etc.; acknowledge and wait
                    // for the real redirect.
                    debug!(
                        target,
                        "ignoring non-callback request on the loopback listener"
                    );
                    respond(&mut stream, "404 Not Found", "").await;
                }
            }
        }
    };

    match tokio::time::timeout(CALLBACK_TIMEOUT, accept).await {
        Ok(result) => result,
        Err(_) => Err(ProxyAuthK8sError::SsoLoginError(format!(
            "timed out after {}s waiting for the SSO callback",
            CALLBACK_TIMEOUT.as_secs()
        ))),
    }
}

async fn respond(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
}
