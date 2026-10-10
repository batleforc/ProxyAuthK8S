//! The body rewriting between the virtual `Project` shape and the upstream
//! `Namespace` one.
//!
//! Kept apart from the routing/discovery surface in `mod.rs` because it is pure
//! `serde_json` translation: no paths, no verbs, and — since the bodies come
//! from a client or a semi-trusted upstream — deliberately defensive about
//! shapes that would otherwise panic on an index-assignment.

use serde_json::{Value, json};

use crate::VirtualApiMapper;
use crate::openshift::project::{
    CORE_API_VERSION, DESCRIPTION_ANNOTATION, DISPLAY_NAME_ANNOTATION, NAMESPACE_KIND,
    NAMESPACE_LIST_KIND, NAMESPACES_PATH, OpenShiftProjectMapper, PROJECT_KIND, PROJECT_LIST_KIND,
};

impl OpenShiftProjectMapper {
    /// `Namespace` -> `Project`, `NamespaceList` -> `ProjectList`.
    ///
    /// Anything else (a `Status` error body, most notably) is returned as-is:
    /// the client must still see the real apiserver error.
    pub(super) fn namespace_to_project(&self, mut body: Value) -> Value {
        match body.get("kind").and_then(Value::as_str) {
            Some(NAMESPACE_KIND) => {
                body["kind"] = json!(PROJECT_KIND);
                body["apiVersion"] = json!(self.api_version());
                body
            }
            Some(NAMESPACE_LIST_KIND) => {
                body["kind"] = json!(PROJECT_LIST_KIND);
                body["apiVersion"] = json!(self.api_version());
                if let Some(items) = body.get_mut("items").and_then(Value::as_array_mut) {
                    for item in items.iter_mut() {
                        // List items carry no kind of their own upstream; stamp
                        // the virtual one so clients reading items in isolation
                        // still see a Project. Only touch well-formed object
                        // items — index-assigning into a non-object `Value`
                        // panics, and the upstream body is only semi-trusted.
                        if let Some(obj) = item.as_object_mut() {
                            obj.insert("kind".to_string(), json!(PROJECT_KIND));
                            obj.insert("apiVersion".to_string(), json!(self.api_version()));
                        }
                    }
                }
                body
            }
            _ => body,
        }
    }

    /// `ProjectRequest`/`Project` -> `Namespace`.
    ///
    /// `displayName` and `description` are top-level fields on a `ProjectRequest`
    /// but annotations on a Namespace.
    pub(super) fn project_request_to_namespace(&self, body: Value) -> Value {
        // Only carry `metadata` over when it is actually an object; a client
        // could otherwise send `"metadata": "x"` and turn the annotation writes
        // below into a serde_json index-assignment panic.
        let metadata = body
            .get("metadata")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut namespace = json!({
            "kind": NAMESPACE_KIND,
            "apiVersion": CORE_API_VERSION,
            "metadata": metadata,
        });

        let annotations = [
            (DISPLAY_NAME_ANNOTATION, body.get("displayName")),
            (DESCRIPTION_ANNOTATION, body.get("description")),
        ];
        // Build the annotations map defensively rather than index-assigning into
        // a `Value` whose shape came from the client.
        let to_set: Vec<(&str, &str)> = annotations
            .into_iter()
            .filter_map(|(key, value)| value.and_then(Value::as_str).map(|value| (key, value)))
            .collect();
        if !to_set.is_empty()
            && let Some(meta) = namespace.get_mut("metadata").and_then(Value::as_object_mut)
            && let Some(anns) = meta
                .entry("annotations")
                .or_insert_with(|| json!({}))
                .as_object_mut()
        {
            for (key, value) in to_set {
                anns.insert(key.to_string(), json!(value));
            }
        }

        namespace
    }

