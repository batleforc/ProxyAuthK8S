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

use tokio::sync::mpsc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing::{debug, warn};

use crate::{
    error::ProxyAuthK8sError,
    login::sso::callback::{CallbackResult, parse_callback, parse_pasted},
};

/// How long to wait for the browser round-trip before giving up.
const CALLBACK_TIMEOUT: Duration = Duration::from_mins(3);
/// Longer when the user may be signing in on another machine and copying the
/// redirect URL back by hand.
const PASTE_TIMEOUT: Duration = Duration::from_mins(10);

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

/// Wait for the provider's redirect: on the loopback listener, or, when
/// `pasted` is given, as a URL the user pastes (browser on another machine,
/// whose redirect to `localhost` cannot reach this one). Whichever comes
/// first wins. Times out after [`CALLBACK_TIMEOUT`], or [`PASTE_TIMEOUT`]
/// when pasting is possible, so the CLI never hangs forever.
pub(super) async fn wait_for_callback(
    listeners: &[TcpListener],
    pasted: Option<mpsc::Receiver<String>>,
) -> Result<(String, String), ProxyAuthK8sError> {
    let timeout = if pasted.is_some() {
        PASTE_TIMEOUT
    } else {
        CALLBACK_TIMEOUT
    };
    let wait = async {
        match pasted {
            None => accept_callback(listeners).await,
            Some(pasted) => tokio::select! {
                result = accept_callback(listeners) => result,
                result = read_pasted_callback(pasted) => result,
            },
        }
    };
    match tokio::time::timeout(timeout, wait).await {
        Ok(result) => result,
        Err(_) => Err(ProxyAuthK8sError::SsoLoginError(format!(
            "timed out after {}s waiting for the SSO callback",
            timeout.as_secs()
        ))),
    }
}

/// Take pasted lines until one is a redirect URL. A closed input (stdin at
/// EOF) leaves the loopback listener as the only way in.
async fn read_pasted_callback(
    mut pasted: mpsc::Receiver<String>,
) -> Result<(String, String), ProxyAuthK8sError> {
    while let Some(line) = pasted.recv().await {
        match parse_pasted(&line) {
            Some(CallbackResult::Success { code, state }) => return Ok((code, state)),
            Some(CallbackResult::ProviderError { error, description }) => {
                return Err(provider_error(error, description));
            }
            None => warn!(
                "That is not the redirect URL (it must contain code= and state=); \
                 paste the full address of the page the browser landed on."
            ),
        }
    }
    std::future::pending().await
}

fn provider_error(error: String, description: Option<String>) -> ProxyAuthK8sError {
    let detail = match description {
        Some(desc) => format!("{error}: {desc}"),
        None => error,
    };
    ProxyAuthK8sError::SsoLoginError(format!(
        "the identity provider returned an error ({detail})"
    ))
}

/// Accept loopback connections until one carries an OIDC `code`+`state`, then
/// answer the browser with a small success page.
async fn accept_callback(listeners: &[TcpListener]) -> Result<(String, String), ProxyAuthK8sError> {
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
                return Err(provider_error(error, description));
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
}

async fn respond(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    /// Stand in for the browser: send `GET <target>` to `addr` and return the
    /// raw HTTP response.
    async fn browse(addr: SocketAddr, target: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let request = format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    /// Listeners on a free port (port 0: each family gets its own), and the
    /// address of the first one.
    async fn listeners() -> (Vec<TcpListener>, SocketAddr) {
        let listeners = bind_loopback_listeners(0).await.unwrap();
        let addr = listeners[0].local_addr().unwrap();
        (listeners, addr)
    }

    #[tokio::test]
    async fn the_code_and_state_are_captured_after_ignoring_other_requests() {
        let (listeners, addr) = listeners().await;
        let browser = async {
            let favicon = browse(addr, "/favicon.ico").await;
            let callback = browse(addr, "/?code=the-code&state=the-state").await;
            (favicon, callback)
        };

        let (result, (favicon, callback)) =
            tokio::join!(wait_for_callback(&listeners, None), browser);

        assert_eq!(
            result.unwrap(),
            ("the-code".to_string(), "the-state".to_string())
        );
        assert!(favicon.starts_with("HTTP/1.1 404 Not Found"));
        assert!(callback.starts_with("HTTP/1.1 200 OK"));
        assert!(callback.contains("Login complete"));
    }

    #[tokio::test]
    async fn a_provider_error_ends_the_wait_with_its_details() {
        let cases = [
            (
                "/?error=access_denied&error_description=user%20said%20no",
                "access_denied: user said no",
            ),
            ("/?error=invalid_request", "(invalid_request)"),
        ];
        for (target, detail) in cases {
            let (listeners, addr) = listeners().await;
            let (result, page) =
                tokio::join!(wait_for_callback(&listeners, None), browse(addr, target));

            assert!(matches!(
                result,
                Err(ProxyAuthK8sError::SsoLoginError(msg)) if msg.contains(detail)
            ));
            assert!(page.starts_with("HTTP/1.1 200 OK"));
            assert!(page.contains("Login failed"));
        }
    }

    #[tokio::test]
    async fn binding_fails_only_when_no_loopback_family_is_free() {
        // Hold the port on both families (IPv6 may be unavailable, which makes
        // its bind fail anyway).
        let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = v4.local_addr().unwrap().port();
        let _v6 = TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await;

        assert!(matches!(
            bind_loopback_listeners(port).await,
            Err(ProxyAuthK8sError::SsoLoginError(msg)) if msg.contains("PROXYAUTH_CALLBACK_PORT")
        ));
    }
}
