//! The shapes exchanged between the proxy and a mapper.

/// What a mapper recognised in an incoming request.
///
/// A mapper produces this from the request path alone; the proxy then asks it
/// to turn it into the upstream request to actually issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualRoute {
    /// Resource plural as it appeared in the virtual path, e.g. `projects`.
    pub resource: String,
    /// Object name, when the request targets a single object.
    pub name: Option<String>,
    /// Subresource, when the request targets one, e.g. `status`.
    pub subresource: Option<String>,
    /// HTTP method of the incoming request, uppercased.
    pub method: String,
}

impl VirtualRoute {
    pub fn new(method: &str, resource: &str) -> Self {
        Self {
            resource: resource.to_string(),
            name: None,
            subresource: None,
            method: method.to_ascii_uppercase(),
        }
    }

    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    pub fn with_subresource(mut self, subresource: &str) -> Self {
        self.subresource = Some(subresource.to_string());
        self
    }
}

/// The request the proxy should send upstream in place of the virtual one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamRequest {
    /// HTTP method, uppercased.
    pub method: String,
    /// Absolute path on the target cluster, starting with `/`.
    pub path: String,
    /// Replacement request body, when the mapper had to rewrite it.
    ///
    /// `None` means "forward the client body unchanged".
    pub body: Option<Vec<u8>>,
}

impl UpstreamRequest {
    pub fn new(method: &str, path: impl Into<String>) -> Self {
        Self {
            method: method.to_ascii_uppercase(),
            path: path.into(),
            body: None,
        }
    }

    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }
}

/// Split a path into its non-empty segments.
pub fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_builders_uppercase_the_method() {
        let route = VirtualRoute::new("get", "projects")
            .with_name("dev")
            .with_subresource("status");
        assert_eq!(route.method, "GET");
        assert_eq!(route.resource, "projects");
        assert_eq!(route.name.as_deref(), Some("dev"));
        assert_eq!(route.subresource.as_deref(), Some("status"));
    }

    #[test]
    fn upstream_request_defaults_to_forwarding_the_client_body() {
        let request = UpstreamRequest::new("delete", "/api/v1/namespaces/dev");
        assert_eq!(request.method, "DELETE");
        assert_eq!(request.body, None);
        assert_eq!(
            UpstreamRequest::new("post", "/api/v1/namespaces")
                .with_body(b"{}".to_vec())
                .body,
            Some(b"{}".to_vec())
        );
    }

    #[test]
    fn segments_ignores_empty_pieces() {
        assert_eq!(
            segments("/apis/project.openshift.io/v1/projects"),
            vec!["apis", "project.openshift.io", "v1", "projects"]
        );
        assert!(segments("/").is_empty());
    }
}
