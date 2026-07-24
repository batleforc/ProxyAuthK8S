//! Interactive, browser-based SSO login for a cluster whose provider is OIDC.
//!
//! Flow (mirrors what the front does, but headless-friendly):
//! 1. bind a local loopback listener and advertise it as `x-kubectl-callback`;
//! 2. ask the server for the provider's authorization URL;
//! 3. open the user's browser there;
//! 4. capture the `code`/`state` the provider redirects back to the listener;
//! 5. exchange them via the server's callback endpoint and keep the `id_token`
//!    (the token a cluster login is expected to store — see the front's
//!    `ClusterCallbackView`).

use std::time::Duration;

use client_api::apis::{
    auth_clusters_api::{callback_login, cluster_login},
    configuration::Configuration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tracing::{debug, info, warn};

use crate::{ctx::CliCtx, error::ProxyAuthK8sError};

/// How long to wait for the browser round-trip before giving up.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);

/// Loopback port the local callback listener binds. Overridable because the
/// resulting redirect URI must be registered at the IdP for the cluster's OIDC
/// client.
fn callback_port() -> u16 {
    std::env::var("PROXYAUTH_CALLBACK_PORT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(18_000)
}

/// Minimal HTML shown in the browser once the callback has been received.
const SUCCESS_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>ProxyAuthK8S</title></head><body style=\"font-family:sans-serif\">\
<h2>Login complete</h2><p>You can close this tab and return to your terminal.</p>\
</body></html>";

impl CliCtx {
    /// Run the interactive OIDC login for `ns/cluster` and return the `id_token`.
    ///
    /// `config` must already carry the server bearer token (the `/auth/login`
    /// endpoint is authenticated).
    pub(crate) async fn sso_cluster_login(
        config: &Configuration,
        ns: &str,
        cluster: &str,
    ) -> Result<String, ProxyAuthK8sError> {
        let port = callback_port();
        let listener = TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| {
            ProxyAuthK8sError::SsoLoginError(format!(
                "could not bind the local callback listener on 127.0.0.1:{port} ({e}). \
                 Set PROXYAUTH_CALLBACK_PORT to a free port registered at your IdP."
            ))
        })?;
        let redirect = format!("http://localhost:{}/", port);

        // 1. Ask the server for the provider authorization URL.
        let auth_url = cluster_login(config, ns, cluster, None, Some(&redirect))
            .await
            .map_err(|e| {
                ProxyAuthK8sError::SsoLoginError(format!("could not start the OIDC login: {e:?}"))
            })?;

        // 2. Send the user to their provider.
        info!(
            "Opening your browser to sign in. If nothing happens, open this URL manually:\n{}",
            auth_url
        );
        open_in_browser(&auth_url);

        // 3. Wait for the provider to redirect back to the loopback listener.
        let (code, state) = wait_for_callback(&listener).await?;

        // 4. Exchange the code for tokens through the server callback.
        let callback = callback_login(config, ns, cluster, None, Some(&redirect), &code, &state)
            .await
            .map_err(|e| {
                ProxyAuthK8sError::SsoLoginError(format!(
                    "could not exchange the authorization code: {e:?}"
                ))
            })?;

        if callback.id_token.is_empty() {
            return Err(ProxyAuthK8sError::SsoLoginError(
                "the provider did not return an ID token".to_string(),
            ));
        }
        Ok(callback.id_token)
    }
}

/// Accept loopback connections until one carries an OIDC `code`+`state`, then
/// answer the browser with a small success page. Times out after
/// [`CALLBACK_TIMEOUT`] so the CLI never hangs forever.
async fn wait_for_callback(listener: &TcpListener) -> Result<(String, String), ProxyAuthK8sError> {
    let accept = async {
        loop {
            let (mut stream, _) = listener.accept().await.map_err(|e| {
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

            match parse_code_state(target) {
                Some((code, state)) => {
                    respond(&mut stream, "200 OK", SUCCESS_PAGE).await;
                    return Ok((code, state));
                }
                None => {
                    // Browsers also fetch /favicon.ico etc.; acknowledge and wait
                    // for the real redirect.
                    debug!(target, "ignoring non-callback request on the loopback listener");
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

/// Extract `code` and `state` from a request target such as
/// `//auth/callback/ns/cluster?code=..&state=..`. Returns `None` unless both are
/// present.
fn parse_code_state(target: &str) -> Option<(String, String)> {
    let url = reqwest::Url::parse(&format!("http://localhost{target}")).ok()?;
    let mut code = None;
    let mut state = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            _ => {}
        }
    }
    Some((code?, state?))
}

async fn respond(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
}

/// Best-effort browser launch. The URL is always printed as well, so a failure
/// here (headless box, no opener) is not fatal.
fn open_in_browser(url: &str) {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    if let Err(e) = std::process::Command::new(program).args(args).spawn() {
        warn!("could not launch a browser automatically ({e}); open the URL above manually");
    }
}
