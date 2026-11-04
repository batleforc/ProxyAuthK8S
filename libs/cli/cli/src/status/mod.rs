//! `status`: which servers you are logged in to, as whom, until when, and the
//! clusters logged in through each. Local state only: no network call, and no
//! token is ever printed.

use serde::{Deserialize, Serialize};

use crate::{
    cli_config::cli_server_config::CliServerConfig,
    ctx::CliCtx,
    error::ProxyAuthK8sError,
    helper::{display_claims, unix_now},
    output::{KubeList, TableRow},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StatusOutput {
    pub default: bool,
    pub server: String,
    /// `sso` (renewable session), `token` (pasted, not renewed) or `none`.
    pub login: String,
    pub user: Option<String>,
    /// Unix seconds at which the server token expires, when known.
    pub expires_at: Option<i64>,
    /// `namespace/name` of each cluster logged in through this server.
    pub clusters: Vec<String>,
}

/// Time left before `expires_at`, as a person reads it.
fn remaining(expires_at: Option<i64>, now: i64, renewable: bool) -> String {
    let Some(at) = expires_at else {
        return "unknown".to_string();
    };
    let left = at.saturating_sub(now);
    if left <= 0 {
        return if renewable {
            "expired (renewed on next use)".to_string()
        } else {
            "expired".to_string()
        };
    }
    match left {
        ..60 => "in <1m".to_string(),
        60..3_600 => format!("in {}m", left / 60),
        3_600..86_400 => format!("in {}h", left / 3600),
        _ => format!("in {}d", left / 86_400),
    }
}

impl StatusOutput {
    fn new(server: &CliServerConfig, default: bool) -> Self {
        let token = server.server_token().ok();
        let session = server.server_session().ok().flatten();
        let claims = token.as_deref().map(display_claims).unwrap_or_default();
        let login = match (&token, &session) {
            (None, _) => "none",
            (Some(_), Some(_)) => "sso",
            (Some(_), None) => "token",
        };
        let mut clusters: Vec<String> = server
            .clusters
            .iter()
            .map(|(key, cluster)| {
                if cluster.token_exchange {
                    format!("{key} (exchange)")
                } else {
                    key.clone()
                }
            })
            .collect();
        clusters.sort();
        StatusOutput {
            default,
            server: server.url.clone(),
            login: login.to_string(),
            user: claims.preferred_username.or(claims.email).or(claims.sub),
            expires_at: session.and_then(|s| s.expires_at).or(claims.exp),
            clusters,
        }
    }
}

impl TableRow for StatusOutput {
    fn headers() -> Vec<String> {
        ["DEFAULT", "SERVER", "LOGIN", "USER", "EXPIRES", "CLUSTERS"]
            .map(String::from)
            .to_vec()
    }

    fn row(&self) -> Vec<String> {
        let expires = if self.login == "none" {
            "-".to_string()
        } else {
            remaining(self.expires_at, unix_now(), self.login == "sso")
        };
        vec![
            if self.default { "*" } else { "" }.to_string(),
            self.server.clone(),
            self.login.clone(),
            self.user.clone().unwrap_or_else(|| "-".to_string()),
            expires,
            if self.clusters.is_empty() {
                "-".to_string()
            } else {
                self.clusters.join(", ")
            },
        ]
    }
}

impl CliCtx {
    /// Print the login state of every known server (or of `--server-url`).
    pub fn handle_status(&self) -> Result<(), ProxyAuthK8sError> {
        let typed = (!self.server_url.is_empty()).then_some(&self.server_url);
        let only = typed.map(|url| CliServerConfig::url_to_name_from_string(url.clone()));
        let mut rows: Vec<StatusOutput> = self
            .config
            .servers
            .iter()
            .filter(|(name, _)| only.as_ref().is_none_or(|only| *name == only))
            .map(|(name, server)| {
                StatusOutput::new(server, *name == self.config.default_server_name)
            })
            .collect();
        if rows.is_empty() {
            return Err(match typed {
                Some(url) => ProxyAuthK8sError::ServerNotFound(url.clone()),
                None => ProxyAuthK8sError::NotLoggedIn,
            });
        }
        rows.sort_by(|a, b| b.default.cmp(&a.default).then(a.server.cmp(&b.server)));
        println!("{}", KubeList::new(rows).to_output(self.format.clone()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_config::cli_cluster_config::CliClusterConfig;
    use crate::test_support::jwt;

    #[test]
    fn claims_are_read_from_a_jwt_and_absent_from_an_opaque_token() {
        let claims = display_claims(&jwt(&serde_json::json!({
            "sub": "u-1", "email": "a@example.com", "exp": 42
        })));
        assert_eq!(claims.email.as_deref(), Some("a@example.com"));
        assert_eq!(claims.exp, Some(42));
        assert!(claims.preferred_username.is_none());

        for opaque in ["opaque-token", "a.!!!.c", "", "a.e30"] {
            let claims = display_claims(opaque);
            assert!(claims.sub.is_none() && claims.exp.is_none(), "{opaque}");
        }
    }

    #[test]
    fn time_left_reads_like_a_person_would_say_it() {
        assert_eq!(remaining(None, 0, false), "unknown");
        assert_eq!(remaining(Some(100), 100, false), "expired");
        assert_eq!(
            remaining(Some(50), 100, true),
            "expired (renewed on next use)"
        );
        assert_eq!(remaining(Some(30), 0, false), "in <1m");
        assert_eq!(remaining(Some(1_800), 0, false), "in 30m");
        assert_eq!(remaining(Some(7_200), 0, false), "in 2h");
        assert_eq!(remaining(Some(3 * 86_400), 0, false), "in 3d");
    }

    #[test]
    fn status_can_be_narrowed_to_one_known_server() {
        let mut ctx = CliCtx::for_test();
        for url in ["https://status-one.example", "https://status-two.example"] {
            ctx.config.get_or_insert_server_config(
                CliServerConfig::url_to_name_from_string(url.to_string()),
                url.to_string(),
            );
        }
        ctx.server_url = "https://status-two.example".to_string();
        ctx.handle_status().unwrap();
    }

    #[test]
    fn status_without_a_server_is_refused() {
        let mut ctx = CliCtx::for_test();
        assert!(matches!(
            ctx.handle_status(),
            Err(ProxyAuthK8sError::NotLoggedIn)
        ));
        ctx.config.get_or_insert_server_config(
            "status-known".to_string(),
            "https://status-known".to_string(),
        );
        ctx.server_url = "https://status-other".to_string();
        assert!(matches!(
            ctx.handle_status(),
            Err(ProxyAuthK8sError::ServerNotFound(url)) if url == "https://status-other"
        ));
    }

    #[test]
    fn a_server_row_shows_the_user_and_clusters_never_the_token() {
        let url = "https://status-row.example".to_string();
        let mut server = CliServerConfig::new(url.clone());
        let token =
            jwt(&serde_json::json!({"preferred_username": "alice", "exp": 4_000_000_000_i64}));
        server.set_server_token(token.clone()).unwrap();
        server
            .clusters
            .insert("team-a/prod".to_string(), CliClusterConfig::new());
        server.clusters.insert(
            "team-a/exch".to_string(),
            CliClusterConfig::token_exchange(),
        );

        let row = StatusOutput::new(&server, true);
        assert_eq!(row.login, "token");
        assert_eq!(row.user.as_deref(), Some("alice"));
        assert_eq!(row.expires_at, Some(4_000_000_000));
        assert_eq!(row.clusters, vec!["team-a/exch (exchange)", "team-a/prod"]);
        let rendered = KubeList::new(vec![row]).to_output(crate::ctx::ContextFormat::Json);
        assert!(!rendered.contains(&token));

        server.clear_server_token().unwrap();
        let row = StatusOutput::new(&server, false);
        assert_eq!(row.login, "none");
        assert_eq!(row.row()[4], "-");
    }

    #[test]
    fn an_sso_session_shows_as_sso_with_the_idp_expiry() {
        use client_api::models::SessionTokens;
        let server = CliServerConfig::new("https://status-sso.example".to_string());
        // An opaque access token: the expiry comes from the IdP's expires_in,
        // not from the token.
        let mut tokens = SessionTokens::new("opaque-access".to_string(), "Bearer".to_string());
        tokens.refresh_token = Some(Some("refresh".to_string()));
        tokens.expires_in = Some(Some(1_800));
        server.store_session_tokens(tokens, None).unwrap();

        let row = StatusOutput::new(&server, true);
        assert_eq!(row.login, "sso");
        assert!(row.user.is_none());
        let left = row.expires_at.unwrap() - unix_now();
        assert!((1_790..=1_800).contains(&left), "{left}");
        // 1800 s left in the same second, 1799 s one tick later.
        let expires = &row.row()[4];
        assert!(expires == "in 30m" || expires == "in 29m", "{expires}");
        // Expired: a session renews on next use, a pasted token does not.
        assert_eq!(
            remaining(Some(0), 1, row.login == "sso"),
            "expired (renewed on next use)"
        );
    }
}
