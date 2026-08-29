//! Everything the redirect workers need, bundled once.
//!
//! `redirect()` resolves the cluster, authenticates the caller, applies the
//! throttling and authorization checks, and only then picks a worker. All three
//! workers need the same set of values, which used to be threaded through as
//! nine positional arguments — four of them `String`/`Option<..>` shaped alike,
//! so a transposition would have compiled and silently forwarded the wrong
//! thing on the security-sensitive hot path. Passing one named struct makes
//! that class of mistake a compile error.

use actix_web::{HttpRequest, dev::PeerAddr, http, web};
use common::State;
use crd::ProxyKubeApi;

use crate::cluster::redirect::audit::AuditContext;
use crate::model::user::User;

pub(super) struct RedirectContext {
    pub req: HttpRequest,
    pub data: web::Data<State>,
    pub payload: web::Payload,
    pub method: http::Method,
    pub peer_addr: Option<PeerAddr>,
    pub proxy: ProxyKubeApi,
    /// The caller, when the cluster validates tokens. `None` means validation
    /// is disabled — never "the caller failed to authenticate", which is
    /// refused before a worker is reached.
    pub user: Option<User>,
    pub audit: AuditContext,
    /// Upstream origin, with any trailing slash already trimmed.
    pub base_url: String,
    /// Request path with the `/clusters/{ns}/{cluster}` routing prefix removed.
    /// Always starts with `/`, or is empty for a request to the prefix itself.
    pub upstream_path: String,
}

impl RedirectContext {
    /// The absolute upstream URL for a plain forward.
    pub(super) fn url_to_call(&self) -> String {
        join_upstream_url(&self.base_url, &self.upstream_path, self.req.query_string())
    }
}

/// Assemble the upstream URL from the cluster origin, the routed path, and the
/// client's query string.
///
/// Everything is concatenated verbatim, deliberately:
///
/// - the query string is the client's, and rewriting or re-encoding it here
///   would change what the cluster's own admission and RBAC see — a
///   `fieldSelector` or `labelSelector` that means something different upstream
///   than what was authorized is exactly the bug this must not introduce;
/// - the path has already been stripped of the `/clusters/{ns}/{cluster}`
///   routing prefix and matched against the allow-list by `redirect()`, so
///   normalising it *now* would forward something other than what was checked.
fn join_upstream_url(base_url: &str, upstream_path: &str, query_string: &str) -> String {
    if query_string.is_empty() {
        format!("{base_url}{upstream_path}")
    } else {
        format!("{base_url}{upstream_path}?{query_string}")
    }
}

#[cfg(test)]
mod tests {
    use super::join_upstream_url;

    const BASE: &str = "https://cluster.example.com:6443";

    #[test]
    fn a_path_without_a_query_is_appended_to_the_origin() {
        assert_eq!(
            join_upstream_url(BASE, "/api/v1/namespaces/dev/pods", ""),
            "https://cluster.example.com:6443/api/v1/namespaces/dev/pods"
        );
    }

    #[test]
    fn a_query_string_is_appended_after_a_single_question_mark() {
        assert_eq!(
            join_upstream_url(BASE, "/api/v1/pods", "watch=true"),
            "https://cluster.example.com:6443/api/v1/pods?watch=true"
        );
    }

    #[test]
    fn a_query_string_is_forwarded_byte_for_byte() {
        // Re-encoding here would change the selector the cluster evaluates, so
        // the escapes, the repeated keys and the ordering all survive as-is.
        let query = "fieldSelector=metadata.name%3Dweb&labelSelector=a%3Db%2Cc%3Dd&limit=500&continue=ey%2FJ%2B";
        assert_eq!(
            join_upstream_url(BASE, "/api/v1/pods", query),
            format!("{BASE}/api/v1/pods?{query}")
        );
    }

    #[test]
    fn a_request_to_the_cluster_root_keeps_the_bare_origin() {
        // `/clusters/{ns}/{cluster}` with nothing after it strips to an empty
        // path, which must not become "//" or a stray "/".
        assert_eq!(join_upstream_url(BASE, "", ""), BASE);
        assert_eq!(
            join_upstream_url(BASE, "", "timeout=30s"),
            "https://cluster.example.com:6443?timeout=30s"
        );
    }

    #[test]
    fn the_path_is_forwarded_verbatim_including_what_looks_like_routing() {
        // `redirect()` strips only the leading routing prefix, so a path that
        // legitimately repeats it must reach the cluster intact rather than
        // being stripped a second time here.
        assert_eq!(
            join_upstream_url(BASE, "/api/v1/namespaces/clusters/dev/pods", ""),
            "https://cluster.example.com:6443/api/v1/namespaces/clusters/dev/pods"
        );
        // Percent-encoded separators stay encoded: decoding them would let a
        // path that passed the allow-list reach a different upstream resource.
        assert_eq!(
            join_upstream_url(BASE, "/api/v1/namespaces/dev%2Fadmin/pods", ""),
            "https://cluster.example.com:6443/api/v1/namespaces/dev%2Fadmin/pods"
        );
    }

    #[test]
    fn an_empty_query_never_leaves_a_trailing_question_mark() {
        // A bare "?" is legal but changes the request line the cluster logs and
        // audits; an absent query must stay absent.
        assert!(!join_upstream_url(BASE, "/api/v1/pods", "").ends_with('?'));
    }
}
