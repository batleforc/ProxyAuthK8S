use kube::{Api, Client};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum Service {
    /// Kubernetes service
    KubernetesService {
        /// Name of the service
        name: String,
        /// If not set, will use the resource namespace
        namespace: Option<String>,
        /// Port of the service
        port: Option<u16>,
        /// Port name of the service
        port_name: Option<String>,
    },
    /// External service
    ExternalService {
        /// URL of the external service (e.g. <https://example.com>)
        url: String,
    },
}

/// Resolve the host to dial for a Kubernetes service.
///
/// A `ClusterIP` is used when present, but headless services report
/// `spec.clusterIP == "None"` (and `ExternalName` / not-yet-assigned services
/// report an empty string). In those cases we fall back to the stable in-cluster
/// DNS name `{name}.{namespace}.svc`, which resolves via the pod search domains.
/// The previous fallback (`{name}:{namespace}`) produced a second colon in the
/// URL authority (`https://name:ns:port`) and was always invalid.
fn service_host(cluster_ip: Option<&str>, name: &str, namespace: &str) -> String {
    match cluster_ip {
        Some(ip) if !ip.is_empty() && !ip.eq_ignore_ascii_case("None") => ip.to_string(),
        _ => format!("{name}.{namespace}.svc"),
    }
}

/// Prefer the nodePort when set (the service is reached from off-cluster via the
/// node), otherwise the service port.
fn dial_port(svc_port: &k8s_openapi::api::core::v1::ServicePort) -> i32 {
    svc_port.node_port.unwrap_or(svc_port.port)
}

impl Service {
    /// Get the URL to call for the service
    pub async fn url_to_call(&self, client: Client, main_ns: String) -> Result<String, String> {
        match self {
            Service::KubernetesService {
                name,
                namespace,
                port,
                port_name,
            } => {
                let target_ns = namespace.as_deref().unwrap_or(main_ns.as_str());
                let services: Api<k8s_openapi::api::core::v1::Service> =
                    Api::namespaced(client, target_ns);
                let svc = services.get(name).await.map_err(|e| e.to_string())?;
                let spec = svc
                    .spec
                    .ok_or_else(|| format!("No spec found for service {name}"))?;
                let ports = spec
                    .ports
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| format!("No ports found in service {name}"))?;

                let svc_port = if let Some(target_port) = port {
                    ports
                        .iter()
                        .find(|p| p.port == i32::from(*target_port))
                        .ok_or_else(|| format!("Port {target_port} not found in service {name}"))?
                } else if let Some(port_name) = port_name {
                    ports
                        .iter()
                        .find(|p| p.name.as_deref() == Some(port_name))
                        .ok_or_else(|| {
                            format!("Port name {port_name} not found in service {name}")
                        })?
                } else {
                    // Safe: `ports` is guaranteed non-empty above.
                    &ports[0]
                };

                let host = service_host(spec.cluster_ip.as_deref(), name, target_ns);
                Ok(format!("https://{}:{}", host, dial_port(svc_port)))
            }
            Service::ExternalService { url } => Ok(url.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_ip_used_when_present() {
        assert_eq!(service_host(Some("10.0.0.5"), "svc", "ns"), "10.0.0.5");
    }

    #[test]
    fn headless_falls_back_to_dns_name() {
        // Headless services report the literal string "None".
        assert_eq!(service_host(Some("None"), "svc", "ns"), "svc.ns.svc");
        assert_eq!(service_host(Some("none"), "svc", "ns"), "svc.ns.svc");
    }

    #[test]
    fn missing_or_empty_cluster_ip_falls_back_to_dns_name() {
        assert_eq!(service_host(None, "svc", "ns"), "svc.ns.svc");
        assert_eq!(service_host(Some(""), "svc", "ns"), "svc.ns.svc");
    }
}
