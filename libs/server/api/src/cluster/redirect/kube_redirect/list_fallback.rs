//! Per-namespace visibility filtering for a virtual API's `LIST`.
//!
//! A mapper can offer a [`virtual_api::AccessProbe`] for a route (today, only
//! `OpenShiftProject`'s `LIST projects`). When the cluster also configures a
//! `list_fallback_token` for that virtual API, this module enumerates the
//! candidate items with that least-privilege credential and keeps only the
//! ones the impersonated caller can individually access — unconditionally,
//! whether or not the caller could also have listed the collection directly.
//! See `project.rs` for why this can't be done with `SelfSubjectAccessReview`
//! alone, and why the token exists at all.
//!
//! A cluster with many namespaces makes this expensive if every `LIST` runs
//! one `SelfSubjectAccessReview` per candidate, so several optimizations sit
//! in front of that per-item loop:
//!
//! - a single `SelfSubjectRulesReview` resolves most or all candidates in one
//!   call (see [`interpret_rules_review`] for why this is trustworthy here
//!   specifically, unlike the general case);
//! - the resolved outcome — both what is allowed and what is explicitly
//!   denied — is cached per `(cluster, caller identity, probe)` for a short
//!   TTL, so repeated polling (dashboards, `oc projects`) does not re-run the
//!   rules review, and does not re-run a per-item access review for a name
//!   already resolved either way;
//! - concurrent requests that all observe a cache miss for the same key are
//!   deduplicated behind a per-key lock, so only one of them pays for the
//!   rules review — the rest find the cache populated once unblocked;
//! - fetching the candidate collection and resolving the cached/rules-review
//!   outcome are independent upstream calls, so they run concurrently rather
//!   than one after the other.
//!
//! Per-item `SelfSubjectAccessReview` is still the fallback whenever the
//! rules review can't resolve a candidate — it is the only source ever
//! trusted for the actual allow/deny decision when there is any doubt.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use actix_web::{HttpRequest, dev::PeerAddr, http, web};
use common::State;
use crd::ProxyKubeApi;
use crd::certificate::CertSource;
use crd::virtual_api::VirtualApiKind;
use futures_util::stream::{self, StreamExt};
use kube::ResourceExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;
use tracing::{debug, warn};

use super::upstream::{apply_forward_headers, ca_only_client};
use crate::model::user::User;

/// Upper bound on concurrent `SelfSubjectAccessReview` calls for one LIST.
const DEFAULT_CONCURRENCY: usize = 16;

/// Environment variables are process-global and never change after startup,
/// so each of these knobs is parsed once and cached rather than re-read from
/// `std::env` on every call in what can be a per-request hot path.
fn concurrency() -> usize {
    static CONCURRENCY: OnceLock<usize> = OnceLock::new();
    *CONCURRENCY.get_or_init(|| {
        std::env::var("PROXY_VIRTUAL_LIST_FALLBACK_CONCURRENCY")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_CONCURRENCY)
    })
}

/// How long a resolved allow-set is trusted before being recomputed.
///
/// Short enough that a revoked grant stops being honoured soon after; long
/// enough that a polling dashboard does not re-run a rules review (or worse,
/// per-item access reviews) on every refresh. `0` disables caching outright.
const DEFAULT_CACHE_TTL_SECONDS: u64 = 30;

fn cache_ttl_seconds() -> u64 {
    static TTL: OnceLock<u64> = OnceLock::new();
    *TTL.get_or_init(|| {
        std::env::var("PROXY_VIRTUAL_LIST_FALLBACK_CACHE_TTL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_CACHE_TTL_SECONDS)
    })
}

/// Upper bound on a single upstream call this module makes directly
/// (`SelfSubjectRulesReview`, `SelfSubjectAccessReview`, or the privileged
/// namespace list). Applied per request via `RequestBuilder::timeout`, not on
/// the shared client — the shared client sets none, precisely so a long-lived
/// watch (built and used elsewhere, never through this module) is unaffected.
/// A cache-miss resolution here runs behind a per-key lock (see
/// [`resolve_outcome_for`]), so without this a stalled upstream call would
/// queue every concurrent caller sharing that key indefinitely instead of
/// just itself.
const DEFAULT_UPSTREAM_TIMEOUT_SECONDS: u64 = 30;

fn upstream_timeout() -> std::time::Duration {
    static TIMEOUT: OnceLock<std::time::Duration> = OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        let seconds = std::env::var("PROXY_VIRTUAL_LIST_FALLBACK_TIMEOUT_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_UPSTREAM_TIMEOUT_SECONDS);
        std::time::Duration::from_secs(seconds)
    })
}

