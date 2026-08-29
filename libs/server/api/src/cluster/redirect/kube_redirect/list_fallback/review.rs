//! The two authorization reviews this module issues as the impersonated
//! caller, and the interpretation of the rules review's response.

use std::collections::HashSet;

use actix_web::{HttpRequest, dev::PeerAddr, http};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cluster::redirect::kube_redirect::{
    list_fallback::tuning::upstream_timeout, upstream::apply_forward_headers,
};
use crate::model::user::User;

/// `POST .../selfsubjectaccessreviews` as the impersonated caller.
///
/// `Ok(false)` is an explicit denial: the caller records it into
/// `RulesOutcome::denied` and caches it exactly like an allow. `Err(())` on
/// any transport/parse failure means "cannot confirm" instead — the caller
/// excludes the item for this request but never caches the result, so it is
/// re-checked on every subsequent request until it actually resolves either
/// way. Only the latter is worth a warning; a plain denial is expected,
/// ordinary filtering.
pub(super) async fn check_access(
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
pub(super) struct RulesOutcome {
    pub(super) unrestricted: bool,
    pub(super) names: HashSet<String>,
    #[serde(default)]
    pub(super) denied: HashSet<String>,
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
pub(super) async fn resolve_via_rules_review(
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
