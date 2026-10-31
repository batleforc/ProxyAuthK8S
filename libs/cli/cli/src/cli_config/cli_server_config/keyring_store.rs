//! Bearer-token storage, backed by the OS keyring.
//!
//! Three kinds of entry are kept under the `proxyauthk8s` service: the server
//! token, keyed by the server URL; the SSO session that renews it (RFC 0010),
//! keyed by `<server url>::session`; and the per-cluster tokens, keyed by
//! `<server url>::<ns>/<cluster>`.

use client_api::models::SessionTokens;
use keyring_core::Entry;
use serde::{Deserialize, Serialize};
use tracing::{debug, error};

use crate::{
    cli_config::{cli_cluster_config::CliClusterConfig, cli_server_config::CliServerConfig},
    error::ProxyAuthK8sError,
    helper::unix_now,
    keystore,
};

/// Keyring service every server and cluster token is stored under.
const KEYRING_SERVICE: &str = "proxyauthk8s";

/// [`KEYRING_SERVICE`], unless `PROXYAUTH_KEYRING_SERVICE` names another one:
/// the entries are keyed by server URL, so two profiles of one OS user on the
/// same server (or the integration suite's personas) would otherwise
/// overwrite each other's tokens.
fn keyring_service(from_env: Option<String>) -> String {
    from_env
        .filter(|service| !service.is_empty())
        .unwrap_or_else(|| KEYRING_SERVICE.to_string())
}

/// What renews a server token obtained by the SSO login to the server (RFC
/// 0010 §6.5). No `Debug`: it holds a secret.
#[derive(Serialize, Deserialize)]
pub(crate) struct ServerSession {
    pub refresh_token: String,
    /// Unix seconds at which the access token expires, when the `IdP` said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

impl ServerSession {
    /// Whether the access token has less than `margin` seconds left at `now`.
    /// shortcut: an unknown expiry never renews; refresh on a 401 if an `IdP`
    /// ever omits `expires_in`.
    pub(crate) fn expires_within(&self, margin: i64, now: i64) -> bool {
        self.expires_at
            .is_some_and(|at| at.saturating_sub(now) < margin)
    }
}

impl CliServerConfig {
    pub fn server_token(&self) -> Result<String, ProxyAuthK8sError> {
        self.read_token(&self.url)
    }

    pub fn set_server_token(&self, token: String) -> Result<(), ProxyAuthK8sError> {
        self.write_token(&self.url, &token)
    }

    pub fn clear_server_token(&self) -> Result<(), ProxyAuthK8sError> {
        self.forget_server_session();
        self.delete_token(&self.url)
    }

    /// Keyring user of the server's SSO session. A sibling of the server token
    /// rather than one JSON with it: each stays under the 2560-byte blob limit
    /// of the Windows Credential Manager. No `/`, so no cluster key clashes.
    fn session_keyring_user(&self) -> String {
        format!("{}::session", self.url)
    }

    /// The stored SSO session, or `None` for a pasted token (no session).
    pub(crate) fn server_session(&self) -> Result<Option<ServerSession>, ProxyAuthK8sError> {
        let entry = self.keyring_entry(
            &self.session_keyring_user(),
            ProxyAuthK8sError::KeyringReadError,
        )?;
        let unreadable = |detail: String| {
            debug!("Keyring session read error: {}", detail);
            ProxyAuthK8sError::KeyringReadError(format!(
                "Failed to read the session from keyring for server URL: {}",
                self.url
            ))
        };
        match entry.get_password() {
            Ok(raw) => serde_json::from_str(&raw)
                .map(Some)
                .map_err(|e| unreadable(e.to_string())),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(unreadable(e.to_string())),
        }
    }

