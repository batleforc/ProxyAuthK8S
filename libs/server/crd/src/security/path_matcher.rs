//! Path pattern matching used by `SecurityConfiguration::allowed_resources`.
//!
//! Patterns are matched segment by segment against the *upstream* request path
//! (the path as it is sent to the Kubernetes API, i.e. after the
//! `/clusters/{ns}/{cluster}` prefix has been stripped):
//!
//! - a literal segment matches itself;
//! - `*` inside a segment matches any run of characters within that segment
//!   only, so `dev-*` matches `dev-team` but never `dev-team/pods`;
//! - a `**` segment matches any number of remaining segments, which is what
//!   makes subresources (`.../pods/mypod/log`) reachable.
//!
//! Matching never looks at the query string.

/// Split a path into its non-empty segments, ignoring leading/trailing slashes.
fn split_segments(value: &str) -> Vec<&str> {
    value.split('/').filter(|s| !s.is_empty()).collect()
}

/// A request segment that carries no authorization meaning by itself but changes
/// the effective resource once the upstream apiserver path-cleans it.
///
/// `..`/`.` (and their common percent-encodings) let a request such as
/// `/api/v1/namespaces/dev/../prod/secrets` slip past an allow rule scoped to
/// `dev` while the apiserver resolves it to `prod`. Encoded slashes (`%2f`) are
/// equally dangerous because they hide a segment boundary from the matcher.
fn is_traversal_segment(segment: &str) -> bool {
    if segment == "." || segment == ".." {
        return true;
    }
    let lower = segment.to_ascii_lowercase();
    lower == "%2e" || lower == "%2e%2e" || lower.contains("%2f") || lower.contains("%5c")
}

/// Reject any request path that contains a traversal / encoded-separator segment
/// so the matcher and the upstream agree on which resource is addressed.
fn request_path_is_safe(path_segments: &[&str]) -> bool {
    !path_segments.iter().any(|segment| is_traversal_segment(segment))
}

/// Match a single segment against a pattern segment that may contain `*`.
fn segment_matches(pattern: &str, segment: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == segment;
    }

    let parts: Vec<&str> = pattern.split('*').collect();
    let last_index = parts.len() - 1;
    let mut rest = segment;

    for (index, part) in parts.iter().enumerate() {
        if index == 0 {
            match rest.strip_prefix(part) {
                Some(remainder) => rest = remainder,
                None => return false,
            }
        } else if index == last_index {
            // A trailing `*` swallows whatever is left.
            if part.is_empty() {
                return true;
            }
            return rest.len() >= part.len() && rest.ends_with(part);
        } else {
            match rest.find(part) {
                Some(idx) => rest = &rest[idx + part.len()..],
                None => return false,
            }
        }
    }

    true
}

fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            if rest.is_empty() {
                return true;
            }
            (0..=path.len()).any(|skipped| match_segments(rest, &path[skipped..]))
        }
        Some((head, rest)) => match path.split_first() {
            Some((path_head, path_rest)) if segment_matches(head, path_head) => {
                match_segments(rest, path_rest)
            }
            _ => false,
        },
    }
}

/// Match a request path against a configured pattern.
pub fn path_matches_pattern(pattern: &str, path: &str) -> bool {
    let path_segments = split_segments(path);
    if !request_path_is_safe(&path_segments) {
        return false;
    }
    match_segments(&split_segments(pattern), &path_segments)
}

/// Compare two paths ignoring leading/trailing slash differences only.
pub fn path_equals(configured: &str, path: &str) -> bool {
    let path_segments = split_segments(path);
    if !request_path_is_safe(&path_segments) {
        return false;
    }
    split_segments(configured) == path_segments
}

/// Whether a username/group claim value can be substituted into a single path
/// segment without leaking pattern semantics into the matcher.
///
/// Rejects empty values, wildcard characters (`*`), segment separators (`/`) and
/// traversal segments so the injected value can only ever match itself literally
/// (otherwise a caller whose claim is `*` could self-escalate).
pub fn is_safe_placeholder_value(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('*')
        && !value.contains('/')
        && value != "."
        && value != ".."
}

