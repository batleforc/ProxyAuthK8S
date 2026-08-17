//! `project.openshift.io/v1` Projects mapped onto core Namespaces.
//!
//! An `OpenShift` Project *is* a Namespace with a different name and a couple of
//! annotations, which is why this mapping is faithful enough to be useful:
//!
//! | virtual                                              | upstream                        |
//! |------------------------------------------------------|---------------------------------|
//! | `GET/WATCH /apis/project.openshift.io/v1/projects`   | `GET/WATCH /api/v1/namespaces`  |
//! | `GET/DELETE .../projects/{name}`                     | `.../namespaces/{name}`         |
//! | `POST .../projectrequests`                           | `POST /api/v1/namespaces`       |
//!
//! Known limitation: `OpenShift`'s `LIST projects` returns only the projects the
//! caller can see, because the apiserver filters them per user. `LIST
//! namespaces` has no such behaviour and requires cluster-wide list rights, so
//! a user without them gets a 403 here where `OpenShift` would have returned a
//! (possibly empty) list. See the roadmap entry about per-namespace
//! `SelfSubjectAccessReview` fallback.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::{APIResource, APIResourceList};
use serde_json::{Value, json};

use crate::VirtualApiMapper;
use crate::route::{UpstreamRequest, VirtualRoute, segments};

pub const GROUP: &str = "project.openshift.io";
pub const VERSION: &str = "v1";
pub const PROJECT_KIND: &str = "Project";
pub const PROJECT_LIST_KIND: &str = "ProjectList";
pub const PROJECTS_RESOURCE: &str = "projects";
pub const PROJECT_REQUESTS_RESOURCE: &str = "projectrequests";

const NAMESPACE_KIND: &str = "Namespace";
const NAMESPACE_LIST_KIND: &str = "NamespaceList";
const CORE_API_VERSION: &str = "v1";
const NAMESPACES_PATH: &str = "/api/v1/namespaces";

const DISPLAY_NAME_ANNOTATION: &str = "openshift.io/display-name";
const DESCRIPTION_ANNOTATION: &str = "openshift.io/description";

#[derive(Debug, Default, Clone, Copy)]
pub struct OpenShiftProjectMapper;

