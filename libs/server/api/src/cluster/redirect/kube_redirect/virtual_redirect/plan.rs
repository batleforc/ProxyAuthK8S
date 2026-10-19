//! Deciding, from the request path and method alone, what a virtual API does
//! with a request — and classifying a watch, which cannot be buffered.

use serde_json::Value;
use virtual_api::MapperRegistry;
use virtual_api::discovery::{
    DiscoveryRequest, api_group_response, api_resource_list_response, classify,
};

/// What the proxy will do with a request a virtual API claims.
pub(crate) enum VirtualPlan {
    /// Answer from the mapper, no upstream call at all.
    Direct(Value),
    /// Fetch `/apis` upstream, then merge the virtual groups into it.
    MergeApiGroups,
    /// Rewrite the request onto `upstream_path` and translate the response.
    Mapped { upstream_path: String },
    /// The path is a known virtual resource but the method is not one it
    /// supports; answer `405` (with this `Allow` header) without any upstream call.
    MethodNotAllowed { allow: String },
}

impl VirtualPlan {
    /// The path actually reached on the target cluster, when there is one.
    ///
    /// Used to run the resource allow-list against what is really accessed, not
    /// only against the virtual path the client typed.
    pub(crate) fn upstream_path(&self) -> Option<&str> {
        match self {
            VirtualPlan::Direct(_) => None,
            VirtualPlan::MergeApiGroups => Some("/apis"),
            VirtualPlan::Mapped { upstream_path } => Some(upstream_path),
            VirtualPlan::MethodNotAllowed { .. } => None,
        }
    }
}

/// Decide what a virtual API does with `path`, if anything.
///
/// `path` must be free of its query string.
pub(crate) fn plan(registry: &MapperRegistry, path: &str, method: &str) -> Option<VirtualPlan> {
    if registry.is_empty() {
        return None;
    }

    if method.eq_ignore_ascii_case("GET") {
        match classify(registry, path) {
            Some(DiscoveryRequest::ApiGroupList) => return Some(VirtualPlan::MergeApiGroups),
            Some(DiscoveryRequest::ApiGroup(group)) => {
                let mapper = registry.find_group(&group)?;
                return Some(VirtualPlan::Direct(api_group_response(mapper)));
            }
            Some(DiscoveryRequest::ApiResourceList(group, version)) => {
                let mapper = registry.find_group_version(&group, &version)?;
                return Some(VirtualPlan::Direct(api_resource_list_response(mapper)));
            }
            None => {}
        }
    }

    let (mapper, mut route) = registry.resolve(path)?;
    route.method = method.to_ascii_uppercase();
    if let Some(allow) = mapper.method_not_allowed(&route) {
        return Some(VirtualPlan::MethodNotAllowed { allow });
    }
    Some(VirtualPlan::Mapped {
        upstream_path: mapper.map_request(&route).path,
    })
}

/// `true` when the request asks for a watch stream.
pub(super) fn is_watch(query_string: &str) -> bool {
    query_string
        .split('&')
        .any(|param| matches!(param, "watch=true" | "watch=1"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crd::virtual_api::VirtualApiKind;

    fn registry() -> MapperRegistry {
        MapperRegistry::from_kinds(&[VirtualApiKind::OpenShiftProject])
    }

    #[test]
    fn an_empty_registry_plans_nothing() {
        assert!(plan(&MapperRegistry::new(), "/apis", "GET").is_none());
    }

    #[test]
    fn an_unsupported_verb_plans_a_405() {
        let plan = plan(
            &registry(),
            "/apis/project.openshift.io/v1/projectrequests",
            "DELETE",
        )
        .expect("a known virtual path should still be planned");
        match &plan {
            VirtualPlan::MethodNotAllowed { allow } => assert_eq!(allow, "GET, POST"),
            _ => panic!("DELETE on projectrequests must be refused"),
        }
        assert_eq!(plan.upstream_path(), None);
    }

    #[test]
    fn discovery_of_a_served_group_is_answered_locally() {
        let plan = plan(&registry(), "/apis/project.openshift.io", "GET")
            .expect("group discovery should be planned");
        assert!(matches!(plan, VirtualPlan::Direct(_)));
        assert_eq!(plan.upstream_path(), None);
    }

    #[test]
    fn the_group_list_is_merged_with_the_cluster() {
        let plan = plan(&registry(), "/apis", "GET").expect("group list should be planned");
        assert!(matches!(plan, VirtualPlan::MergeApiGroups));
        assert_eq!(plan.upstream_path(), Some("/apis"));
    }

    #[test]
    fn a_resource_request_is_mapped_onto_the_real_api() {
        let plan = plan(
            &registry(),
            "/apis/project.openshift.io/v1/projects/dev",
            "GET",
        )
        .expect("project should be planned");
        assert_eq!(plan.upstream_path(), Some("/api/v1/namespaces/dev"));
    }

    #[test]
    fn a_project_request_maps_onto_a_namespace_creation() {
        let plan = plan(
            &registry(),
            "/apis/project.openshift.io/v1/projectrequests",
            "POST",
        )
        .expect("project request should be planned");
        assert_eq!(plan.upstream_path(), Some("/api/v1/namespaces"));
    }

    #[test]
    fn real_api_paths_are_left_to_the_standard_proxy() {
        let registry = registry();
        assert!(plan(&registry, "/api/v1/namespaces", "GET").is_none());
        assert!(plan(&registry, "/apis/apps/v1/deployments", "GET").is_none());
        // Discovery only applies to GET.
        assert!(plan(&registry, "/apis/project.openshift.io", "POST").is_none());
    }

    #[test]
    fn watch_detection_only_accepts_the_real_parameter() {
        assert!(is_watch("watch=true"));
        assert!(is_watch("resourceVersion=1&watch=1"));
        assert!(!is_watch("watch=false"));
        assert!(!is_watch("allowWatchBookmarks=true"));
        assert!(!is_watch(""));
    }
}
