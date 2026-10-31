//! `TokenExchange` on the proxy path (RFC 0010 §6.3): after the last gate,
//! the caller's proxy token is exchanged with the cluster's IdP, through the
//! per-replica cache of [`common::token_exchange`].

use std::{sync::Arc, time::Duration};

use axum::response::Response;
use common::State;
use common::token_exchange::{self, ExchangeClient, ExchangeError, Source};
use crd::ProxyKubeApi;
use crd::upstream_auth::TokenExchangeSettings;
use crd_runtime::ProxyKubeApiRuntime as _;
use secrecy::SecretString;
use tracing::warn;

use crate::cluster::redirect::audit::AuditContext;
use crate::cluster::redirect::status_response::{
    bad_gateway, unauthorized, upstream_mode_unavailable,
};

/// The exchanged token for `token` on `proxy`, or why there is none (answer
/// with [`refusal`]).
/// Each real exchange is audited and counted; a cache hit, or a failure before
/// any request to the IdP (its config, its discovery), is only counted.
/// No failure counts toward a ban: the token passed gate 4 (decision 7).
pub(crate) async fn exchanged_token(
    data: &Arc<State>,
    proxy: &ProxyKubeApi,
    settings: &TokenExchangeSettings,
    token: &str,
    source_ttl: Option<Duration>,
    audit: &AuditContext,
) -> Result<Arc<SecretString>, ExchangeError> {
    let started = std::time::Instant::now();
    let outcome = exchange(data, proxy, settings, token, source_ttl).await;
    let seconds = started.elapsed().as_secs_f64();
    match outcome {
        Ok((token, Source::CacheHit)) => {
            crate::metrics::record_token_exchange("cache_hit", None);
            Ok(token)
        }
        Ok((token, Source::Exchanged)) => {
            crate::metrics::record_token_exchange("exchanged", Some(seconds));
            audit.emit_token_exchange("exchanged");
            Ok(token)
        }
        Err(Failed { error, reached_idp }) => {
            warn!(%error, reached_idp, "token exchange failed");
            crate::metrics::record_token_exchange(error.as_str(), Some(seconds));
            if reached_idp {
                audit.emit_token_exchange(error.as_str());
            }
            Err(error)
        }
    }
}

/// `503` unavailable, `401` refused, `502` unreadable (decision 7); the
/// message names who can act and never carries a token.
pub(crate) fn refusal(error: &ExchangeError, cluster: &str) -> Response {
    match error {
        ExchangeError::Unavailable(_) => upstream_mode_unavailable(&format!(
            "the identity provider of cluster {cluster} is unavailable for the token exchange; retry later"
        )),
        ExchangeError::Refused(_) => unauthorized(&format!(
            "the identity provider of cluster {cluster} refused the token exchange; ask the cluster operator"
        )),
        ExchangeError::Unreadable(_) => bad_gateway(&format!(
            "the identity provider of cluster {cluster} answered the token exchange with something unreadable; ask the cluster operator"
        )),
    }
}

/// A failed exchange, and whether a request reached the IdP (only then is it
/// an exchange the audit records, §10.1).
struct Failed {
    error: ExchangeError,
    reached_idp: bool,
}

impl Failed {
    fn before_idp(error: impl std::fmt::Display) -> Self {
        Self {
            error: ExchangeError::Unavailable(error.to_string()),
            reached_idp: false,
        }
    }
}

async fn exchange(
    data: &Arc<State>,
    proxy: &ProxyKubeApi,
    settings: &TokenExchangeSettings,
    token: &str,
    source_ttl: Option<Duration>,
) -> Result<(Arc<SecretString>, Source), Failed> {
    let conf = match proxy.get_oidc_conf(data.clone(), false, None).await {
        Ok(Some(conf)) => conf,
        Ok(None) => {
            return Err(Failed::before_idp(
                "the cluster has no enabled oidc_provider",
            ));
        }
        Err(e) => return Err(Failed::before_idp(e)),
    };
    // The UID scopes the cache to this cluster (§5.2); the apiserver always
    // sets one, so its absence is a broken object, never a shared key.
    let Some(uid) = proxy.metadata.uid.as_deref() else {
        return Err(Failed::before_idp("the cluster has no UID"));
    };
    let key = token_exchange::cache_key(token, uid, &conf.issuer_url, &conf.client_id, settings);
    // A cache hit never waits on the IdP's discovery (its outage included).
    if let Some(token) = token_exchange::cached(&key) {
        return Ok((token, Source::CacheHit));
    }
    let token_endpoint = conf.token_endpoint().await.map_err(Failed::before_idp)?;
    let http = conf.reqwest_client().map_err(Failed::before_idp)?;
    let client = ExchangeClient {
        token_endpoint: &token_endpoint,
        client_id: &conf.client_id,
        client_secret: conf.client_secret.as_deref(),
    };
    token_exchange::exchange_cached(&http, &client, settings, &key, token, source_ttl)
        .await
        .map_err(|error| Failed {
            error,
            reached_idp: true,
        })
}