/// Why the fallback could not produce a filtered list at all.
///
/// Distinct from a single namespace failing its access check (which just
/// drops that one item, see [`list_projects_filtered`]): this is a failure of
/// the discovery step itself, and the caller must not fall back to an
/// unfiltered list (over-exposure) or a silent empty one (misleading) — it
/// surfaces as a `503`.
#[derive(Debug)]
pub(super) struct DiscoveryError(pub(super) String);

/// The `list_fallback_token` configured for `kind` on this proxy, if any.
fn configured_token(proxy: &ProxyKubeApi, kind: VirtualApiKind) -> Option<&CertSource> {
    proxy
        .spec
        .virtual_apis
        .iter()
        .find(|configuration| configuration.kind == kind && configuration.enabled)
        .and_then(|configuration| configuration.list_fallback_token.as_ref())
}

/// Whether `proxy` has opted into filtered `LIST projects`.
pub(super) fn is_configured(proxy: &ProxyKubeApi) -> bool {
    configured_token(proxy, VirtualApiKind::OpenShiftProject).is_some()
}

/// Fetch the full candidate collection with the privileged, least-privilege
/// token — `list` only, nothing else required on the target cluster.
///
/// `namespaces_path` is the upstream path for the collection (e.g.
/// `/api/v1/namespaces`), derived by the caller from the same mapper that
/// planned the request, so this module never has to know or duplicate that
/// mapping itself.
async fn privileged_namespaces(
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

/// `POST .../selfsubjectaccessreviews` as the impersonated caller.
///
/// `Ok(false)` is an explicit denial: the caller records it into
/// `RulesOutcome::denied` and caches it exactly like an allow. `Err(())` on
/// any transport/parse failure means "cannot confirm" instead — the caller
/// excludes the item for this request but never caches the result, so it is
/// re-checked on every subsequent request until it actually resolves either
/// way. Only the latter is worth a warning; a plain denial is expected,
/// ordinary filtering.
async fn check_access(
    client: &reqwest::Client,
    req: &HttpRequest,
    peer_addr: Option<PeerAddr>,
    user: Option<&User>,
    base_url: &str,
    probe: virtual_api::AccessProbe,
    name: &str,
) -> Result<bool, ()> {
    let url = format!("{base_url}/apis/authorization.k8s.io/v1/selfsubjectaccessreviews");
    let body = json!({
        "kind": "SelfSubjectAccessReview",
        "apiVersion": "authorization.k8s.io/v1",
        "spec": {
            "resourceAttributes": {
                "group": probe.group,
                "resource": probe.resource,
                "verb": probe.verb,
                "name": name,
            }
        }
    });

    let mut builder = client
        .request(reqwest::Method::POST, &url)
        .timeout(upstream_timeout());
    builder = apply_forward_headers(builder, req, peer_addr, user);
    builder = builder
        .header(http::header::CONTENT_TYPE.as_str(), "application/json")
        .body(body.to_string());

    let res = builder.send().await.map_err(|_| ())?;
    if !res.status().is_success() {
        return Err(());
    }
    let json: Value = res.json().await.map_err(|_| ())?;
    json["status"]["allowed"].as_bool().ok_or(())
}

/// What a `SelfSubjectRulesReview` resolved about `probe`.
///
/// `unrestricted` means some matching rule carries no `resourceNames` — every
/// candidate is allowed, present or future, so it is never safe to cache as a
/// finite name set (a namespace created after the cache was populated must
/// still be included). `names` holds every `resourceNames` entry from every
/// matching rule, unioned. `denied` is never populated by a rules review — it
/// only ever fills up from a real per-item `SelfSubjectAccessReview`, so a
/// cache hit does not have to re-review a name already known to be denied.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct RulesOutcome {
    unrestricted: bool,
    names: HashSet<String>,
    #[serde(default)]
    denied: HashSet<String>,
}

