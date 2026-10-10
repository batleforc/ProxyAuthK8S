//! Turning a candidate collection into a resolved allow/deny outcome: the
//! privileged fetch of the candidates, the per-item review of whatever the
//! rules review left unresolved, and the cache/lock dance around both.

use actix_web::web;
use common::State;
use crd::ProxyKubeApi;
use crd::virtual_api::VirtualApiKind;
use futures_util::stream::{self, StreamExt};
use kube::ResourceExt;
use serde_json::Value;
use tracing::warn;

use crate::cluster::redirect::kube_redirect::{
    list_fallback::{
        ListFallbackArgs,
        cache::{
            acquire_outcome_lock, cache_key, load_cached_outcome, release_outcome_lock,
            store_cached_outcome,
        },
        configured_token,
        review::{RulesOutcome, check_access, resolve_via_rules_review},
        tuning::{cache_ttl_seconds, concurrency, upstream_timeout},
    },
    upstream::ca_only_client,
};

/// Fetch the full candidate collection with the privileged, least-privilege
/// token — `list` only, nothing else required on the target cluster.
///
/// `namespaces_path` is the upstream path for the collection (e.g.
/// `/api/v1/namespaces`), derived by the caller from the same mapper that
/// planned the request, so this module never has to know or duplicate that
/// mapping itself.
pub(super) async fn privileged_namespaces(
    proxy: &ProxyKubeApi,
    state: &web::Data<State>,
    base_url: &str,
    namespaces_path: &str,
    query_string: &str,
) -> Result<Value, String> {
    let token = configured_token(proxy, VirtualApiKind::OpenShiftProject)
        .ok_or_else(|| "no list_fallback_token configured".to_string())?;
    let namespace = proxy.namespace().unwrap_or_default();
    let token = token
        .get_cert(state.client.clone(), &namespace)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "list_fallback_token resolved to no value".to_string())?;

    let client = ca_only_client(proxy, state).await?;
    let url = if query_string.is_empty() {
        format!("{base_url}{namespaces_path}")
    } else {
        format!("{base_url}{namespaces_path}?{query_string}")
    };

    let res = client
        .get(&url)
        .timeout(upstream_timeout())
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| err.to_string())?;
    if !res.status().is_success() {
        return Err(format!(
            "privileged namespace list failed with status {}",
            res.status()
        ));
    }
    res.json::<Value>().await.map_err(|err| err.to_string())
}

/// Check whatever candidates in `items` `outcome` doesn't already cover
/// (allowed or denied), merge the results in, and — only when `outcome` was
/// not already trusted from the cache — store the updated outcome.
///
/// A cache hit is deliberately never re-stored just because it left a few
/// new candidates unresolved (e.g. a namespace created after the cache was
/// populated): doing so would slide the entry's TTL forward on every such
/// request, and under steady namespace churn the rules-review-derived part
/// of the outcome would then never actually re-expire — a revoked grant
/// would stop being "honoured soon after" as the module doc promises.
async fn resolve_unresolved(
    args: &ListFallbackArgs<'_>,
    mut outcome: RulesOutcome,
    items: &[Value],
    from_cache: bool,
) -> RulesOutcome {
    let unresolved: Vec<&Value> = if outcome.unrestricted {
        Vec::new()
    } else {
        items
            .iter()
            .filter(|item| {
                let name = item
                    .get("metadata")
                    .and_then(|metadata| metadata.get("name"))
                    .and_then(Value::as_str);
                !name.is_some_and(|name| {
                    outcome.names.contains(name) || outcome.denied.contains(name)
                })
            })
            .collect()
    };

    if !unresolved.is_empty() {
        let checked: Vec<(String, bool)> = stream::iter(unresolved)
            .map(|item| async move {
                let name = item
                    .get("metadata")
                    .and_then(|metadata| metadata.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string)?;
                let checked = check_access(
                    args.client,
                    args.req,
                    args.peer_addr,
                    args.user,
                    args.base_url,
                    args.probe,
                    &name,
                )
                .await;
                match checked {
                    Ok(allowed) => Some((name, allowed)),
                    Err(()) => {
                        warn!(namespace = %name, "could not confirm namespace visibility, excluding it");
                        None
                    }
                }
            })
            .buffer_unordered(concurrency())
            .filter_map(|entry| async move { entry })
            .collect()
            .await;
        for (name, allowed) in checked {
            if allowed {
                outcome.names.insert(name);
            } else {
                outcome.denied.insert(name);
            }
        }
    }

    if !from_cache {
        store_cached_outcome(args.state, args.proxy, args.user, args.probe, &outcome).await;
    }
    outcome
}

/// Whether `outcome` already covers every candidate in `items` — either
/// because it is `unrestricted`, or because every name is already recorded
/// as allowed or denied — with no per-item check left to run.
fn fully_resolved(outcome: &RulesOutcome, items: &[Value]) -> bool {
    outcome.unrestricted
        || items.iter().all(|item| {
            item.get("metadata")
                .and_then(|metadata| metadata.get("name"))
                .and_then(Value::as_str)
                .is_some_and(|name| outcome.names.contains(name) || outcome.denied.contains(name))
        })
}

/// A live `SelfSubjectRulesReview` for the caller, or an empty outcome
/// (everything left to the per-item checks) if it can't be used.
async fn rules_review(args: &ListFallbackArgs<'_>) -> RulesOutcome {
    resolve_via_rules_review(
        args.client,
        args.req,
        args.peer_addr,
        args.user,
        args.base_url,
        args.probe,
    )
    .await
    .unwrap_or_default()
}

/// Resolve the outcome for `items` — from `cached` when it already covers
/// every candidate, otherwise via a live rules review and/or per-item checks
/// — deduplicating that whole live-resolution path (rules review, per-item
/// checks, and the store) across concurrent requests for the same key within
/// this process: only the first one actually performs it, the rest wait on a
/// per-key lock and then find the cache populated. A `cached` outcome that
/// already covers everything is the one case with nothing to dedupe, so it
/// skips the lock entirely. Returns `(outcome, from_cache)`.
pub(super) async fn resolve_outcome_for(
    args: &ListFallbackArgs<'_>,
    cached: Option<RulesOutcome>,
    items: &[Value],
) -> (RulesOutcome, bool) {
    if let Some(outcome) = cached
        && fully_resolved(&outcome, items)
    {
        return (outcome, true);
    }

    // Caching disabled: nothing to dedupe, so skip the lock entirely rather
    // than serializing concurrent requests for no benefit.
    if cache_ttl_seconds() == 0 {
        let outcome = rules_review(args).await;
        let outcome = resolve_unresolved(args, outcome, items, false).await;
        return (outcome, false);
    }

    let key = cache_key(args.proxy, args.user, args.probe);
    let lock = acquire_outcome_lock(&key);
    let result = {
        let _guard = lock.lock().await;
        // Double-checked: another request for this key may have resolved and
        // stored a fully-checked outcome — including the per-item checks —
        // while we were waiting for the lock.
        match load_cached_outcome(args.state, args.proxy, args.user, args.probe).await {
            Some(outcome) => (resolve_unresolved(args, outcome, items, true).await, true),
            None => {
                let outcome = rules_review(args).await;
                (resolve_unresolved(args, outcome, items, false).await, false)
            }
        }
    };
    drop(lock);
    release_outcome_lock(&key);
    result
}
