//! RFC 0006 T1: `request_info` agrees with the apiserver's own
//! `RequestInfoFactory` on every case of `request_info_reference/cases.json`.
//! The reference is generated from Go (`task gen:request-info-reference`) and
//! versioned; the documented differences are all refusals, never a looser
//! verb:
//! - `exec`, `attach`, `portforward` are `create` whatever the method (the
//!   apiserver authorizes them so);
//! - a verb a rule cannot name is an error: unknown method, legacy `proxy`,
//!   a non-resource request other than `GET`, a non-read on `/watch/`;
//! - a traversal, encoded separator or empty segment is an error.

use crd::security::{Verb, request_info};
use serde::Deserialize;

#[derive(Deserialize)]
struct Reference {
    method: String,
    path: String,
    query: String,
    error: bool,
    is_resource_request: bool,
    verb: String,
    api_group: String,
    version: String,
    namespace: String,
    resource: String,
    name: String,
    subresource: String,
}

fn nameable(verb: &str) -> Option<Verb> {
    serde_json::from_value(serde_json::Value::String(verb.to_string()))
        .ok()
        .filter(|verb| *verb != Verb::All)
}

#[test]
fn request_info_agrees_with_the_apiserver() {
    let references: Vec<Reference> =
        serde_json::from_str(include_str!("request_info_reference.json")).unwrap();
    assert!(references.len() > 200, "the reference looks truncated");

    for go in references {
        let case = format!("{} {}?{}", go.method, go.path, go.query);
        let ours = request_info(&go.method, &go.path, &go.query);
        let separator = go.path.to_ascii_lowercase().contains("%2f");

        // An empty verb (unknown method) is one no authorizer grants.
        let expected = if go.error || separator || go.verb.is_empty() {
            None
        } else if ["exec", "attach", "portforward"].contains(&go.subresource.as_str()) {
            Some(Verb::Create)
        } else if !go.is_resource_request {
            (go.verb == "get").then_some(Verb::Get)
        } else if go.verb == "watch" && !matches!(go.method.as_str(), "GET" | "HEAD") {
            None
        } else {
            nameable(&go.verb)
        };

        match (expected, ours) {
            (None, Err(_)) => {}
            (None, Ok(info)) => {
                panic!("{case}: the apiserver gives no nameable verb, we gave {info:?}")
            }
            (Some(_), Err(err)) => panic!("{case}: we refused ({err}), the apiserver did not"),
            (Some(verb), Ok(info)) => {
                assert_eq!(info.verb, verb, "{case}: verb");
                assert_eq!(info.is_resource_request, go.is_resource_request, "{case}");
                if go.is_resource_request {
                    assert_eq!(
                        (
                            info.api_group.as_str(),
                            info.version.as_str(),
                            info.namespace.as_str(),
                            info.resource.as_str(),
                            info.name.as_str(),
                            info.subresource.as_str()
                        ),
                        (
                            go.api_group.as_str(),
                            go.version.as_str(),
                            go.namespace.as_str(),
                            go.resource.as_str(),
                            go.name.as_str(),
                            go.subresource.as_str()
                        ),
                        "{case}: parts"
                    );
                }
            }
        }
    }
}