/// Interpret a `SelfSubjectRulesReview` response body against `probe`. Pure,
/// so its rule-matching logic is unit-testable without a live cluster.
///
/// `None` when the review reports `incomplete` (aggregated `ClusterRole`s the
/// apiserver could not fully expand) or is missing its `status` — there is
/// nothing safe to conclude, and the caller must fall back to a real
/// `SelfSubjectAccessReview` for every candidate.
///
/// Trusting a resolved, non-incomplete result for an allow decision — which
/// the Kubernetes API docs otherwise warn against for authorization purposes
/// — is safe specifically for the `("", "namespaces", "get")` probe this
/// module uses `AccessProbe` for, and specifically because the review is
/// always issued with an empty `namespace` (see [`resolve_via_rules_review`]):
/// `Namespace` is a cluster-scoped resource, so only `ClusterRole`s bound via
/// `ClusterRoleBinding` can ever grant `get` on one — a namespaced
/// `RoleBinding` cannot, regardless of `resourceNames`, since its grant only
/// applies within its own namespace, which a cluster-scoped request has none
/// of. But `resourceRules` reports namespace-scoped `RoleBinding` rules too
/// whenever the review's `namespace` argument is non-empty, and nothing in
/// the response distinguishes those from `ClusterRoleBinding`-derived ones —
/// so an empty `namespace` is not a cosmetic choice, it is what keeps this
/// result trustworthy for a cluster-scoped decision. This still never denies
/// on rules-review data alone — an unresolved candidate always falls through
/// to a real per-item review (see [`list_projects_filtered`]) — it only ever
/// shortcuts an *allow*.
fn interpret_rules_review(body: &Value, probe: virtual_api::AccessProbe) -> Option<RulesOutcome> {
    let status = body.get("status")?;
    if status
        .get("incomplete")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let rules = status.get("resourceRules").and_then(Value::as_array)?;

    let mut outcome = RulesOutcome::default();
    for rule in rules {
        let matches = |field: &str, wanted: &str| {
            rule.get(field)
                .and_then(Value::as_array)
                .is_some_and(|values| {
                    values
                        .iter()
                        .any(|value| value.as_str() == Some(wanted) || value.as_str() == Some("*"))
                })
        };
        if !matches("apiGroups", probe.group) || !matches("resources", probe.resource) {
            continue;
        }
        // `verbs` is required by the API (never absent), unlike the two
        // above which may legitimately be omitted on some rule shapes.
        let verb_matches = rule
            .get("verbs")
            .and_then(Value::as_array)
            .is_some_and(|verbs| {
                verbs
                    .iter()
                    .any(|v| v.as_str() == Some(probe.verb) || v.as_str() == Some("*"))
            });
        if !verb_matches {
            continue;
        }

        match rule.get("resourceNames").and_then(Value::as_array) {
            None => outcome.unrestricted = true,
            Some(names) if names.is_empty() => outcome.unrestricted = true,
            Some(names) => outcome
                .names
                .extend(names.iter().filter_map(Value::as_str).map(str::to_string)),
        }
    }
    Some(outcome)
}

/// `POST .../selfsubjectrulesreviews` as the impersonated caller, then
/// interpret it. Transport/parse failure is treated exactly like
/// `incomplete`: `None`, safe to fall back on.
async fn resolve_via_rules_review(
    client: &reqwest::Client,
    req: &HttpRequest,
    peer_addr: Option<PeerAddr>,
    user: Option<&User>,
    base_url: &str,
    probe: virtual_api::AccessProbe,
) -> Option<RulesOutcome> {
    let url = format!("{base_url}/apis/authorization.k8s.io/v1/selfsubjectrulesreviews");
    // An empty namespace means no namespace-scoped RoleBindings are ever
    // consulted by the apiserver's rule resolver; ClusterRoleBindings — the
    // only way to grant rights on the cluster-scoped `namespaces` resource
    // this module probes for — are reported in `resourceRules` regardless of
    // the namespace argument. A non-empty namespace would instead also pull
    // that namespace's RoleBinding-derived rules into `resourceRules`, which
    // `interpret_rules_review` cannot distinguish from real ClusterRoleBinding
    // grants — silently trusting a namespace-scoped rule as if it granted a
    // cluster-scoped `get` on `namespaces`.
    let body = json!({
        "kind": "SelfSubjectRulesReview",
        "apiVersion": "authorization.k8s.io/v1",
        "spec": { "namespace": "" }
    });

    let mut builder = client
        .request(reqwest::Method::POST, &url)
        .timeout(upstream_timeout());
    builder = apply_forward_headers(builder, req, peer_addr, user);
    builder = builder
        .header(http::header::CONTENT_TYPE.as_str(), "application/json")
        .body(body.to_string());

    let res = builder.send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    let json: Value = res.json().await.ok()?;
    interpret_rules_review(&json, probe)
}