    /// Store a session the server handed out (login or refresh) and return
    /// its access token. `previous_refresh_token` is kept when the `IdP` did
    /// not rotate it; without any refresh token, no session is kept.
    pub(crate) fn store_session_tokens(
        &self,
        tokens: SessionTokens,
        previous_refresh_token: Option<String>,
    ) -> Result<String, ProxyAuthK8sError> {
        let session = tokens
            .refresh_token
            .flatten()
            .or(previous_refresh_token)
            .map(|refresh_token| ServerSession {
                refresh_token,
                expires_at: tokens
                    .expires_in
                    .flatten()
                    .map(|seconds| unix_now().saturating_add(seconds)),
            });
        self.set_server_login(&tokens.access_token, session.as_ref())?;
        Ok(tokens.access_token)
    }

    /// Store the server token and its session; `None` (a pasted token) drops
    /// any previous session, which would otherwise renew over the new token.
    pub(crate) fn set_server_login(
        &self,
        access_token: &str,
        session: Option<&ServerSession>,
    ) -> Result<(), ProxyAuthK8sError> {
        self.write_token(&self.url, access_token)?;
        let Some(session) = session else {
            self.forget_server_session();
            return Ok(());
        };
        let raw = serde_json::to_string(session).map_err(|e| {
            ProxyAuthK8sError::KeyringWriteError(format!(
                "Failed to encode the session for server URL {}: {e}",
                self.url
            ))
        })?;
        self.write_token(&self.session_keyring_user(), &raw)
    }

    /// Drop the SSO session, if any: there is often none (pasted token).
    fn forget_server_session(&self) {
        if let Err(e) = self.delete_token(&self.session_keyring_user()) {
            debug!("No server session dropped: {}", e);
        }
    }

    pub fn set_cluster_token(
        &mut self,
        ns: String,
        cluster: String,
        token: String,
    ) -> Result<(), ProxyAuthK8sError> {
        let key = format!("{ns}/{cluster}");
        // Replaced, not kept: a cluster leaving token exchange loses its flag.
        self.clusters.insert(key.clone(), CliClusterConfig::new());
        self.write_token(&self.cluster_keyring_user(&key), &token)
    }

    pub fn get_cluster_token(
        &self,
        ns: String,
        cluster: String,
    ) -> Result<String, ProxyAuthK8sError> {
        self.read_token(&self.cluster_keyring_user(&format!("{ns}/{cluster}")))
    }

    pub fn clear_cluster_token(
        &self,
        ns: String,
        cluster: String,
    ) -> Result<(), ProxyAuthK8sError> {
        self.delete_token(&self.cluster_keyring_user(&format!("{ns}/{cluster}")))
    }

    /// Keyring user under which the token of cluster `key` (`"<ns>/<cluster>"`)
    /// is stored. Changing this format would orphan every stored cluster token.
    fn cluster_keyring_user(&self, key: &str) -> String {
        format!("{}::{}", self.url, key)
    }

    /// Open the keyring entry for `user` under [`keyring_service`], mapping a
    /// failure to the caller's error variant (`err`).
    fn keyring_entry(
        &self,
        user: &str,
        err: fn(String) -> ProxyAuthK8sError,
    ) -> Result<Entry, ProxyAuthK8sError> {
        let service = keyring_service(std::env::var("PROXYAUTH_KEYRING_SERVICE").ok());
        keystore::entry(&service, user).map_err(|e| {
            debug!("Keyring entry creation error: {}", e);
            err(format!(
                "Failed to create keyring entry for server URL: {}",
                self.url
            ))
        })
    }

    fn read_token(&self, user: &str) -> Result<String, ProxyAuthK8sError> {
        let entry = self.keyring_entry(user, ProxyAuthK8sError::KeyringReadError)?;
        entry.get_password().map_err(|e| {
            debug!("Keyring read error: {}", e);
            ProxyAuthK8sError::KeyringReadError(format!(
                "Failed to read token from keyring for server URL: {}",
                self.url
            ))
        })
    }

    fn write_token(&self, user: &str, token: &str) -> Result<(), ProxyAuthK8sError> {
        let entry = self.keyring_entry(user, ProxyAuthK8sError::KeyringWriteError)?;
        entry.set_password(token).map_err(|e| {
            debug!("Keyring write error: {}", e);
            ProxyAuthK8sError::KeyringWriteError(format!(
                "Failed to write token to keyring for server URL: {}",
                self.url
            ))
        })
    }

