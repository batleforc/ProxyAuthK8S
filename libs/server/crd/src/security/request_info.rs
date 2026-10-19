//! The Kubernetes verb of a request, computed the way the apiserver computes
//! it (`RequestInfoFactory.NewRequestInfo`, `k8s.io/apiserver/pkg/endpoints/
//! request/requestinfo.go`), so a request cannot be judged a read here and run
//! as a write there.
//!
//! One deliberate addition: the connect subresources `exec`, `attach` and
//! `portforward` are `create` whatever the method, as the apiserver authorizes
//! them (a websocket exec is a `GET`).

use super::path_matcher::{path_has_no_traversal, percent_decode_once};
use super::verb::Verb;

/// Subresources that open a session in the container: always `create`.
const CONNECT_SUBRESOURCES: [&str; 3] = ["exec", "attach", "portforward"];

/// Namespace subresources: `namespaces/{ns}/status` is the namespace's own.
const NAMESPACE_SUBRESOURCES: [&str; 2] = ["status", "finalize"];

/// What the apiserver reads from a request, for the allow-list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestInfo {
    pub verb: Verb,
    /// `false` for `/version`, `/healthz`, `/openapi/*`, discovery…
    pub is_resource_request: bool,
    pub api_group: String,
    pub version: String,
    pub namespace: String,
    pub resource: String,
    pub name: String,
    pub subresource: String,
}

/// Compute the verb of `method` on the upstream `path` (no query) with `query`.
///
/// An `Err` is a request whose verb cannot be expressed by a rule: an unknown
/// method, the legacy `/proxy/` verb, a non-resource request other than `GET`,
/// or a path the matchers refuse (traversal, encoded separator, empty
/// segment). Under a verb restriction it is denied.
pub fn request_info(method: &str, path: &str, query: &str) -> Result<RequestInfo, String> {
    if !path_has_no_traversal(path) {
        return Err("the path has a traversal or an encoded separator".to_string());
    }
    // Go's `splitPath`: trim the slashes around, split, keep empty segments.
    let trimmed = path.trim_matches('/');
    let decoded: Vec<String> = if trimmed.is_empty() {
        Vec::new()
    } else {
        trimmed.split('/').map(percent_decode_once).collect()
    };
    if decoded.iter().any(String::is_empty) {
        return Err("the path has an empty segment".to_string());
    }
    let parts: Vec<&str> = decoded.iter().map(String::as_str).collect();

    let mut info = RequestInfo {
        verb: Verb::Get,
        is_resource_request: false,
        api_group: String::new(),
        version: String::new(),
        namespace: String::new(),
        resource: String::new(),
        name: String::new(),
        subresource: String::new(),
    };
    let non_resource = |info: RequestInfo| {
        // Lower-cased method: only `get` is a verb a rule can name.
        if method == "GET" {
            Ok(info)
        } else {
            Err(format!("method {method} on a non-resource path"))
        }
    };

    if parts.len() < 3 || !matches!(parts[0], "api" | "apis") {
        return non_resource(info);
    }
    let mut rest = &parts[1..];
    if parts[0] == "apis" {
        if rest.len() < 3 {
            return non_resource(info);
        }
        info.api_group = rest[0].to_string();
        rest = &rest[1..];
    }
    info.is_resource_request = true;
    info.version = rest[0].to_string();
    rest = &rest[1..];

    let mut verb = match rest[0] {
        "watch" | "proxy" => {
            if rest.len() < 2 {
                return Err("a special verb without a resource".to_string());
            }
            let special = rest[0];
            rest = &rest[1..];
            match special {
                // The apiserver only serves `/watch/` to reads; any other
                // method has no verb a rule could name.
                "watch" if matches!(method, "GET" | "HEAD") => Some(Verb::Watch),
                "watch" => return Err(format!("method {method} on a /watch/ path")),
                // The legacy `/proxy/` verb has no rule equivalent.
                _ => return Err("the legacy proxy verb".to_string()),
            }
        }
        _ => match method {
            "POST" => Some(Verb::Create),
            "GET" | "HEAD" => Some(Verb::Get),
            "PUT" => Some(Verb::Update),
            "PATCH" => Some(Verb::Patch),
            "DELETE" => Some(Verb::Delete),
            _ => None,
        },
    };

    if rest[0] == "namespaces" && rest.len() > 1 {
        info.namespace = rest[1].to_string();
        if rest.len() > 2 && !NAMESPACE_SUBRESOURCES.contains(&rest[2]) {
            rest = &rest[2..];
        }
    }
    // resource/name/subresource/what/the/apiserver/does/not/interpret
    if let Some(resource) = rest.first() {
        info.resource = (*resource).to_string();
    }
    if let Some(name) = rest.get(1) {
        info.name = (*name).to_string();
    }
    if let Some(subresource) = rest.get(2) {
        info.subresource = (*subresource).to_string();
    }

    if info.name.is_empty() && verb == Some(Verb::Get) {
        verb = Some(if is_watch(query) {
            Verb::Watch
        } else {
            Verb::List
        });
    }
    if info.name.is_empty() && verb == Some(Verb::Delete) {
        verb = Some(Verb::Deletecollection);
    }
    let Some(mut verb) = verb else {
        return Err(format!("unknown method {method}"));
    };
    if CONNECT_SUBRESOURCES.contains(&info.subresource.as_str()) {
        verb = Verb::Create;
    }
    info.verb = verb;
    Ok(info)
}

