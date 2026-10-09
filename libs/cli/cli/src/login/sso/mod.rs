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

use tracing::{info, warn};

use client_api::apis::{
    auth_clusters_api::{callback_login, cluster_login},
    configuration::Configuration,
};

use crate::{
    ctx::CliCtx,
    error::ProxyAuthK8sError,
    login::sso::listener::{bind_loopback_listeners, wait_for_callback},
};

mod callback;
mod listener;

/// Loopback port the local callback listener binds. Overridable because the
/// resulting redirect URI must be registered at the `IdP` for the cluster's OIDC
/// client.
fn callback_port() -> u16 {
    std::env::var("PROXYAUTH_CALLBACK_PORT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(18_000)
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
    use crate::ctx::CliCtx;

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