/// Deterministic, fixed-length identity for a cache key: the raw username and
/// groups could contain anything a client sends, so hash rather than embed
/// them directly.
fn identity_fingerprint(user: Option<&User>) -> String {
    let mut hasher = Sha256::new();
    match user {
        Some(user) => {
            hasher.update(user.username.as_bytes());
            let mut groups: Vec<&str> = user.groups.iter().map(String::as_str).collect();
            groups.sort_unstable();
            for group in groups {
                hasher.update(b"\0");
                hasher.update(group.as_bytes());
            }
        }
        None => hasher.update(b"anonymous"),
    }
    format!("{:x}", hasher.finalize())
}

/// A cache/lock key scoped to `(proxy, caller identity, probe)` — including
/// the probe matters as soon as more than one mapper shares this module's
/// caching path, so two unrelated (resource, verb) checks for the same caller
/// never collide on the same entry.
fn cache_key(proxy: &ProxyKubeApi, user: Option<&User>, probe: virtual_api::AccessProbe) -> String {
    format!(
        "proxyk8sauth:listfallback:{}:{}:{}:{}:{}",
        proxy.to_path(),
        identity_fingerprint(user),
        probe.group,
        probe.resource,
        probe.verb,
    )
}

async fn load_cached_outcome(
    state: &web::Data<State>,
    proxy: &ProxyKubeApi,
    user: Option<&User>,
    probe: virtual_api::AccessProbe,
) -> Option<RulesOutcome> {
    if cache_ttl_seconds() == 0 {
        return None;
    }
    let raw = state
        .redis_get(&cache_key(proxy, user, probe))
        .await
        .ok()??;
    serde_json::from_str(&raw).ok()
}

async fn store_cached_outcome(
    state: &web::Data<State>,
    proxy: &ProxyKubeApi,
    user: Option<&User>,
    probe: virtual_api::AccessProbe,
    outcome: &RulesOutcome,
) {
    let ttl = cache_ttl_seconds();
    if ttl == 0 {
        return;
    }
    let Ok(raw) = serde_json::to_string(outcome) else {
        return;
    };
    if let Err(err) = state
        .redis_set(&cache_key(proxy, user, probe), &raw, Some(ttl))
        .await
    {
        warn!(%err, "could not cache the resolved namespace allow-set");
    }
}

/// Per-key async locks deduplicating concurrent full-resolution work (rules
/// review, per-item checks, and the store) for the same cache key. An entry
/// is removed as soon as nothing else references it (see
/// [`release_outcome_lock`]), so this does not grow without bound over the
/// process lifetime.
///
/// This only dedupes within one process: the cache itself is shared (Redis)
/// across replicas, but this lock is not, so two different pods can still
/// each resolve the same cold key concurrently. Deduplicating across pods
/// too would need a distributed lock instead of this in-memory one.
static OUTCOME_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();

