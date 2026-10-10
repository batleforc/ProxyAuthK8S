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
//!
//! Steps 1 and 4 are `listener.rs`; classifying what the provider sent back is
//! `callback.rs`. This module is the orchestration and the browser launch.

use std::io::IsTerminal;

use tokio::sync::mpsc;
use tracing::{info, warn};

use client_api::apis::{
    auth_clusters_api::{callback_login, cluster_login},
    configuration::Configuration,
};

use crate::{
    cli_config::browser::BrowserConfig,
    ctx::CliCtx,
    error::ProxyAuthK8sError,
    login::sso::listener::{bind_loopback_listeners, wait_for_callback},
};

mod callback;
mod listener;

/// Port the callback listener binds when `PROXYAUTH_CALLBACK_PORT` is unset.
const DEFAULT_CALLBACK_PORT: u16 = 18_000;

/// Loopback port the local callback listener binds, from the value of
/// `PROXYAUTH_CALLBACK_PORT`. Overridable because the resulting redirect URI
/// must be registered at the `IdP` for the cluster's OIDC client.
fn callback_port(value: Option<&str>) -> u16 {
    value
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(DEFAULT_CALLBACK_PORT)
}

impl CliCtx {
    /// Run the interactive OIDC login for `ns/cluster` and return the `id_token`.
    ///
    /// `config` must already carry the server bearer token (the `/auth/login`
    /// endpoint is authenticated).
    pub(crate) async fn sso_cluster_login(
        config: &Configuration,
        ns: &str,
        cluster: &str,
        browser: Option<&BrowserConfig>,
    ) -> Result<String, ProxyAuthK8sError> {
        let port = callback_port(std::env::var("PROXYAUTH_CALLBACK_PORT").ok().as_deref());
        // From a terminal, the redirect can also be pasted: needed whenever
        // the browser runs on another machine (Eclipse Che or any remote
        // workspace, SSH), where its redirect to `localhost` cannot reach us.
        let pasted = std::io::stdin().is_terminal().then(stdin_lines);
        Self::sso_cluster_login_with(
            config,
            ns,
            cluster,
            port,
            |url| {
                open_with(browser, url);
            },
            pasted,
        )
        .await
    }

