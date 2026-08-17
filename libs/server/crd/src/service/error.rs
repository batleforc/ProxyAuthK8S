//! Error type for resolving a [`super::Service`] to a URL to dial.

/// Failure while resolving a Kubernetes [`super::Service`] to its callable URL.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// The Service object could not be read from the API server.
    #[error("failed to read service {name}: {source}")]
    Read {
        name: String,
        #[source]
        source: kube::Error,
    },
    /// The Service has no `spec`.
    #[error("no spec found for service {name}")]
    NoSpec { name: String },
    /// The Service exposes no ports.
    #[error("no ports found in service {name}")]
    NoPorts { name: String },
    /// The requested numeric port is not exposed by the Service.
    #[error("port {port} not found in service {name}")]
    PortNotFound { port: u16, name: String },
    /// The requested named port is not exposed by the Service.
    #[error("port name {port_name} not found in service {name}")]
    PortNameNotFound { port_name: String, name: String },
}
