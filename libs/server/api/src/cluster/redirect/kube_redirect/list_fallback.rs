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
//! one `SelfSubjectAccessReview` per candidate, so two optimizations sit in
//! front of that per-item loop:
//!
//! - a single `SelfSubjectRulesReview` resolves most or all candidates in one
//!   call (see [`interpret_rules_review`] for why this is trustworthy here
//!   specifically, unlike the general case);
//! - the resolved allow-set is cached per `(cluster, caller identity)` for a
//!   short TTL, so repeated polling (dashboards, `oc projects`) does not
//!   re-run either the rules review or any access review at all.
//!
//! Per-item `SelfSubjectAccessReview` is still the fallback whenever the
//! rules review can't resolve a candidate — it is the only source ever
//! trusted for the actual allow/deny decision when there is any doubt.

use std::collections::HashSet;

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
use tracing::{debug, warn};

use super::upstream::{apply_forward_headers, ca_only_client};
use crate::model::user::User;

/// Upper bound on concurrent `SelfSubjectAccessReview` calls for one LIST.
const DEFAULT_CONCURRENCY: usize = 16;

fn concurrency() -> usize {
    std::env::var("PROXY_VIRTUAL_LIST_FALLBACK_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_CONCURRENCY)
}

/// How long a resolved allow-set is trusted before being recomputed.
///
/// Short enough that a revoked grant stops being honoured soon after; long
/// enough that a polling dashboard does not re-run a rules review (or worse,
/// per-item access reviews) on every refresh. `0` disables caching outright.
const DEFAULT_CACHE_TTL_SECONDS: u64 = 30;

fn cache_ttl_seconds() -> u64 {
    std::env::var("PROXY_VIRTUAL_LIST_FALLBACK_CACHE_TTL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_CACHE_TTL_SECONDS)
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
async fn privileged_namespaces(
    proxy: &ProxyKubeApi,
    state: &web::Data<State>,
    base_url: &str,
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
        format!("{base_url}/api/v1/namespaces")
    } else {
        format!("{base_url}/api/v1/namespaces?{query_string}")
    };

    let res = client
        .get(&url)
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
/// `Ok(false)` on an explicit denial, `Err(())` on any transport/parse
/// failure — the caller treats both as "cannot confirm", but only the latter
/// is worth a warning: a denial is expected, ordinary filtering.
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

    let mut builder = client.request(reqwest::Method::POST, &url);
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
/// matching rule, unioned.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct RulesOutcome {
    unrestricted: bool,
    names: HashSet<String>,
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
/// module uses `AccessProbe` for: `Namespace` is a cluster-scoped resource,
/// so only `ClusterRole`s bound via `ClusterRoleBinding` can ever grant `get`
/// on one (a namespaced `RoleBinding` cannot, regardless of `resourceNames`
/// — its grant only applies within its own namespace, which a cluster-scoped
/// request has none of). `resourceRules` already reports every matching
/// `ClusterRoleBinding`-derived rule regardless of the `namespace` argument
/// the review was made with, so a complete, non-incomplete result is not
/// just a hint here: it is the same computation a live
/// `SelfSubjectAccessReview` would do. This still never denies on rules-review
/// data alone — an unresolved candidate always falls through to a real
/// per-item review (see [`list_projects_filtered`]) — it only ever shortcuts
/// an *allow*.
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
            Some(names) => outcome.names.extend(
                names
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string),
            ),
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
    // The namespace argument only selects which namespace-local RoleBindings
    // are consulted; ClusterRoleBindings — the only way to grant rights on a
    // cluster-scoped resource — are included regardless, so a fixed
    // placeholder is fine and does not need to exist.
    let body = json!({
        "kind": "SelfSubjectRulesReview",
        "apiVersion": "authorization.k8s.io/v1",
        "spec": { "namespace": "default" }
    });

    let mut builder = client.request(reqwest::Method::POST, &url);
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

fn cache_key(proxy: &ProxyKubeApi, user: Option<&User>) -> String {
    format!(
        "proxyk8sauth:listfallback:{}:{}",
        proxy.to_path(),
        identity_fingerprint(user)
    )
}

async fn load_cached_outcome(
    state: &web::Data<State>,
    proxy: &ProxyKubeApi,
    user: Option<&User>,
) -> Option<RulesOutcome> {
    if cache_ttl_seconds() == 0 {
        return None;
    }
    let raw = state.redis_get(&cache_key(proxy, user)).await.ok()??;
    serde_json::from_str(&raw).ok()
}

async fn store_cached_outcome(
    state: &web::Data<State>,
    proxy: &ProxyKubeApi,
    user: Option<&User>,
    outcome: &RulesOutcome,
) {
    let ttl = cache_ttl_seconds();
    if ttl == 0 {
        return;
    }
    let Ok(raw) = serde_json::to_string(outcome) else {
        return;
    };
    if let Err(err) = state.redis_set(&cache_key(proxy, user), &raw, Some(ttl)).await {
        warn!(%err, "could not cache the resolved namespace allow-set");
    }
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
    query_string: &str,
    probe: virtual_api::AccessProbe,
) -> Result<Value, DiscoveryError> {
    let mut candidates = privileged_namespaces(proxy, state, base_url, query_string)
        .await
        .map_err(DiscoveryError)?;

    let items = candidates
        .get_mut("items")
        .and_then(Value::as_array_mut)
        .map(std::mem::take)
        .unwrap_or_default();

    let cached = load_cached_outcome(state, proxy, user).await;
    let from_cache = cached.is_some();
    let mut outcome = match cached {
        Some(outcome) => outcome,
        None => resolve_via_rules_review(client, req, peer_addr, user, base_url, probe)
            .await
            .unwrap_or_default(),
    };

    let unresolved: Vec<Value> = if outcome.unrestricted {
        Vec::new()
    } else {
        items
            .iter()
            .filter(|item| {
                let name = item
                    .get("metadata")
                    .and_then(|metadata| metadata.get("name"))
                    .and_then(Value::as_str);
                !name.is_some_and(|name| outcome.names.contains(name))
            })
            .cloned()
            .collect()
    };

    let checked = stream::iter(unresolved)
        .map(|item| async move {
            let name = item
                .get("metadata")
                .and_then(|metadata| metadata.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)?;
            match check_access(client, req, peer_addr, user, base_url, probe, &name).await {
                Ok(true) => Some(name),
                Ok(false) => None,
                Err(()) => {
                    warn!(namespace = %name, "could not confirm namespace visibility, excluding it");
                    None
                }
            }
        })
        .buffer_unordered(concurrency())
        .filter_map(|name| async move { name })
        .collect::<Vec<String>>()
        .await;
    outcome.names.extend(checked);

    if !from_cache {
        store_cached_outcome(state, proxy, user, &outcome).await;
    }

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
