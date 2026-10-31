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
/// Each real exchange is audited and counted; a cache hit is only counted.
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
        Err(error) => {
            warn!(%error, "token exchange failed");
            crate::metrics::record_token_exchange(error.as_str(), Some(seconds));
            audit.emit_token_exchange(error.as_str());
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

async fn exchange(
    data: &Arc<State>,
    proxy: &ProxyKubeApi,
    settings: &TokenExchangeSettings,
    token: &str,
    source_ttl: Option<Duration>,
) -> Result<(Arc<SecretString>, Source), ExchangeError> {
    let conf = match proxy.get_oidc_conf(data.clone(), false, None).await {
        Ok(Some(conf)) => conf,
        Ok(None) => {
            return Err(ExchangeError::Unavailable(
                "the cluster has no enabled oidc_provider".to_string(),
            ));
        }
        Err(e) => return Err(ExchangeError::Unavailable(e.to_string())),
    };
    let key = token_exchange::cache_key(
        token,
        proxy.metadata.uid.as_deref().unwrap_or_default(),
        &conf.issuer_url,
        &conf.client_id,
        settings,
    );
    // A cache hit never waits on the IdP's discovery (its outage included).
    if let Some(token) = token_exchange::cached(&key) {
        return Ok((token, Source::CacheHit));
    }
    let token_endpoint = conf
        .token_endpoint()
        .await
        .map_err(|e| ExchangeError::Unavailable(e.to_string()))?;
    let http = conf
        .reqwest_client()
        .map_err(|e| ExchangeError::Unavailable(e.to_string()))?;
    let client = ExchangeClient {
        token_endpoint: &token_endpoint,
        client_id: &conf.client_id,
        client_secret: conf.client_secret.as_deref(),
    };
    token_exchange::exchange_cached(&http, &client, settings, &key, token, source_ttl).await
}