fn acquire_outcome_lock(key: &str) -> Arc<AsyncMutex<()>> {
    let registry = OUTCOME_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut map = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    map.entry(key.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

/// Drop `key`'s entry once nothing else still holds a clone of it. Runs under
/// the same registry mutex as [`acquire_outcome_lock`], so a concurrent
/// caller can never observe (or race against) a torn removal.
fn release_outcome_lock(key: &str) {
    let registry = OUTCOME_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut map = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = map.get(key)
        && Arc::strong_count(entry) == 1
    {
        map.remove(key);
    }
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
#[allow(clippy::too_many_arguments)]
async fn resolve_unresolved(
    state: &web::Data<State>,
    proxy: &ProxyKubeApi,
    client: &reqwest::Client,
    req: &HttpRequest,
    peer_addr: Option<PeerAddr>,
    user: Option<&User>,
    base_url: &str,
    probe: virtual_api::AccessProbe,
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
                match check_access(client, req, peer_addr, user, base_url, probe, &name).await {
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
        store_cached_outcome(state, proxy, user, probe, &outcome).await;
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

/// Resolve the outcome for `items` — from `cached` when it already covers
/// every candidate, otherwise via a live rules review and/or per-item checks
/// — deduplicating that whole live-resolution path (rules review, per-item
/// checks, and the store) across concurrent requests for the same key within
/// this process: only the first one actually performs it, the rest wait on a
/// per-key lock and then find the cache populated. A `cached` outcome that
/// already covers everything is the one case with nothing to dedupe, so it
/// skips the lock entirely. Returns `(outcome, from_cache)`.
#[allow(clippy::too_many_arguments)]
async fn resolve_outcome_for(
    proxy: &ProxyKubeApi,
    state: &web::Data<State>,
    client: &reqwest::Client,
    req: &HttpRequest,
    peer_addr: Option<PeerAddr>,
    user: Option<&User>,
    base_url: &str,
    probe: virtual_api::AccessProbe,
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
        let outcome = resolve_via_rules_review(client, req, peer_addr, user, base_url, probe)
            .await
            .unwrap_or_default();
        let outcome = resolve_unresolved(
            state, proxy, client, req, peer_addr, user, base_url, probe, outcome, items, false,
        )
        .await;
        return (outcome, false);
    }

    let key = cache_key(proxy, user, probe);
    let lock = acquire_outcome_lock(&key);
    let result = {
        let _guard = lock.lock().await;
        // Double-checked: another request for this key may have resolved and
        // stored a fully-checked outcome — including the per-item checks —
        // while we were waiting for the lock.
        match load_cached_outcome(state, proxy, user, probe).await {
            Some(outcome) => {
                let outcome = resolve_unresolved(
                    state, proxy, client, req, peer_addr, user, base_url, probe, outcome, items,
                    true,
                )
                .await;
                (outcome, true)
            }
            None => {
                let outcome =
                    resolve_via_rules_review(client, req, peer_addr, user, base_url, probe)
                        .await
                        .unwrap_or_default();
                let outcome = resolve_unresolved(
                    state, proxy, client, req, peer_addr, user, base_url, probe, outcome, items,
                    false,
                )
                .await;
                (outcome, false)
            }
        }
    };
    drop(lock);
    release_outcome_lock(&key);
    result
}

/// Produce the `NamespaceList` filtered to what the impersonated caller can
/// individually `get`, or a [`DiscoveryError`] if the candidate set itself
/// could not be produced.
///
/// An individual namespace's access check erroring (as opposed to a plain
/// denial) drops just that namespace and logs a warning: fail-closed per
/// item, never whole-request. See the module doc for why this is
/// unconditional rather than a 403 rescue, and for the rules-review fast path
/// and cache in front of the per-item loop.
#[allow(clippy::too_many_arguments)]
pub(super) async fn list_projects_filtered(
    proxy: &ProxyKubeApi,
    state: &web::Data<State>,
    client: &reqwest::Client,
    req: &HttpRequest,
    peer_addr: Option<PeerAddr>,
    user: Option<&User>,
    base_url: &str,
    namespaces_path: &str,
    query_string: &str,
    probe: virtual_api::AccessProbe,
) -> Result<Value, DiscoveryError> {
    // Independent upstream calls: the candidate collection itself, and
    // whatever a cache hit can already resolve for it. Run them concurrently
    // rather than paying for both round-trips one after the other.
    let (candidates_result, cached) = tokio::join!(
        privileged_namespaces(proxy, state, base_url, namespaces_path, query_string),
        load_cached_outcome(state, proxy, user, probe)
    );
    let mut candidates = candidates_result.map_err(DiscoveryError)?;

    let items = candidates
        .get_mut("items")
        .and_then(Value::as_array_mut)
        .map(std::mem::take)
        .unwrap_or_default();

    let (outcome, from_cache) = resolve_outcome_for(
        proxy, state, client, req, peer_addr, user, base_url, probe, cached, &items,
    )
    .await;

    let kept: Vec<Value> = items
        .into_iter()
        .filter(|item| {
            outcome.unrestricted
                || item
                    .get("metadata")
                    .and_then(|metadata| metadata.get("name"))
                    .and_then(Value::as_str)
                    .is_some_and(|name| outcome.names.contains(name))
        })
        .collect();

    debug!(
        kept = kept.len(),
        from_cache, "filtered LIST projects to visible namespaces"
    );
    candidates["items"] = Value::Array(kept);
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe() -> virtual_api::AccessProbe {
        virtual_api::AccessProbe {
            group: "",
            resource: "namespaces",
            verb: "get",
        }
    }

    #[test]
    fn resolves_resource_names_restricted_rules() {
        let body = json!({
            "status": {
                "incomplete": false,
                "resourceRules": [
                    {
                        "verbs": ["get"],
                        "apiGroups": [""],
                        "resources": ["namespaces"],
                        "resourceNames": ["dev", "staging"],
                    },
                ],
            }
        });
        let outcome = interpret_rules_review(&body, probe()).expect("should resolve");
        assert!(!outcome.unrestricted);
        assert_eq!(
            outcome.names,
            HashSet::from(["dev".to_string(), "staging".to_string()])
        );
    }

    #[test]
    fn unions_resource_names_across_multiple_matching_rules() {
        let body = json!({
            "status": {
                "resourceRules": [
                    { "verbs": ["get"], "apiGroups": [""], "resources": ["namespaces"], "resourceNames": ["dev"] },
                    { "verbs": ["get"], "apiGroups": [""], "resources": ["namespaces"], "resourceNames": ["staging"] },
                    { "verbs": ["list"], "apiGroups": [""], "resources": ["namespaces"] },
                ],
            }
        });
        let outcome = interpret_rules_review(&body, probe()).expect("should resolve");
        assert!(!outcome.unrestricted);
        assert_eq!(
            outcome.names,
            HashSet::from(["dev".to_string(), "staging".to_string()])
        );
    }

    #[test]
    fn a_rule_without_resource_names_is_unrestricted() {
        let body = json!({
            "status": {
                "resourceRules": [
                    { "verbs": ["get", "list"], "apiGroups": [""], "resources": ["namespaces"] },
                ],
            }
        });
        let outcome = interpret_rules_review(&body, probe()).expect("should resolve");
        assert!(outcome.unrestricted);
    }

    #[test]
    fn a_wildcard_verb_or_group_or_resource_matches() {
        let body = json!({
            "status": {
                "resourceRules": [
                    { "verbs": ["*"], "apiGroups": ["*"], "resources": ["*"], "resourceNames": ["dev"] },
                ],
            }
        });
        let outcome = interpret_rules_review(&body, probe()).expect("should resolve");
        assert_eq!(outcome.names, HashSet::from(["dev".to_string()]));
    }

    #[test]
    fn unrelated_rules_are_ignored() {
        let body = json!({
            "status": {
                "resourceRules": [
                    { "verbs": ["get"], "apiGroups": [""], "resources": ["pods"], "resourceNames": ["dev"] },
                    { "verbs": ["watch"], "apiGroups": [""], "resources": ["namespaces"], "resourceNames": ["dev"] },
                ],
            }
        });
        let outcome = interpret_rules_review(&body, probe()).expect("should resolve");
        assert!(!outcome.unrestricted);
        assert!(outcome.names.is_empty());
    }

    #[test]
    fn incomplete_reviews_resolve_nothing() {
        let body = json!({
            "status": {
                "incomplete": true,
                "resourceRules": [
                    { "verbs": ["get"], "apiGroups": [""], "resources": ["namespaces"] },
                ],
            }
        });
        assert!(interpret_rules_review(&body, probe()).is_none());
    }

    #[test]
    fn a_missing_status_resolves_nothing() {
        assert!(interpret_rules_review(&json!({}), probe()).is_none());
    }
}
