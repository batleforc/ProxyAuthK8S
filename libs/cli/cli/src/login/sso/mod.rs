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
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    if let Err(e) = std::process::Command::new(program).args(args).spawn() {
        warn!("could not launch a browser automatically ({e}); open the URL above manually");
    }
}
