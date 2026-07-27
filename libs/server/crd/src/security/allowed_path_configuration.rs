use std::sync::LazyLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::default::default_disabled;
use crate::security::path_matcher::{
    expand_parametised_patterns, path_equals, path_matches_pattern,
};

/// Mustache-like parameters (`{{username}}`, `{{group}}`) inside a configured path.
/// Compiled once: the pattern is a literal, so it cannot fail to compile.
static MUSTACHE_REGEX: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\{\{(\w+)\}\}").expect("static regex is valid"));

/// The placeholder names used in a parametised template, e.g.
/// `["username", "group"]`. Shared with the namespace access rules so both
/// validate placeholders identically.
pub(crate) fn mustache_captures(template: &str) -> Vec<String> {
    MUSTACHE_REGEX
        .captures_iter(template)
        .map(|capture| capture[1].to_string())
        .collect()
}

/// RE2 pattern accepting only the supported placeholders, shared between the
/// CEL admission rule and the test that keeps it honest against `validate()`.
// Only referenced from tests, but kept next to the rule it mirrors.
#[allow(dead_code)]
pub(crate) const PLACEHOLDER_PATTERN: &str = "^[^{}]*(([{][{](username|group)[}][}])[^{}]*)*$";

/// The CEL rule built from [`PLACEHOLDER_PATTERN`].
const PLACEHOLDER_RULE: &str = concat!(
    "!self.parametised || self.path.matches('",
    "^[^{}]*(([{][{](username|group)[}][}])[^{}]*)*$",
    "')"
);

/// Allowed path configuration, used in conjunction with the allowed_paths configuration
///
/// The CEL rules mirror [`AllowedPathConfiguration::validate`] so a malformed
/// rule is refused at admission rather than at reconcile time.
///
/// The placeholder regex matches braces through character classes (`[{]`)
/// rather than backslash escapes, for two independent reasons: CEL string
/// literals reject `\{` outright, and the CRD is also shipped as a Helm
/// template where two consecutive `{` would be read as a Go template action.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "self.path.startsWith('/')",
        "message": "path must start with '/'",
    }),
    serde_json::json!({
        "rule": PLACEHOLDER_RULE,
        "message": "a parametised path only accepts the username and group placeholders",
    }),
]))]
pub struct AllowedPathConfiguration {
    /// The path to allow, if the request path equals this path, it will be allowed
    ///
    /// The length bound is not cosmetic: without it the apiserver assumes an
    /// unbounded string and refuses the CEL rule above as too expensive.
    #[schemars(length(max = 512))]
    pub path: String,

    /// Wether or not the path is parametised
    /// if true, the path will be treated as a template, it either handle wildcard parameters, like "*" or "dev-*", or mustache-like parameters, like "{{username}}" or "{{group}}"
    /// for example, if the path is "/api/v1/namespaces/*/pods", it will allow all requests to pods in any namespace
    /// if the path is "/api/v1/namespaces/dev-*/pods", it will allow all requests to pods in namespaces that start with "dev-"
    /// It will also try to detect mustache-like parameters, for example, if the path is "/api/v1/namespaces/{{username}}/pods"
    /// it will allow all requests to pods in namespaces carrying the username as a parameter
    /// if the selected field is an array, like the groups claim, it will try to match any of the values in the array
    /// for example, if the groups claim is ["dev-alice", "dev-bob"] and the path is "/api/v1/namespaces/{{group}}/pods", it will allow all requests to pods in namespaces that match either "dev-alice" or "dev-bob"
    /// Allowed parameters are : {{username}} and {{group}}
    /// A `*` only matches inside a single path segment; use a `**` segment to
    /// match any number of remaining segments (needed for subresources such as
    /// "/api/v1/namespaces/dev/pods/mypod/log")
    /// default: false
    #[serde(default = "default_disabled")]
    pub parametised: bool,
}

