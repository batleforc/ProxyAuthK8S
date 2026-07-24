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
    match_segments(&split_segments(pattern), &split_segments(path))
}

/// Compare two paths ignoring leading/trailing slash differences only.
pub fn path_equals(configured: &str, path: &str) -> bool {
    split_segments(configured) == split_segments(path)
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
}