    /// [`CliCtx::sso_cluster_login`] with the callback port and the browser
    /// launcher given, so tests neither touch the environment nor spawn a
    /// browser.
    async fn sso_cluster_login_with(
        config: &Configuration,
        ns: &str,
        cluster: &str,
        port: u16,
        open_browser: impl FnOnce(&str),
        pasted: Option<mpsc::Receiver<String>>,
    ) -> Result<String, ProxyAuthK8sError> {
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
        open_browser(&auth_url);

        // 3. Wait for the provider to redirect back to the loopback listener.
        if pasted.is_some() {
            info!(
                "If the browser runs on another machine (remote workspace, SSH), the page it \
                 lands on after you sign in will fail to load: copy that page's full URL from \
                 the address bar and paste it here."
            );
        }
        let (code, state) = wait_for_callback(&listeners, pasted).await?;

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

/// Each non-empty line typed on stdin, read on a plain thread: a blocked read
/// there never holds up the runtime's shutdown once the login is over (the
/// process exits with the thread still waiting).
fn stdin_lines() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(4);
    std::thread::spawn(move || {
        for line in std::io::stdin().lines() {
            let Ok(line) = line else { break };
            let line = line.trim().to_string();
            if !line.is_empty() && tx.blocking_send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// Open `url` with the configured browser, or the system's default one.
fn open_with(browser: Option<&BrowserConfig>, url: &str) {
    match browser {
        None => open_in_browser(url),
        Some(browser) if browser.opens_nothing() => {
            info!(
                "Not opening a browser (--browser none): open the URL above in a browser, \
                 on this machine or another one."
            );
        }
        Some(browser) => {
            // Run directly, never through a shell: the URL is one argument.
            if let Err(e) = std::process::Command::new(&browser.program)
                .args(browser.command_args(url))
                .spawn()
            {
                warn!(
                    "could not launch the browser '{}' ({e}); open the URL above manually, \
                     or change it with `login --browser`",
                    browser.program
                );
            }
        }
    }
}

/// Best-effort browser launch. The URL is always printed as well, so a failure
/// here (headless box, no opener) is not fatal.
fn open_in_browser(url: &str) {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        // Not `cmd /C start`: `cmd` reads the `&` separating the query
        // parameters of every authorization URL as a command separator, so the
        // browser got a truncated URL and the rest ran as shell commands.
        // `rundll32` hands the URL to the default handler without a shell.
        ("rundll32", vec!["url.dll,FileProtocolHandler", url])
    } else {
        ("xdg-open", vec![url])
    };
    if let Err(e) = std::process::Command::new(program).args(args).spawn() {
        warn!("could not launch a browser automatically ({e}); open the URL above manually");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const NS: &str = "default";
    const CLUSTER: &str = "demo-cluster";

    #[test]
    fn the_callback_port_defaults_and_ignores_garbage() {
        assert_eq!(callback_port(None), DEFAULT_CALLBACK_PORT);
        assert_eq!(callback_port(Some(" 18123 ")), 18_123);
        assert_eq!(callback_port(Some("not-a-port")), DEFAULT_CALLBACK_PORT);
        assert_eq!(callback_port(Some("70000")), DEFAULT_CALLBACK_PORT);
    }

    /// A loopback port free right now.
    fn free_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .and_then(|listener| listener.local_addr())
            .expect("free port")
            .port()
    }

    fn config(server: &MockServer) -> Configuration {
        Configuration {
            base_path: server.uri(),
            bearer_access_token: Some("server-token".to_string()),
            ..Default::default()
        }
    }

    /// Step 1: the server hands out the provider's authorize URL.
    async fn mount_login(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path(format!("/clusters/{NS}/{CLUSTER}/auth/login")))
            .and(header("authorization", "Bearer server-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(format!("{}/upstream-idp/authorize", server.uri()))
                    .insert_header("content-type", "text/plain"),
            )
            .mount(server)
            .await;
    }

    /// Step 3: the server exchanges the code; `expected` calls, `response`.
    async fn mount_callback(server: &MockServer, response: ResponseTemplate, expected: u64) {
        Mock::given(method("GET"))
            .and(path(format!("/clusters/{NS}/{CLUSTER}/auth/callback")))
            .and(query_param("code", "upstream-code"))
            .and(query_param("state", "upstream-state"))
            .respond_with(response)
            .expect(expected)
            .mount(server)
            .await;
    }

    fn callback_body(id_token: &str) -> serde_json::Value {
        serde_json::json!({
            "access_token": "upstream-access-token",
            "cluster_url": format!("https://proxy.example/clusters/{NS}/{CLUSTER}"),
            "id_token": id_token,
            "refresh_token": "upstream-refresh-token",
            "subject": "alice-sub",
        })
    }

    /// Stands in for the browser: records the URL it was asked to open, then
    /// follows the provider's redirect to `redirect_target` on the loopback
    /// listener. The listener is bound before the browser is opened, so a
    /// single connection is enough.
    fn fake_browser(
        port: u16,
        redirect_target: &'static str,
        opened: Arc<Mutex<Vec<String>>>,
    ) -> impl FnOnce(&str) {
        move |url: &str| {
            opened.lock().unwrap().push(url.to_string());
            tokio::spawn(async move {
                let mut stream = TcpStream::connect(("127.0.0.1", port))
                    .await
                    .expect("the callback listener is bound before the browser opens");
                let request = format!(
                    "GET {redirect_target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
                );
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut page = String::new();
                let _ = stream.read_to_string(&mut page).await;
            });
        }
    }

    async fn login(
        server: &MockServer,
        redirect_target: &'static str,
    ) -> (Result<String, ProxyAuthK8sError>, Vec<String>) {
        let port = free_port();
        let opened = Arc::new(Mutex::new(Vec::new()));
        let result = CliCtx::sso_cluster_login_with(
            &config(server),
            NS,
            CLUSTER,
            port,
            fake_browser(port, redirect_target, Arc::clone(&opened)),
            None,
        )
        .await;
        let opened = opened.lock().unwrap().clone();
        (result, opened)
    }

    const PROVIDER_REDIRECT: &str = "/?code=upstream-code&state=upstream-state";

    /// End to end for the CLI's half of the login: a real HTTP server stands
    /// in for ProxyAuthK8S and the fake browser for the provider's redirect,
    /// through the real loopback listener and the `/auth/login` +
    /// `/auth/callback` exchange.
    #[tokio::test]
    async fn the_full_browser_round_trip_returns_the_id_token() {
        let server = MockServer::start().await;
        mount_login(&server).await;
        mount_callback(
            &server,
            ResponseTemplate::new(200).set_body_json(callback_body("signed-id-token")),
            1,
        )
        .await;

        let (result, opened) = login(&server, PROVIDER_REDIRECT).await;
        assert_eq!(
            result.expect("the SSO round trip succeeds"),
            "signed-id-token"
        );
        assert_eq!(
            opened,
            vec![format!("{}/upstream-idp/authorize", server.uri())]
        );
    }

    #[tokio::test]
    async fn the_browser_is_not_opened_when_the_server_refuses_to_start() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/clusters/{NS}/{CLUSTER}/auth/login")))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let (result, opened) = login(&server, PROVIDER_REDIRECT).await;
        let err = result.expect_err("no authorize URL");
        assert!(
            err.to_string().contains("could not start the OIDC login"),
            "{err}"
        );
        assert!(opened.is_empty());
    }

    #[tokio::test]
    async fn a_provider_error_is_reported_without_exchanging_anything() {
        let server = MockServer::start().await;
        mount_login(&server).await;
        mount_callback(&server, ResponseTemplate::new(200), 0).await;

        let (result, _) = login(
            &server,
            "/?error=access_denied&error_description=user%20declined",
        )
        .await;
        let err = result.expect_err("the user declined");
        let message = err.to_string();
        assert!(message.contains("access_denied"), "{message}");
        assert!(message.contains("user declined"), "{message}");
    }

    #[tokio::test]
    async fn a_failed_code_exchange_is_reported() {
        let server = MockServer::start().await;
        mount_login(&server).await;
        mount_callback(&server, ResponseTemplate::new(502), 1).await;

        let (result, _) = login(&server, PROVIDER_REDIRECT).await;
        let err = result.expect_err("the exchange failed");
        assert!(err.to_string().contains("could not exchange"), "{err}");
    }

    #[tokio::test]
    async fn an_exchange_without_an_id_token_is_refused() {
        let server = MockServer::start().await;
        mount_login(&server).await;
        mount_callback(
            &server,
            ResponseTemplate::new(200).set_body_json(callback_body("")),
            1,
        )
        .await;

        let (result, _) = login(&server, PROVIDER_REDIRECT).await;
        let err = result.expect_err("no id_token");
        assert!(
            err.to_string().contains("did not return an ID token"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_busy_callback_port_fails_before_anything_is_requested() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        // Hold both loopback families so nothing can bind the port.
        let v4 = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = v4.local_addr().unwrap().port();
        let _v6 = std::net::TcpListener::bind(("::1", port));

        let opened = Arc::new(Mutex::new(Vec::new()));
        let err = CliCtx::sso_cluster_login_with(
            &config(&server),
            NS,
            CLUSTER,
            port,
            fake_browser(port, PROVIDER_REDIRECT, Arc::clone(&opened)),
            None,
        )
        .await
        .expect_err("the port is taken");
        assert!(err.to_string().contains("PROXYAUTH_CALLBACK_PORT"), "{err}");
        assert!(opened.lock().unwrap().is_empty());
    }

    // --- the configured browser ---------------------------------------------

    #[cfg(unix)]
    #[tokio::test]
    async fn a_configured_browser_gets_the_url_as_one_argument() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("opened");
        // Stands in for a corporate browser: records the argument it got.
        // `&` and `?` must arrive untouched, as a single argument.
        let browser = BrowserConfig {
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!("printf '%s' \"$1\" > '{}'", out.display()),
                "corp-browser".to_string(),
                "{url}".to_string(),
            ],
        };
        let url = "https://idp.example/authorize?client_id=a&state=b c";
        open_with(Some(&browser), url);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Ok(opened) = std::fs::read_to_string(&out)
                && !opened.is_empty()
            {
                assert_eq!(opened, url);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the browser was not launched"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    #[test]
    fn a_missing_browser_or_none_does_not_fail_the_login() {
        // The URL is always printed; a browser that cannot start only warns.
        open_with(
            Some(&BrowserConfig {
                program: "/nonexistent/corp-browser".to_string(),
                args: vec![],
            }),
            "https://idp.example/authorize",
        );
        open_with(
            Some(&BrowserConfig {
                program: "none".to_string(),
                args: vec![],
            }),
            "https://idp.example/authorize",
        );
    }

    // --- browser on another machine -------------------------------------------

    /// A browser on another machine: the URL is opened elsewhere, its redirect
    /// to `localhost` never reaches the listener, and the user pastes it.
    async fn login_by_pasting(
        server: &MockServer,
        lines: &[&str],
    ) -> Result<String, ProxyAuthK8sError> {
        let (tx, rx) = mpsc::channel(8);
        let lines: Vec<String> = lines.iter().map(ToString::to_string).collect();
        CliCtx::sso_cluster_login_with(
            &config(server),
            NS,
            CLUSTER,
            free_port(),
            move |_url| {
                tokio::spawn(async move {
                    for line in lines {
                        tx.send(line).await.unwrap();
                    }
                });
            },
            Some(rx),
        )
        .await
    }

    #[tokio::test]
    async fn a_pasted_redirect_completes_the_login() {
        let server = MockServer::start().await;
        mount_login(&server).await;
        mount_callback(
            &server,
            ResponseTemplate::new(200).set_body_json(callback_body("signed-id-token")),
            1,
        )
        .await;

        let id_token = login_by_pasting(
            &server,
            &[
                // A mistake first: the wait goes on.
                "the page did not load",
                "http://localhost:18000/?state=upstream-state&code=upstream-code",
            ],
        )
        .await
        .expect("the pasted redirect is exchanged");
        assert_eq!(id_token, "signed-id-token");
    }

    #[tokio::test]
    async fn a_pasted_provider_error_ends_the_login() {
        let server = MockServer::start().await;
        mount_login(&server).await;
        mount_callback(&server, ResponseTemplate::new(200), 0).await;

        let err = login_by_pasting(
            &server,
            &["http://localhost:18000/?error=access_denied&error_description=nope"],
        )
        .await
        .expect_err("the provider refused");
        assert!(err.to_string().contains("access_denied"), "{err}");
    }

    #[tokio::test]
    async fn the_loopback_still_works_when_pasting_is_possible() {
        let server = MockServer::start().await;
        mount_login(&server).await;
        mount_callback(
            &server,
            ResponseTemplate::new(200).set_body_json(callback_body("signed-id-token")),
            1,
        )
        .await;
        let port = free_port();
        // Nothing is ever pasted, and stdin closes (EOF) at once.
        let (tx, rx) = mpsc::channel::<String>(1);
        drop(tx);
        let id_token = CliCtx::sso_cluster_login_with(
            &config(&server),
            NS,
            CLUSTER,
            port,
            fake_browser(port, PROVIDER_REDIRECT, Arc::new(Mutex::new(Vec::new()))),
            Some(rx),
        )
        .await
        .expect("the local browser's redirect still completes the login");
        assert_eq!(id_token, "signed-id-token");
    }
}
