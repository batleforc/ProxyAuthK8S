//! Discovery payloads for the virtual APIs.
//!
//! `kubectl` refuses to talk to a resource it cannot discover, so a virtual API
//! is only usable if `/apis`, `/apis/{group}` and `/apis/{group}/{version}`
//! answer for it. The first is a merge with what the real cluster returns; the
//! other two the proxy answers on its own.
//!
//! `/openapi/v2` and `/openapi/v3` are deliberately left alone: clients tolerate
//! a resource missing from the `OpenAPI` document (they lose client-side field
//! validation and `kubectl explain` for it, nothing more).

use serde_json::{json, Value};

use crate::route::segments;
use crate::{MapperRegistry, VirtualApiMapper};

/// A discovery request the proxy can answer without the cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryRequest {
    /// `GET /apis`: forward upstream, then merge the virtual groups in.
    ApiGroupList,
    /// `GET /apis/{group}`: answered directly.
    ApiGroup(String),
    /// `GET /apis/{group}/{version}`: answered directly.
    ApiResourceList(String, String),
}

/// Classify a path as a discovery request served by `registry`, if it is one.
///
/// Only `GET` requests can be discovery; the caller checks the method.
#[must_use]
pub fn classify(registry: &MapperRegistry, path: &str) -> Option<DiscoveryRequest> {
    if registry.is_empty() {
        return None;
    }

    match segments(path).as_slice() {
        ["apis"] => Some(DiscoveryRequest::ApiGroupList),
        ["apis", group] => registry
            .find_group(group)
            .map(|_| DiscoveryRequest::ApiGroup((*group).to_string())),
        ["apis", group, version] => registry.find_group_version(group, version).map(|_| {
            DiscoveryRequest::ApiResourceList((*group).to_string(), (*version).to_string())
        }),
        _ => None,
    }
}

/// The `APIGroup` entry describing a mapper.
pub fn api_group(mapper: &dyn VirtualApiMapper) -> Value {
    let (group, version) = mapper.group_version();
    let group_version = json!({
        "groupVersion": format!("{}/{}", group, version),
        "version": version,
    });

    json!({
        "name": group,
        "versions": [group_version],
        "preferredVersion": group_version,
    })
}

/// Full body for `GET /apis/{group}`.
pub fn api_group_response(mapper: &dyn VirtualApiMapper) -> Value {
    let mut body = api_group(mapper);
    body["kind"] = json!("APIGroup");
    body["apiVersion"] = json!("v1");
    body
}

/// Full body for `GET /apis/{group}/{version}`.
pub fn api_resource_list_response(mapper: &dyn VirtualApiMapper) -> Value {
    let resources = mapper.api_resources();
    let mut body = serde_json::to_value(&resources).unwrap_or_else(|_| json!({}));
    body["kind"] = json!("APIResourceList");
    body["apiVersion"] = json!("v1");
    body["groupVersion"] = json!(resources.group_version);
    body
}

/// Merge the virtual groups into the cluster's own `APIGroupList`.
///
/// A group the cluster already serves wins: the proxy must never shadow a real
/// API with a synthesised one.
pub fn merge_api_group_list(registry: &MapperRegistry, mut upstream: Value) -> Value {
    let Some(groups) = upstream.get_mut("groups").and_then(Value::as_array_mut) else {
        // Not a shape we recognise (an error Status, most likely): leave it be.
        return upstream;
    };

    for mapper in registry.iter() {
        let (group, _) = mapper.group_version();
        let already_served = groups
            .iter()
            .any(|entry| entry.get("name").and_then(Value::as_str) == Some(group));
        if already_served {
            tracing::debug!(
                group,
                "cluster already serves this API group; not shadowing it with the virtual one"
            );
            continue;
        }
        groups.push(api_group(mapper));
    }

    upstream
}

#[cfg(test)]
mod tests {
    use super::*;
    use crd::virtual_api::VirtualApiKind;

    fn registry() -> MapperRegistry {
        MapperRegistry::from_kinds(&[VirtualApiKind::OpenShiftProject])
    }