/// Expand a parametised template by substituting `{{username}}` / `{{group}}`
/// with the caller's (literal-safe) claim values, yielding candidate patterns.
///
/// A `{{username}}` whose value is unsafe collapses the whole expansion to no
/// candidate (fail-closed); unsafe individual groups are simply skipped. Shared
/// by the allowed-path rules and the namespace access rules so both apply the
/// exact same substitution hardening.
pub fn expand_parametised_patterns(
    template: &str,
    username: &str,
    groups: &[String],
) -> Vec<String> {
    let mut path = template.to_string();
    if path.contains("{{username}}") {
        if !is_safe_placeholder_value(username) {
            return Vec::new();
        }
        path = path.replace("{{username}}", username);
    }
    if path.contains("{{group}}") {
        groups
            .iter()
            .filter(|group| is_safe_placeholder_value(group))
            .map(|group| path.replace("{{group}}", group))
            .collect()
    } else {
        vec![path]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_paths_match_exactly() {
        assert!(path_matches_pattern("/api/v1/pods", "/api/v1/pods"));
        assert!(!path_matches_pattern("/api/v1/pods", "/api/v1/services"));
        assert!(!path_matches_pattern("/api/v1/pods", "/api/v1/pods/mypod"));
    }

    #[test]
    fn leading_and_trailing_slashes_are_irrelevant() {
        assert!(path_matches_pattern("api/v1/pods", "/api/v1/pods"));
        assert!(path_matches_pattern("/api/v1/pods/", "/api/v1/pods"));
        assert!(path_equals("/api/v1/pods/", "api/v1/pods"));
    }

    #[test]
    fn star_matches_a_single_segment() {
        assert!(path_matches_pattern(
            "/api/v1/namespaces/*/pods",
            "/api/v1/namespaces/dev/pods"
        ));
        // `*` must not cross a `/`.
        assert!(!path_matches_pattern(
            "/api/v1/namespaces/*/pods",
            "/api/v1/namespaces/dev/team/pods"
        ));
        assert!(!path_matches_pattern(
            "/api/v1/namespaces/*/pods",
            "/api/v1/namespaces/dev/pods/mypod"
        ));
    }

    #[test]
    fn star_matches_a_prefix_inside_a_segment() {
        assert!(path_matches_pattern(
            "/api/v1/namespaces/dev-*/pods",
            "/api/v1/namespaces/dev-team/pods"
        ));
        assert!(!path_matches_pattern(
            "/api/v1/namespaces/dev-*/pods",
            "/api/v1/namespaces/prod-team/pods"
        ));
        // A bare `dev-*` must not match the empty-suffix-only case `dev`.
        assert!(!path_matches_pattern(
            "/api/v1/namespaces/dev-*/pods",
            "/api/v1/namespaces/dev/pods"
        ));
    }

    #[test]
    fn star_can_appear_in_the_middle_of_a_segment() {
        assert!(segment_matches("dev-*-front", "dev-team-front"));
        assert!(!segment_matches("dev-*-front", "dev-team-back"));
        assert!(segment_matches("*-front", "dev-front"));
        assert!(segment_matches("*", "anything"));
        assert!(segment_matches("*", ""));
    }

    #[test]
    fn double_star_matches_any_number_of_segments() {
        assert!(path_matches_pattern(
            "/api/v1/namespaces/dev/pods/**",
            "/api/v1/namespaces/dev/pods"
        ));
        assert!(path_matches_pattern(
            "/api/v1/namespaces/dev/pods/**",
            "/api/v1/namespaces/dev/pods/mypod/log"
        ));
        assert!(!path_matches_pattern(
            "/api/v1/namespaces/dev/pods/**",
            "/api/v1/namespaces/prod/pods/mypod"
        ));
        assert!(path_matches_pattern("/**", "/apis/apps/v1/deployments"));
    }

    #[test]
    fn double_star_in_the_middle_backtracks() {
        assert!(path_matches_pattern("/api/**/log", "/api/v1/pods/x/log"));
        assert!(path_matches_pattern("/api/**/log", "/api/log"));
        assert!(!path_matches_pattern("/api/**/log", "/api/v1/pods/x/exec"));
    }

    #[test]
    fn empty_pattern_only_matches_the_root() {
        assert!(path_matches_pattern("/", "/"));
        assert!(!path_matches_pattern("/", "/api"));
    }

    #[test]
    fn traversal_segments_are_rejected() {
        // `..` must not let a `dev`-scoped rule reach `prod`.
        assert!(!path_matches_pattern(
            "/api/v1/namespaces/dev/**",
            "/api/v1/namespaces/dev/../prod/secrets"
        ));
        assert!(!path_matches_pattern("/**", "/api/v1/../secrets"));
        assert!(!path_matches_pattern("/api/./v1/pods", "/api/./v1/pods"));
        // Percent-encoded dot segments and encoded slashes are refused too.
        assert!(!path_matches_pattern("/**", "/api/v1/%2e%2e/secrets"));
        assert!(!path_matches_pattern("/**", "/api/v1/namespaces%2fprod/secrets"));
        // `path_equals` is guarded identically.
        assert!(!path_equals("/api/v1/../secrets", "/api/v1/../secrets"));
    }
}