impl OpenShiftProjectMapper {
    /// `Namespace` -> `Project`, `NamespaceList` -> `ProjectList`.
    ///
    /// Anything else (a `Status` error body, most notably) is returned as-is:
    /// the client must still see the real apiserver error.
    fn namespace_to_project(&self, mut body: Value) -> Value {
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
    fn project_request_to_namespace(&self, body: Value) -> Value {
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

    fn projects_path(name: Option<&str>) -> String {
        match name {
            Some(name) => format!("{NAMESPACES_PATH}/{name}"),
            None => NAMESPACES_PATH.to_string(),
        }
    }
}

impl VirtualApiMapper for OpenShiftProjectMapper {
    fn group_version(&self) -> (&str, &str) {
        (GROUP, VERSION)
    }

    fn api_resources(&self) -> APIResourceList {
        APIResourceList {
            group_version: format!("{GROUP}/{VERSION}"),
            resources: vec![
                APIResource {
                    name: PROJECTS_RESOURCE.to_string(),
                    singular_name: "project".to_string(),
                    namespaced: false,
                    kind: PROJECT_KIND.to_string(),
                    verbs: vec![
                        "get".to_string(),
                        "list".to_string(),
                        "watch".to_string(),
                        "delete".to_string(),
                    ],
                    short_names: Some(vec!["proj".to_string()]),
                    ..Default::default()
                },
                APIResource {
                    name: PROJECT_REQUESTS_RESOURCE.to_string(),
                    singular_name: "projectrequest".to_string(),
                    namespaced: false,
                    kind: "ProjectRequest".to_string(),
                    verbs: vec!["create".to_string(), "list".to_string()],
                    ..Default::default()
                },
            ],
        }
    }

    fn matches(&self, path: &str) -> Option<VirtualRoute> {
        let segments = segments(path);
        // /apis/{group}/{version}/{resource}[/{name}[/{subresource}]]
        match segments.as_slice() {
            ["apis", GROUP, VERSION, resource, rest @ ..] => {
                let resource = *resource;
                if resource != PROJECTS_RESOURCE && resource != PROJECT_REQUESTS_RESOURCE {
                    return None;
                }
                // `matches` only sees the path; the caller fills in the method

                let mut route = VirtualRoute::new("", resource);
                if let Some(name) = rest.first() {
                    route = route.with_name(name);
                }
                if let Some(subresource) = rest.get(1) {
                    route = route.with_subresource(subresource);
                }
                Some(route)
            }
            _ => None,
        }
    }

    fn method_not_allowed(&self, route: &VirtualRoute) -> Option<String> {
        // Verbs each resource actually supports, expressed as HTTP methods (see
        // `api_resources`). Anything else must not be mapped onto a namespace
        // operation: e.g. `DELETE projectrequests` would otherwise become a
        // namespace deletecollection.
        let allowed: &[&str] = match route.resource.as_str() {
            // projectrequests: create (POST), list (GET).
            PROJECT_REQUESTS_RESOURCE => &["GET", "POST"],
            // projects: get/list/watch (GET), delete (DELETE).
            PROJECTS_RESOURCE => &["GET", "DELETE"],
            _ => return None,
        };
        if allowed.contains(&route.method.as_str()) {
            None
        } else {
            Some(allowed.join(", "))
        }
    }

    fn map_request(&self, route: &VirtualRoute) -> UpstreamRequest {
        if route.resource == PROJECT_REQUESTS_RESOURCE {
            // A project request creates a namespace; listing them is not
            // something a vanilla cluster can answer, so it maps to the
            // namespace list and comes back as a ProjectList.
            return UpstreamRequest::new(&route.method, Self::projects_path(None));
        }

        let mut path = Self::projects_path(route.name.as_deref());
        if let Some(subresource) = &route.subresource {
            path.push('/');
            path.push_str(subresource);
        }
        UpstreamRequest::new(&route.method, path)
    }

    fn map_request_body(&self, route: &VirtualRoute, body: Value) -> Value {
        match body.get("kind").and_then(Value::as_str) {
            Some("ProjectRequest" | PROJECT_KIND) => self.project_request_to_namespace(body),
            // An unrecognised body on a projectrequests create is still meant to
            // become a namespace; anything else goes through untouched.
            _ if route.resource == PROJECT_REQUESTS_RESOURCE => {
                self.project_request_to_namespace(body)
            }
            _ => body,
        }
    }

    fn map_response(&self, body: Value) -> Value {
        self.namespace_to_project(body)
    }

    fn map_watch_event(&self, mut event: Value) -> Value {
        if let Some(object) = event.get_mut("object") {
            let mapped = self.namespace_to_project(object.take());
            *object = mapped;
        }
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapper() -> OpenShiftProjectMapper {
        OpenShiftProjectMapper
    }

    fn route(method: &str, path: &str) -> VirtualRoute {
        let mut route = mapper().matches(path).expect("path should be recognised");
        route.method = method.to_ascii_uppercase();
        route
    }

    #[test]
    fn recognises_the_project_paths() {
        let mapper = mapper();
        assert_eq!(
            mapper.matches("/apis/project.openshift.io/v1/projects"),
            Some(VirtualRoute::new("", "projects"))
        );
        assert_eq!(
            mapper.matches("/apis/project.openshift.io/v1/projects/dev"),
            Some(VirtualRoute::new("", "projects").with_name("dev"))
        );
        assert_eq!(
            mapper.matches("/apis/project.openshift.io/v1/projectrequests"),
            Some(VirtualRoute::new("", "projectrequests"))
        );
    }

    #[test]
    fn ignores_everything_else() {
        let mapper = mapper();
        assert!(mapper.matches("/api/v1/namespaces").is_none());
        assert!(mapper.matches("/apis/apps/v1/deployments").is_none());
        assert!(
            mapper
                .matches("/apis/project.openshift.io/v1/projecthelpers")
                .is_none()
        );
        assert!(
            mapper
                .matches("/apis/project.openshift.io/v2/projects")
                .is_none()
        );
        assert!(mapper.matches("/apis/project.openshift.io/v1").is_none());
    }

    #[test]
    fn maps_a_list_onto_the_namespace_collection() {
        let request = mapper().map_request(&route("GET", "/apis/project.openshift.io/v1/projects"));
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/api/v1/namespaces");
        assert_eq!(request.body, None);
    }

    #[test]
    fn maps_a_single_project_onto_its_namespace() {
        let request =
            mapper().map_request(&route("GET", "/apis/project.openshift.io/v1/projects/dev"));
        assert_eq!(request.path, "/api/v1/namespaces/dev");

        let request = mapper().map_request(&route(
            "DELETE",
            "/apis/project.openshift.io/v1/projects/dev",
        ));
        assert_eq!(request.method, "DELETE");
        assert_eq!(request.path, "/api/v1/namespaces/dev");
    }

    #[test]
    fn maps_a_project_request_onto_a_namespace_creation() {
        let request = mapper().map_request(&route(
            "POST",
            "/apis/project.openshift.io/v1/projectrequests",
        ));
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/v1/namespaces");
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
    fn unsupported_verbs_are_refused_not_mapped() {
        let mapper = mapper();
        // projectrequests supports create (POST) and list (GET) only.
        assert_eq!(
            mapper.method_not_allowed(&route(
                "DELETE",
                "/apis/project.openshift.io/v1/projectrequests"
            )),
            Some("GET, POST".to_string())
        );
        assert_eq!(
            mapper.method_not_allowed(&route(
                "PUT",
                "/apis/project.openshift.io/v1/projectrequests"
            )),
            Some("GET, POST".to_string())
        );
        assert!(
            mapper
                .method_not_allowed(&route(
                    "POST",
                    "/apis/project.openshift.io/v1/projectrequests"
                ))
                .is_none()
        );
        // projects supports get/list/watch (GET) and delete (DELETE).
        assert!(
            mapper
                .method_not_allowed(&route(
                    "DELETE",
                    "/apis/project.openshift.io/v1/projects/dev"
                ))
                .is_none()
        );
        assert_eq!(
            mapper.method_not_allowed(&route("POST", "/apis/project.openshift.io/v1/projects")),
            Some("GET, DELETE".to_string())
        );
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

    #[test]
    fn advertises_its_resources_in_discovery() {
        let resources = mapper().api_resources();
        assert_eq!(resources.group_version, "project.openshift.io/v1");

        let names: Vec<&str> = resources
            .resources
            .iter()
            .map(|resource| resource.name.as_str())
            .collect();
        assert_eq!(names, vec!["projects", "projectrequests"]);

        let projects = &resources.resources[0];
        assert_eq!(projects.kind, "Project");
        assert!(!projects.namespaced);
        assert!(projects.verbs.contains(&"watch".to_string()));
    }
}