    pub(super) fn projects_path(name: Option<&str>) -> String {
        match name {
            Some(name) => format!("{NAMESPACES_PATH}/{name}"),
            None => NAMESPACES_PATH.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::route::VirtualRoute;

    fn mapper() -> OpenShiftProjectMapper {
        OpenShiftProjectMapper
    }

    fn route(method: &str, path: &str) -> VirtualRoute {
        let mut route = mapper().matches(path).expect("path should be recognised");
        route.method = method.to_ascii_uppercase();
        route
    }

    #[test]
    fn rewrites_a_project_request_body_into_a_namespace() {
        let route = route("POST", "/apis/project.openshift.io/v1/projectrequests");
        let body = json!({
            "kind": "ProjectRequest",
            "apiVersion": "project.openshift.io/v1",
            "metadata": { "name": "dev" },
            "displayName": "Development",
            "description": "the dev project",
        });

        let namespace = mapper().map_request_body(&route, body);

        assert_eq!(namespace["kind"], "Namespace");
        assert_eq!(namespace["apiVersion"], "v1");
        assert_eq!(namespace["metadata"]["name"], "dev");
        assert_eq!(
            namespace["metadata"]["annotations"]["openshift.io/display-name"],
            "Development"
        );
        assert_eq!(
            namespace["metadata"]["annotations"]["openshift.io/description"],
            "the dev project"
        );
        // The OpenShift-only fields must not leak into the Namespace.
        assert!(namespace.get("displayName").is_none());
    }

    #[test]
    fn a_project_request_without_optional_fields_maps_cleanly() {
        let route = route("POST", "/apis/project.openshift.io/v1/projectrequests");
        let namespace = mapper().map_request_body(
            &route,
            json!({ "kind": "ProjectRequest", "metadata": { "name": "dev" } }),
        );
        assert_eq!(namespace["metadata"]["name"], "dev");
        assert!(namespace["metadata"].get("annotations").is_none());
    }

    #[test]
    fn rewrites_a_namespace_response_into_a_project() {
        let project = mapper().map_response(json!({
            "kind": "Namespace",
            "apiVersion": "v1",
            "metadata": {
                "name": "dev",
                "annotations": { "openshift.io/display-name": "Development" },
            },
            "status": { "phase": "Active" },
        }));

        assert_eq!(project["kind"], "Project");
        assert_eq!(project["apiVersion"], "project.openshift.io/v1");
        // Everything else survives untouched.
        assert_eq!(project["metadata"]["name"], "dev");
        assert_eq!(
            project["metadata"]["annotations"]["openshift.io/display-name"],
            "Development"
        );
        assert_eq!(project["status"]["phase"], "Active");
    }

    #[test]
    fn rewrites_a_namespace_list_into_a_project_list() {
        let list = mapper().map_response(json!({
            "kind": "NamespaceList",
            "apiVersion": "v1",
            "metadata": { "resourceVersion": "42" },
            "items": [
                { "metadata": { "name": "dev" } },
                { "metadata": { "name": "prod" } },
            ],
        }));

        assert_eq!(list["kind"], "ProjectList");
        assert_eq!(list["apiVersion"], "project.openshift.io/v1");
        assert_eq!(list["metadata"]["resourceVersion"], "42");
        assert_eq!(list["items"][0]["kind"], "Project");
        assert_eq!(list["items"][0]["apiVersion"], "project.openshift.io/v1");
        assert_eq!(list["items"][0]["metadata"]["name"], "dev");
        assert_eq!(list["items"][1]["metadata"]["name"], "prod");
    }

    #[test]
    fn a_malformed_project_request_body_does_not_panic() {
        let route = route("POST", "/apis/project.openshift.io/v1/projectrequests");
        // `metadata` is a string, not an object: the old index-assignment would
        // panic here. It must degrade gracefully instead.
        let ns = mapper().map_request_body(
            &route,
            json!({ "kind": "ProjectRequest", "metadata": "oops", "displayName": "x" }),
        );
        assert_eq!(ns["kind"], "Namespace");
        // A non-object metadata is dropped rather than crashing the worker.
        assert!(ns["metadata"].is_object());

        // `metadata.annotations` is a string: must not panic, annotations skipped.
        let ns = mapper().map_request_body(
            &route,
            json!({
                "kind": "ProjectRequest",
                "metadata": { "name": "dev", "annotations": "oops" },
                "displayName": "x"
            }),
        );
        assert_eq!(ns["metadata"]["name"], "dev");
    }

    #[test]
    fn a_namespace_list_with_non_object_items_does_not_panic() {
        let list = mapper().map_response(json!({
            "kind": "NamespaceList",
            "apiVersion": "v1",
            "items": [ null, "x", { "metadata": { "name": "dev" } } ],
        }));
        assert_eq!(list["kind"], "ProjectList");
        // The well-formed item is still stamped; the junk ones are left as-is.
        assert_eq!(list["items"][2]["kind"], "Project");
    }

    #[test]
    fn leaves_an_error_status_untouched() {
        let status = json!({
            "kind": "Status",
            "apiVersion": "v1",
            "status": "Failure",
            "reason": "Forbidden",
            "code": 403,
        });
        assert_eq!(mapper().map_response(status.clone()), status);
    }

    #[test]
    fn rewrites_the_object_inside_a_watch_event() {
        let event = mapper().map_watch_event(json!({
            "type": "ADDED",
            "object": {
                "kind": "Namespace",
                "apiVersion": "v1",
                "metadata": { "name": "dev" },
            },
        }));

        assert_eq!(event["type"], "ADDED");
        assert_eq!(event["object"]["kind"], "Project");
        assert_eq!(event["object"]["apiVersion"], "project.openshift.io/v1");
        assert_eq!(event["object"]["metadata"]["name"], "dev");
    }

    #[test]
    fn leaves_a_bookmark_event_alone() {
        let event = json!({ "type": "BOOKMARK", "object": { "kind": "Namespace", "metadata": { "resourceVersion": "99" } } });
        let mapped = mapper().map_watch_event(event);
        assert_eq!(mapped["type"], "BOOKMARK");
        assert_eq!(mapped["object"]["metadata"]["resourceVersion"], "99");
    }
}