    #[test]
    fn an_empty_registry_classifies_nothing() {
        assert_eq!(classify(&MapperRegistry::new(), "/apis"), None);
    }

    #[test]
    fn classifies_the_three_discovery_shapes() {
        let registry = registry();
        assert_eq!(
            classify(&registry, "/apis"),
            Some(DiscoveryRequest::ApiGroupList)
        );
        assert_eq!(
            classify(&registry, "/apis/project.openshift.io"),
            Some(DiscoveryRequest::ApiGroup(
                "project.openshift.io".to_string()
            ))
        );
        assert_eq!(
            classify(&registry, "/apis/project.openshift.io/v1"),
            Some(DiscoveryRequest::ApiResourceList(
                "project.openshift.io".to_string(),
                "v1".to_string()
            ))
        );
    }

    #[test]
    fn ignores_discovery_for_groups_it_does_not_serve() {
        let registry = registry();
        assert_eq!(classify(&registry, "/apis/apps"), None);
        assert_eq!(classify(&registry, "/apis/apps/v1"), None);
        assert_eq!(classify(&registry, "/apis/project.openshift.io/v2"), None);
        // Resource paths are the mappers' business, not discovery's.
        assert_eq!(
            classify(&registry, "/apis/project.openshift.io/v1/projects"),
            None
        );
        assert_eq!(classify(&registry, "/api/v1"), None);
    }

    #[test]
    fn api_group_response_is_a_well_formed_api_group() {
        let registry = registry();
        let mapper = registry.find_group("project.openshift.io").unwrap();
        let body = api_group_response(mapper);

        assert_eq!(body["kind"], "APIGroup");
        assert_eq!(body["apiVersion"], "v1");
        assert_eq!(body["name"], "project.openshift.io");
        assert_eq!(
            body["versions"][0]["groupVersion"],
            "project.openshift.io/v1"
        );
        assert_eq!(body["versions"][0]["version"], "v1");
        assert_eq!(
            body["preferredVersion"]["groupVersion"],
            "project.openshift.io/v1"
        );
    }

    #[test]
    fn api_resource_list_response_is_a_well_formed_resource_list() {
        let registry = registry();
        let mapper = registry
            .find_group_version("project.openshift.io", "v1")
            .unwrap();
        let body = api_resource_list_response(mapper);

        assert_eq!(body["kind"], "APIResourceList");
        assert_eq!(body["apiVersion"], "v1");
        assert_eq!(body["groupVersion"], "project.openshift.io/v1");
        assert_eq!(body["resources"][0]["name"], "projects");
        assert_eq!(body["resources"][0]["kind"], "Project");
        assert_eq!(body["resources"][0]["namespaced"], false);
    }

    #[test]
    fn merging_appends_the_virtual_group() {
        let upstream = json!({
            "kind": "APIGroupList",
            "apiVersion": "v1",
            "groups": [{ "name": "apps", "versions": [{ "groupVersion": "apps/v1", "version": "v1" }] }],
        });

        let merged = merge_api_group_list(&registry(), upstream);
        let groups = merged["groups"]
            .as_array()
            .expect("groups should be a list");

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0]["name"], "apps");
        assert_eq!(groups[1]["name"], "project.openshift.io");
    }

    #[test]
    fn merging_never_shadows_a_group_the_cluster_already_serves() {
        // A real OpenShift cluster behind the proxy: its own group must win.
        let upstream = json!({
            "kind": "APIGroupList",
            "apiVersion": "v1",
            "groups": [{
                "name": "project.openshift.io",
                "versions": [{ "groupVersion": "project.openshift.io/v1", "version": "v1" }],
                "serverAddressByClientCIDRs": [],
            }],
        });

        let merged = merge_api_group_list(&registry(), upstream);
        let groups = merged["groups"]
            .as_array()
            .expect("groups should be a list");

        assert_eq!(groups.len(), 1);
        assert!(groups[0].get("serverAddressByClientCIDRs").is_some());
    }

    #[test]
    fn merging_leaves_an_unexpected_body_alone() {
        let status = json!({ "kind": "Status", "code": 403 });
        assert_eq!(merge_api_group_list(&registry(), status.clone()), status);
    }
}