/// The apiserver's reading of `watch`: the first value, false only for `0` or
/// `false` (any case), so `watch=`, `watch=TRUE` or `watch=yes` are watches.
fn is_watch(query: &str) -> bool {
    query
        .split('&')
        .map(|pair| pair.split_once('=').unwrap_or((pair, "")))
        .find(|(key, _)| percent_decode_once(&key.replace('+', " ")) == "watch")
        .is_some_and(|(_, value)| {
            let value = percent_decode_once(&value.replace('+', " "));
            value != "0" && !value.eq_ignore_ascii_case("false")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verb(method: &str, path: &str, query: &str) -> Result<Verb, String> {
        request_info(method, path, query).map(|info| info.verb)
    }

    /// RFC 0006 §4.2, line by line.
    #[test]
    fn the_verb_table() {
        use Verb::*;
        let cases: &[(&str, &str, &str, Verb)] = &[
            ("GET", "/api/v1/namespaces/dev/pods/web", "", Get),
            ("HEAD", "/api/v1/namespaces/dev/pods/web", "", Get),
            ("GET", "/api/v1/namespaces/dev/pods", "", List),
            ("GET", "/api/v1/pods", "", List),
            ("GET", "/api/v1/namespaces/dev/pods", "watch=true", Watch),
            ("GET", "/api/v1/namespaces/dev/pods", "watch=TRUE", Watch),
            ("GET", "/api/v1/namespaces/dev/pods", "watch=1", Watch),
            ("GET", "/api/v1/namespaces/dev/pods", "watch=", Watch),
            (
                "GET",
                "/api/v1/namespaces/dev/pods",
                "watch=yes&watch=0",
                Watch,
            ),
            ("GET", "/api/v1/namespaces/dev/pods", "watch=false", List),
            ("GET", "/api/v1/namespaces/dev/pods", "watch=0", List),
            ("GET", "/api/v1/namespaces/dev/pods", "watch", Watch),
            ("GET", "/api/v1/namespaces/dev/pods", "%77atch=1", Watch),
            ("GET", "/api/v1/watch/namespaces/dev/pods", "", Watch),
            ("GET", "/api/v1/watch/namespaces/dev/pods/web", "", Watch),
            ("POST", "/api/v1/namespaces/dev/pods", "", Create),
            ("PUT", "/api/v1/namespaces/dev/pods/web", "", Update),
            ("PATCH", "/api/v1/namespaces/dev/pods/web", "", Patch),
            ("DELETE", "/api/v1/namespaces/dev/pods/web", "", Delete),
            (
                "DELETE",
                "/api/v1/namespaces/dev/pods",
                "",
                Deletecollection,
            ),
            // Connect subresources are create whatever the method.
            (
                "GET",
                "/api/v1/namespaces/dev/pods/web/exec",
                "command=ls",
                Create,
            ),
            ("POST", "/api/v1/namespaces/dev/pods/web/exec", "", Create),
            ("GET", "/api/v1/namespaces/dev/pods/web/attach", "", Create),
            (
                "GET",
                "/api/v1/namespaces/dev/pods/web/portforward",
                "",
                Create,
            ),
            ("GET", "/api/v1/namespaces/dev/pods/web/exe%63", "", Create),
            // proxy: the method's verb, like the apiserver.
            ("GET", "/api/v1/namespaces/dev/pods/web:80/proxy/x", "", Get),
            (
                "POST",
                "/api/v1/namespaces/dev/services/web:80/proxy",
                "",
                Create,
            ),
            ("GET", "/api/v1/nodes/n1/proxy/exec/dev/web/app", "", Get),
            // Other subresources: the method's verb on the named object.
            (
                "PATCH",
                "/api/v1/namespaces/dev/pods/web/ephemeralcontainers",
                "",
                Patch,
            ),
            (
                "POST",
                "/api/v1/namespaces/dev/pods/web/eviction",
                "",
                Create,
            ),
            ("GET", "/api/v1/namespaces/dev/pods/web/log", "", Get),
            (
                "PUT",
                "/apis/apps/v1/namespaces/dev/deployments/web/scale",
                "",
                Update,
            ),
            // Namespaces and their own subresources.
            ("GET", "/api/v1/namespaces", "", List),
            ("GET", "/api/v1/namespaces/dev", "", Get),
            ("PUT", "/api/v1/namespaces/dev/status", "", Update),
            ("DELETE", "/api/v1/namespaces/dev", "", Delete),
            // Groups.
            ("GET", "/apis/apps/v1/deployments", "", List),
            (
                "DELETE",
                "/apis/apps/v1/namespaces/dev/deployments",
                "",
                Deletecollection,
            ),
            // Non-resource.
            ("GET", "/version", "", Get),
            ("GET", "/apis", "", Get),
            ("GET", "/apis/apps/v1", "", Get),
            ("GET", "/api/v1", "", Get),
            ("GET", "/openapi/v3", "", Get),
        ];
        for (method, path, query, expected) in cases {
            assert_eq!(
                verb(method, path, query),
                Ok(*expected),
                "{method} {path}?{query}"
            );
        }
    }

    #[test]
    fn undeterminable_verbs_are_errors() {
        for (method, path) in [
            ("OPTIONS", "/api/v1/namespaces/dev/pods"),
            ("TRACE", "/api/v1/namespaces/dev/pods/web"),
            ("get", "/api/v1/namespaces/dev/pods/web"),
            ("POST", "/version"),
            ("HEAD", "/healthz"),
            ("GET", "/api/v1/proxy/namespaces/dev/pods/web"),
            ("GET", "/api/v1/watch"),
            ("POST", "/api/v1/watch/namespaces/dev/pods"),
            ("DELETE", "/api/v1/watch/pods/web"),
            ("GET", "/api/v1/namespaces/dev/pods/web%2Fexec"),
            ("GET", "/api/v1/namespaces/dev/../prod/pods"),
            ("GET", "/api/v1/namespaces//pods"),
        ] {
            assert!(request_info(method, path, "").is_err(), "{method} {path}");
        }
    }

    #[test]
    fn the_parts_are_split_like_the_apiserver() {
        let info = request_info(
            "PATCH",
            "/apis/apps/v1/namespaces/dev/deployments/web/scale",
            "",
        )
        .unwrap();
        assert_eq!(
            info,
            RequestInfo {
                verb: Verb::Patch,
                is_resource_request: true,
                api_group: "apps".to_string(),
                version: "v1".to_string(),
                namespace: "dev".to_string(),
                resource: "deployments".to_string(),
                name: "web".to_string(),
                subresource: "scale".to_string(),
            }
        );
    }

    /// T7: a very long path is classified in linear time, without panicking.
    #[test]
    fn a_long_path_is_handled() {
        let path = format!("/api/v1/namespaces/dev/pods/{}", "a".repeat(64 * 1024));
        assert_eq!(verb("GET", &path, ""), Ok(Verb::Get));
    }
}
