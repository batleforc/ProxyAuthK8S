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

use std::future::poll_fn;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::task::Poll;
use std::time::Duration;

use client_api::apis::{
    auth_clusters_api::{callback_login, cluster_login},
    configuration::Configuration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing::{debug, info, warn};

use crate::{ctx::CliCtx, error::ProxyAuthK8sError};

/// How long to wait for the browser round-trip before giving up.
const CALLBACK_TIMEOUT: Duration = Duration::from_mins(3);

/// Loopback port the local callback listener binds. Overridable because the
/// resulting redirect URI must be registered at the `IdP` for the cluster's OIDC
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

/// Minimal HTML shown in the browser when the provider redirected back with an
/// error. The details are surfaced on the terminal, not the page.
const ERROR_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>ProxyAuthK8S</title></head><body style=\"font-family:sans-serif\">\
<h2>Login failed</h2><p>The sign-in was not completed. Check your terminal for details.</p>\
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
        // The redirect URI is registered as `localhost`, which may resolve to
        // either `127.0.0.1` or `::1` depending on the host and on the opener the
        // provider hands the URL to. Listen on both loopback families so the
        // callback is caught whichever one the browser/opener picks.
        let listeners = bind_loopback_listeners(port).await?;
        let redirect = format!("http://localhost:{port}/");

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
        let (code, state) = wait_for_callback(&listeners).await?;

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

/// Bind a loopback listener on the IPv4 (`127.0.0.1`) and IPv6 (`::1`) loopbacks
/// for `port`. Both are attempted; only one needs to succeed (IPv6 may be
/// disabled, and vice-versa). Errors only when neither family can be bound.
async fn bind_loopback_listeners(port: u16) -> Result<Vec<TcpListener>, ProxyAuthK8sError> {
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
async fn wait_for_callback(
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

/// Outcome of parsing a request that hit the loopback listener.
enum CallbackResult {
    /// The OIDC provider redirected back with an authorization `code`+`state`.
    Success { code: String, state: String },
    /// The provider redirected back with an `error` (RFC 6749 §4.1.2.1).
    ProviderError {
        error: String,
        description: Option<String>,
    },
}

/// Classify a request target such as
/// `/auth/callback/ns/cluster?code=..&state=..` (success) or
/// `/auth/callback/ns/cluster?error=access_denied&error_description=..` (the
/// provider rejected the request). Returns `None` for anything that is neither
/// (favicon fetches, health probes, …) so the caller keeps waiting.
fn parse_callback(target: &str) -> Option<CallbackResult> {
    let url = reqwest::Url::parse(&format!("http://localhost{target}")).ok()?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut description = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "error_description" => description = Some(value.into_owned()),
            _ => {}
        }
    }
    if let Some(error) = error {
        return Some(CallbackResult::ProviderError { error, description });
    }
    Some(CallbackResult::Success {
        code: code?,
        state: state?,
    })
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

#[cfg(test)]
mod tests {
    use super::{CallbackResult, parse_callback};
    use crate::ctx::CliCtx;

    #[test]
    fn parses_code_and_state() {
        let target = "/auth/callback/default/local?code=abc&state=xyz";
        match parse_callback(target) {
            Some(CallbackResult::Success { code, state }) => {
                assert_eq!(code, "abc");
                assert_eq!(state, "xyz");
            }
            _ => panic!("expected a successful callback"),
        }
    }

    #[test]
    fn parses_provider_error() {
        let target =
            "/auth/callback/default/local?error=access_denied&error_description=user%20said%20no";
        match parse_callback(target) {
            Some(CallbackResult::ProviderError { error, description }) => {
                assert_eq!(error, "access_denied");
                assert_eq!(description.as_deref(), Some("user said no"));
            }
            _ => panic!("expected a provider error"),
        }
    }

    #[test]
    fn error_takes_precedence_over_partial_code() {
        // A stray `code` with an `error` must still be treated as a failure.
        let target = "/auth/callback/default/local?error=invalid_request&code=abc";
        assert!(matches!(
            parse_callback(target),
            Some(CallbackResult::ProviderError { .. })
        ));
    }

    #[test]
    fn ignores_non_callback_requests() {
        // Neither code+state nor error -> keep waiting (favicon, health probes).
        assert!(parse_callback("/favicon.ico").is_none());
        assert!(parse_callback("/auth/callback/default/local?state=only").is_none());
    }

    /// End-to-end for the extension's half of the login: a real HTTP server
    /// stands in for ProxyAuthK8S, and a background task stands in for the
    /// browser hitting the loopback listener the way the IdP's redirect
    /// would. Exercises the actual network code (`bind_loopback_listeners`,
    /// `wait_for_callback`, the `/auth/login` + `/auth/callback` exchange) —
    /// not just the pure `parse_callback` parser above.
    #[tokio::test]
    async fn sso_cluster_login_completes_the_full_browser_round_trip() {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpStream;
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // Dedicated port for this test so it never collides with a real login.
        let port: u16 = 18_123;
        // SAFETY: this test is the only place in the crate that reads or
        // writes this variable, so there is no cross-thread data race.
        unsafe {
            std::env::set_var("PROXYAUTH_CALLBACK_PORT", port.to_string());
        }

        let server = MockServer::start().await;
        let (ns, cluster) = ("default", "demo-cluster");

        // Step 1: the CLI asks the server for the provider's authorize URL,
        // authenticated with the already-stored server token — mirrors
        // `handle_login_clusters`'s `base_configuration()`.
        Mock::given(method("GET"))
            .and(path(format!("/clusters/{ns}/{cluster}/auth/login")))
            .and(header("authorization", "Bearer server-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(format!("{}/upstream-idp/authorize", server.uri()))
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&server)
            .await;

        // Step 3: the CLI exchanges the code/state it captured from the
        // simulated browser redirect for the cluster's id_token.
        Mock::given(method("GET"))
            .and(path(format!("/clusters/{ns}/{cluster}/auth/callback")))
            .and(query_param("code", "upstream-code"))
            .and(query_param("state", "upstream-state"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "upstream-access-token",
                "cluster_url": format!("{}/clusters/{ns}/{cluster}", server.uri()),
                "id_token": "signed-id-token",
                "refresh_token": "upstream-refresh-token",
                "subject": "alice-sub",
            })))
            .mount(&server)
            .await;

        let config = client_api::apis::configuration::Configuration {
            base_path: server.uri(),
            bearer_access_token: Some("server-token".to_string()),
            ..Default::default()
        };

        // Step 2: stand in for the browser being redirected back to the
        // loopback listener `sso_cluster_login` just bound. Retries because
        // the listener bind happens concurrently with this task starting.
        let browser = tokio::spawn(async move {
            let request = "GET /?code=upstream-code&state=upstream-state HTTP/1.1\r\n\
                 Host: localhost\r\nConnection: close\r\n\r\n";
            for _ in 0..100 {
                if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)).await {
                    stream
                        .write_all(request.as_bytes())
                        .await
                        .expect("write to the loopback listener should succeed");
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            panic!("could not connect to the loopback callback listener on 127.0.0.1:{port}");
        });

        let id_token = CliCtx::sso_cluster_login(&config, ns, cluster)
            .await
            .expect("the SSO round trip should succeed");

        browser
            .await
            .expect("the simulated browser task should not panic");
        assert_eq!(id_token, "signed-id-token");

        unsafe {
            std::env::remove_var("PROXYAUTH_CALLBACK_PORT");
        }
    }
}