impl AllowedPathConfiguration {
    pub fn validate(&self) -> Result<(), String> {
        if self.parametised {
            // detect any mustache-like parameters
            for cap in MUSTACHE_REGEX.captures_iter(&self.path) {
                let param = &cap[1];
                if param != "username" && param != "group" {
                    return Err(format!("Invalid parameter in path: {}, allowed parameters are {{username}} and {{group}}", param));
                }
            }
        }
        Ok(())
    }

    pub fn extract_parameters(&self) -> Vec<String> {
        let mut params = Vec::new();
        if self.parametised {
            for cap in MUSTACHE_REGEX.captures_iter(&self.path) {
                let param = &cap[1];
                params.push(param.to_string());
            }
        }
        params
    }

    pub fn to_possible_paths(&self, username: &str, groups: &[String]) -> Vec<String> {
        if self.parametised {
            // Substituted claim values are inlined literally (no `*`/`/`
            // semantics) so a caller cannot self-escalate; see
            // [`expand_parametised_patterns`].
            expand_parametised_patterns(&self.path, username, groups)
        } else {
            vec![self.path.clone()]
        }
    }

    pub fn has_wildcard(&self) -> bool {
        self.path.contains('*')
    }

    /// Check whether an upstream request path is allowed by this rule.
    ///
    /// `path` must be the path forwarded to the Kubernetes API (the
    /// `/clusters/{ns}/{cluster}` prefix already stripped) and without its
    /// query string.
    ///
    /// A non-parametised rule is an exact match: wildcards and `{{...}}`
    /// placeholders are only interpreted when `parametised` is set.
    pub fn matches(&self, path: &str, username: &str, groups: &[String]) -> bool {
        if !self.parametised {
            return path_equals(&self.path, path);
        }
        self.to_possible_paths(username, groups)
            .iter()
            .any(|candidate| path_matches_pattern(candidate, path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(path: &str, parametised: bool) -> AllowedPathConfiguration {
        AllowedPathConfiguration {
            path: path.to_string(),
            parametised,
        }
    }

    fn groups() -> Vec<String> {
        vec!["dev-alice".to_string(), "dev-bob".to_string()]
    }

    #[test]
    fn non_parametised_rule_is_an_exact_match() {
        let rule = rule("/api/v1/pods", false);
        assert!(rule.matches("/api/v1/pods", "alice", &groups()));
        assert!(!rule.matches("/api/v1/pods/mypod", "alice", &groups()));
        // Wildcards are inert when the rule is not parametised.
        let literal_star = rule_with_star();
        assert!(!literal_star.matches("/api/v1/namespaces/dev/pods", "alice", &groups()));
        assert!(literal_star.matches("/api/v1/namespaces/*/pods", "alice", &groups()));
    }

    fn rule_with_star() -> AllowedPathConfiguration {
        rule("/api/v1/namespaces/*/pods", false)
    }

    #[test]
    fn parametised_rule_expands_wildcards() {
        let rule = rule("/api/v1/namespaces/*/pods", true);
        assert!(rule.matches("/api/v1/namespaces/dev/pods", "alice", &groups()));
        assert!(!rule.matches("/api/v1/namespaces/dev/pods/mypod", "alice", &groups()));
    }

    #[test]
    fn parametised_rule_expands_username() {
        let rule = rule("/api/v1/namespaces/{{username}}/pods", true);
        assert!(rule.matches("/api/v1/namespaces/alice/pods", "alice", &groups()));
        assert!(!rule.matches("/api/v1/namespaces/bob/pods", "alice", &groups()));
    }

    #[test]
    fn parametised_rule_expands_every_group() {
        let rule = rule("/api/v1/namespaces/{{group}}/pods", true);
        assert!(rule.matches("/api/v1/namespaces/dev-alice/pods", "alice", &groups()));
        assert!(rule.matches("/api/v1/namespaces/dev-bob/pods", "alice", &groups()));
        assert!(!rule.matches("/api/v1/namespaces/prod/pods", "alice", &groups()));
        // No group at all means nothing can match.
        assert!(!rule.matches("/api/v1/namespaces/dev-alice/pods", "alice", &[]));
    }

    #[test]
    fn parametised_rule_combines_username_and_group() {
        let rule = rule("/api/v1/namespaces/{{group}}/pods/{{username}}/log", true);
        assert!(rule.matches(
            "/api/v1/namespaces/dev-bob/pods/alice/log",
            "alice",
            &groups()
        ));
        assert!(!rule.matches(
            "/api/v1/namespaces/dev-bob/pods/bob/log",
            "alice",
            &groups()
        ));
    }

    #[test]
    fn validate_accepts_known_parameters() {
        assert!(rule("/api/v1/namespaces/{{username}}/pods", true)
            .validate()
            .is_ok());
        assert!(rule("/api/v1/namespaces/{{group}}/pods", true)
            .validate()
            .is_ok());
    }

    #[test]
    fn validate_rejects_unknown_parameters() {
        let err = rule("/api/v1/namespaces/{{tenant}}/pods", true)
            .validate()
            .unwrap_err();
        assert!(err.contains("tenant"), "unexpected error: {err}");
    }

    #[test]
    fn validate_ignores_parameters_when_not_parametised() {
        assert!(rule("/api/v1/namespaces/{{tenant}}/pods", false)
            .validate()
            .is_ok());
    }

    /// The CEL admission rule and `validate()` must agree, otherwise a CR is
    /// accepted by one and rejected by the other. CEL uses RE2, and so does the
    /// `regex` crate, so the pattern can be checked here directly.
    #[test]
    fn the_cel_placeholder_rule_agrees_with_validate() {
        let cel_pattern = regex::Regex::new(PLACEHOLDER_PATTERN).unwrap();

        let paths = [
            "/api/v1/pods",
            "/api/v1/namespaces/{{username}}/pods",
            "/api/v1/namespaces/{{group}}/pods",
            "/api/v1/namespaces/{{group}}/pods/{{username}}/log",
            "/api/v1/namespaces/{{tenant}}/pods",
            "/api/v1/namespaces/{{username}}/pods/{{tenant}}",
            "/api/v1/namespaces/dev-*/pods",
        ];

        for path in paths {
            let rule = rule(path, true);
            let cel_accepts = cel_pattern.is_match(path);
            let validate_accepts = rule.validate().is_ok();
            assert_eq!(
                cel_accepts, validate_accepts,
                "CEL and validate() disagree on {path}"
            );
        }
    }

    #[test]
    fn placeholder_value_with_wildcard_cannot_self_escalate() {
        let rule = rule("/api/v1/namespaces/{{username}}/pods/**", true);
        // A caller whose username claim is `*` must not reach every namespace.
        assert!(!rule.matches("/api/v1/namespaces/prod/pods/secret", "*", &groups()));
        // A `/` in the claim must not cross a segment boundary.
        assert!(!rule.matches("/api/v1/namespaces/prod/pods/secret", "dev/prod", &groups()));
        // The legitimate literal case still works.
        assert!(rule.matches("/api/v1/namespaces/alice/pods/x", "alice", &groups()));
    }

    #[test]
    fn unsafe_group_value_is_skipped_but_safe_ones_still_match() {
        let rule = rule("/api/v1/namespaces/{{group}}/pods", true);
        let groups = vec!["*".to_string(), "dev-alice".to_string()];
        assert!(!rule.matches("/api/v1/namespaces/prod/pods", "alice", &groups));
        assert!(rule.matches("/api/v1/namespaces/dev-alice/pods", "alice", &groups));
    }

    #[test]
    fn extract_parameters_lists_placeholders() {
        assert_eq!(
            rule("/api/v1/namespaces/{{group}}/pods/{{username}}", true).extract_parameters(),
            vec!["group".to_string(), "username".to_string()]
        );
        assert!(rule("/api/v1/pods", true).extract_parameters().is_empty());
    }
}