    fn delete_token(&self, user: &str) -> Result<(), ProxyAuthK8sError> {
        let entry = self.keyring_entry(user, ProxyAuthK8sError::KeyringDeleteError)?;
        entry.delete_credential().map_err(|e| {
            debug!("Keyring delete error: {}", e);
            ProxyAuthK8sError::KeyringDeleteError(format!(
                "Failed to delete token from keyring for server URL: {}",
                self.url
            ))
        })
    }

    pub fn clear_all_tokens(&self) {
        for (cluster, config) in &self.clusters {
            if config.token_exchange {
                // Nothing stored for it (RFC 0010).
                continue;
            }
            let val: Vec<&str> = cluster.split('/').collect();
            let ns = val.first().unwrap_or(&"").to_string();
            let cluster = val.get(1).unwrap_or(&"").to_string();
            if let Err(err) = self.clear_cluster_token(ns, cluster.clone()) {
                error!("Error clearing token for cluster {}: {}", cluster, err);
            }
        }
        if let Err(err) = self.clear_server_token() {
            error!("Error clearing server token: {}", err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_keyring_user_keeps_the_stored_token_naming() {
        // Tokens already in users' keyrings are looked up under this exact
        // service/user pair; a format change would make them unreadable.
        assert_eq!(KEYRING_SERVICE, "proxyauthk8s");
        assert_eq!(keyring_service(None), "proxyauthk8s");
        assert_eq!(keyring_service(Some(String::new())), "proxyauthk8s");
        assert_eq!(keyring_service(Some("work".to_string())), "work");
        let config = CliServerConfig::new("https://localhost:5437".to_string());
        assert_eq!(
            config.cluster_keyring_user("team-a/prod"),
            "https://localhost:5437::team-a/prod"
        );
    }

    // The tests below run against the in-memory mock store `keystore` selects
    // under `cfg(test)`. Each uses its own server URL so they stay independent
    // when run in one process (`cargo test`).

    #[test]
    fn server_token_round_trips_through_the_keyring() {
        let config = CliServerConfig::new("https://tokens.example".to_string());
        assert!(matches!(
            config.server_token(),
            Err(ProxyAuthK8sError::KeyringReadError(_))
        ));

        config.set_server_token("tok".to_string()).unwrap();
        assert_eq!(config.server_token().unwrap(), "tok");
        // Stored under the documented service/user pair.
        let raw = keystore::entry(KEYRING_SERVICE, "https://tokens.example").unwrap();
        assert_eq!(raw.get_password().unwrap(), "tok");

        config.clear_server_token().unwrap();
        assert!(config.server_token().is_err());
        // Nothing left to delete.
        assert!(matches!(
            config.clear_server_token(),
            Err(ProxyAuthK8sError::KeyringDeleteError(_))
        ));
    }

    #[test]
    fn cluster_token_round_trips_and_marks_the_cluster_known() {
        let mut config = CliServerConfig::new("https://clusters.example".to_string());
        config
            .set_cluster_token("team-a".to_string(), "prod".to_string(), "ct".to_string())
            .unwrap();
        assert!(config.clusters["team-a/prod"].token_exist);
        assert_eq!(
            config
                .get_cluster_token("team-a".to_string(), "prod".to_string())
                .unwrap(),
            "ct"
        );
        let raw =
            keystore::entry(KEYRING_SERVICE, "https://clusters.example::team-a/prod").unwrap();
        assert_eq!(raw.get_password().unwrap(), "ct");

        config
            .clear_cluster_token("team-a".to_string(), "prod".to_string())
            .unwrap();
        assert!(matches!(
            config.get_cluster_token("team-a".to_string(), "prod".to_string()),
            Err(ProxyAuthK8sError::KeyringReadError(_))
        ));
    }

    #[test]
    fn keyring_failures_map_to_the_matching_error() {
        let config = CliServerConfig::new("https://failing.example".to_string());
        let raw = keystore::entry(KEYRING_SERVICE, "https://failing.example").unwrap();
        let mock: &keyring_core::mock::Cred = raw.as_any().downcast_ref().unwrap();
        mock.set_error(keyring_core::Error::NoStorageAccess(
            "locked".to_string().into(),
        ));
        assert!(matches!(
            config.set_server_token("tok".to_string()),
            Err(ProxyAuthK8sError::KeyringWriteError(_))
        ));
        // The injected error is one-shot: the next write goes through.
        config.set_server_token("tok".to_string()).unwrap();
    }

    #[test]
    fn clear_all_tokens_drops_the_server_and_every_cluster_token() {
        let mut config = CliServerConfig::new("https://all.example".to_string());
        config.set_server_token("st".to_string()).unwrap();
        for cluster in ["a", "b"] {
            config
                .set_cluster_token("ns".to_string(), cluster.to_string(), "ct".to_string())
                .unwrap();
        }
        // A cluster known in the config but without a stored token is only
        // logged, not fatal.
        config
            .clusters
            .insert("ns/no-token".to_string(), CliClusterConfig::new());

        config.clear_all_tokens();

        assert!(config.server_token().is_err());
        for cluster in ["a", "b"] {
            assert!(
                config
                    .get_cluster_token("ns".to_string(), cluster.to_string())
                    .is_err()
            );
        }
        // Clearing again (nothing left) does not panic.
        config.clear_all_tokens();
    }

    #[test]
    fn a_login_session_is_stored_next_to_the_server_token() {
        let config = CliServerConfig::new("https://session.example".to_string());
        let mut tokens = SessionTokens::new("access".to_string(), "Bearer".to_string());
        tokens.expires_in = Some(Some(300));
        tokens.refresh_token = Some(Some("refresh".to_string()));

        assert_eq!(config.store_session_tokens(tokens, None).unwrap(), "access");
        // `server_token()` still reads the access token alone.
        assert_eq!(config.server_token().unwrap(), "access");
        let session = config.server_session().unwrap().unwrap();
        assert_eq!(session.refresh_token, "refresh");
        assert!(!session.expires_within(60, unix_now()));
        assert!(session.expires_within(60, unix_now().saturating_add(250)));
        let raw = keystore::entry(KEYRING_SERVICE, "https://session.example::session").unwrap();
        assert!(
            raw.get_password()
                .unwrap()
                .contains("\"refresh_token\":\"refresh\"")
        );

        // A pasted token afterwards drops the session that would renew it.
        config.set_server_login("pasted", None).unwrap();
        assert_eq!(config.server_token().unwrap(), "pasted");
        assert!(config.server_session().unwrap().is_none());

        // A session without refresh token keeps none; logout drops both.
        let tokens = SessionTokens::new("bare".to_string(), "Bearer".to_string());
        config.store_session_tokens(tokens, None).unwrap();
        assert!(config.server_session().unwrap().is_none());
        config
            .set_server_login(
                "a",
                Some(&ServerSession {
                    refresh_token: "r".to_string(),
                    expires_at: None,
                }),
            )
            .unwrap();
        config.clear_server_token().unwrap();
        assert!(config.server_session().unwrap().is_none());
    }

    #[test]
    fn an_exchange_cluster_has_nothing_to_clear() {
        let mut config = CliServerConfig::new("https://exchange-clear.example".to_string());
        config.set_server_token("st".to_string()).unwrap();
        config
            .clusters
            .insert("ns/x".to_string(), CliClusterConfig::token_exchange());
        config.clear_all_tokens();
        assert!(config.server_token().is_err());

        // A token login replaces the exchange flag.
        config
            .set_cluster_token("ns".to_string(), "x".to_string(), "ct".to_string())
            .unwrap();
        assert!(!config.clusters["ns/x"].token_exchange);
    }
}
