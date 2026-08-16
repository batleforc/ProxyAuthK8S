//! Virtual Kubernetes APIs.
//!
//! A *virtual API* is an API the target cluster does not serve. The proxy
//! answers discovery for it and rewrites requests onto an API the cluster does
//! serve, then rewrites the responses back. The first mapper translates
//! `project.openshift.io/v1` Projects onto core Namespaces, so an `oc`-flavoured
//! client can talk to a vanilla cluster.
//!
//! Everything here is pure translation: no I/O, no cluster access. That keeps
//! it exhaustively unit-testable and keeps the proxy hot path to a registry
//! lookup when no mapper is enabled.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::APIResourceList;

pub mod discovery;
pub mod openshift;
pub mod route;

pub use route::{UpstreamRequest, VirtualRoute, segments};

use crd::virtual_api::VirtualApiKind;

/// Translation between a virtual API and a real one.
///
/// This is a deliberate extension point: the trait plus [`VirtualApiKind`]
/// registry exist so new virtual APIs (beyond the OpenShift Projects mapper)
/// can be added by implementing this trait and registering a kind, without
/// touching the proxy hot path. It is intentionally kept even while a single
/// mapper is implemented.
pub trait VirtualApiMapper: Send + Sync {
    /// The group and version this mapper serves, e.g. `("project.openshift.io", "v1")`.
    fn group_version(&self) -> (&str, &str);

    /// The discovery payload for `GET /apis/{group}/{version}`.
    fn api_resources(&self) -> APIResourceList;

    /// Recognise a request path, which must already be free of its query string.
    fn matches(&self, path: &str) -> Option<VirtualRoute>;

    /// The upstream request to issue in place of the virtual one.
    fn map_request(&self, route: &VirtualRoute) -> UpstreamRequest;

    /// When `route.method` is not a verb this resource supports, the `Allow`
    /// header value listing the methods that are (so the proxy can answer `405`
    /// without forwarding). `None` — the default — means the method is allowed.
    ///
    /// This prevents a verb from being silently mapped onto an unrelated, more
    /// destructive upstream operation (e.g. `DELETE` on a create/list-only
    /// resource landing on a namespace *deletecollection*).
    fn method_not_allowed(&self, _route: &VirtualRoute) -> Option<String> {
        None
    }

    /// Rewrite an upstream response body into the virtual API's shape.
    fn map_response(&self, body: serde_json::Value) -> serde_json::Value;

    /// Rewrite a single watch event.
    fn map_watch_event(&self, event: serde_json::Value) -> serde_json::Value;

    /// Rewrite a client-supplied request body, when the mapper needs to.
    ///
    /// The default forwards the body unchanged.
    fn map_request_body(
        &self,
        _route: &VirtualRoute,
        body: serde_json::Value,
    ) -> serde_json::Value {
        body
    }

    /// `apiVersion` string of the virtual API, e.g. `project.openshift.io/v1`.
    fn api_version(&self) -> String {
        let (group, version) = self.group_version();
        format!("{group}/{version}")
    }
}

/// The mappers enabled on a cluster.
///
/// Lookup is a linear scan over a handful of entries; when nothing is enabled
/// the registry is empty and `resolve` is a single length check, so a cluster
/// without virtual APIs pays nothing.
#[derive(Default)]
pub struct MapperRegistry {
    mappers: Vec<Box<dyn VirtualApiMapper>>,
}

impl MapperRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the registry for the kinds enabled on a cluster.
    #[must_use]
    pub fn from_kinds(kinds: &[VirtualApiKind]) -> Self {
        let mut registry = Self::new();
        for kind in kinds {
            registry.mappers.push(build_mapper(*kind));
        }
        registry
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mappers.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.mappers.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn VirtualApiMapper> {
        self.mappers.iter().map(std::convert::AsRef::as_ref)
    }

    /// Find the mapper that recognises `path`, if any.
    #[must_use]
    pub fn resolve(&self, path: &str) -> Option<(&dyn VirtualApiMapper, VirtualRoute)> {
        self.mappers
            .iter()
            .find_map(|mapper| mapper.matches(path).map(|route| (mapper.as_ref(), route)))
    }

    /// The mapper serving `group`/`version`, if any.
    #[must_use]
    pub fn find_group_version(&self, group: &str, version: &str) -> Option<&dyn VirtualApiMapper> {
        self.iter()
            .find(|mapper| mapper.group_version() == (group, version))
    }

    /// The mapper serving `group`, whatever the version.
    #[must_use]
    pub fn find_group(&self, group: &str) -> Option<&dyn VirtualApiMapper> {
        self.iter().find(|mapper| mapper.group_version().0 == group)
    }
}

fn build_mapper(kind: VirtualApiKind) -> Box<dyn VirtualApiMapper> {
    match kind {
        VirtualApiKind::OpenShiftProject => Box::new(openshift::project::OpenShiftProjectMapper),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_registry_resolves_nothing() {
        let registry = MapperRegistry::new();
        assert!(registry.is_empty());
        assert!(
            registry
                .resolve("/apis/project.openshift.io/v1/projects")
                .is_none()
        );
    }

    #[test]
    fn a_registry_resolves_its_enabled_kinds() {
        let registry = MapperRegistry::from_kinds(&[VirtualApiKind::OpenShiftProject]);
        assert_eq!(registry.len(), 1);

        let (mapper, route) = registry
            .resolve("/apis/project.openshift.io/v1/projects")
            .expect("projects should be recognised");
        assert_eq!(mapper.group_version(), ("project.openshift.io", "v1"));
        assert_eq!(route.resource, "projects");
    }

    #[test]
    fn a_registry_ignores_paths_of_the_real_api() {
        let registry = MapperRegistry::from_kinds(&[VirtualApiKind::OpenShiftProject]);
        assert!(registry.resolve("/api/v1/namespaces").is_none());
        assert!(registry.resolve("/apis/apps/v1/deployments").is_none());
    }

    #[test]
    fn group_lookups_find_the_mapper() {
        let registry = MapperRegistry::from_kinds(&[VirtualApiKind::OpenShiftProject]);
        assert!(
            registry
                .find_group_version("project.openshift.io", "v1")
                .is_some()
        );
        assert!(
            registry
                .find_group_version("project.openshift.io", "v2")
                .is_none()
        );
        assert!(registry.find_group("project.openshift.io").is_some());
        assert!(registry.find_group("apps").is_none());
    }
}
