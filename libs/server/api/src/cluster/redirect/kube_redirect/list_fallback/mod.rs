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

use actix_web::{HttpRequest, dev::PeerAddr, web};
use common::State;
use crd::ProxyKubeApi;
use crd::certificate::CertSource;
use crd::virtual_api::VirtualApiKind;
use serde_json::Value;
use tracing::debug;

use crate::model::user::User;
use cache::load_cached_outcome;
use resolve::{privileged_namespaces, resolve_outcome_for};

mod cache;
mod resolve;
mod review;
mod tuning;

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

/// Everything [`list_projects_filtered`] needs about the request and its
/// target, borrowed from the caller. Three of these are `&str`, so naming them
/// keeps a transposition from compiling.
pub(super) struct ListFallbackArgs<'a> {
    pub(super) proxy: &'a ProxyKubeApi,
    pub(super) state: &'a web::Data<State>,
    /// Client for the caller's own (impersonated) access reviews.
    pub(super) client: &'a reqwest::Client,
    pub(super) req: &'a HttpRequest,
    pub(super) peer_addr: Option<PeerAddr>,
    pub(super) user: Option<&'a User>,
    /// Upstream origin, trailing slash already trimmed.
    pub(super) base_url: &'a str,
    /// Upstream path of the candidate collection, e.g. `/api/v1/namespaces`.
    pub(super) namespaces_path: &'a str,
    /// The client's query string, forwarded to the candidate fetch.
    pub(super) query_string: &'a str,
    pub(super) probe: virtual_api::AccessProbe,
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
pub(super) async fn list_projects_filtered(
    args: &ListFallbackArgs<'_>,
) -> Result<Value, DiscoveryError> {
    // Independent upstream calls: the candidate collection itself, and
    // whatever a cache hit can already resolve for it. Run them concurrently
    // rather than paying for both round-trips one after the other.
    let (candidates_result, cached) = tokio::join!(
        privileged_namespaces(
            args.proxy,
            args.state,
            args.base_url,
            args.namespaces_path,
            args.query_string
        ),
        load_cached_outcome(args.state, args.proxy, args.user, args.probe)
    );
    let mut candidates = candidates_result.map_err(DiscoveryError)?;

    let items = candidates
        .get_mut("items")
        .and_then(Value::as_array_mut)
        .map(std::mem::take)
        .unwrap_or_default();

    let (outcome, from_cache) = resolve_outcome_for(args, cached, &items).await;

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
