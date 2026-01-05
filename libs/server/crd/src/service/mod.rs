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
        /// URL of the external service (e.g. https://example.com)
        url: String,
    },
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
                if let Some(spec) = svc.spec {
                    if let Some(ports) = spec.ports {
                        if let Some(target_port) = port {
                            if let Some(svc_port) =
                                ports.iter().find(|p| p.port == *target_port as i32)
                            {
                                if let Some(node_port) = svc_port.node_port {
                                    Ok(format!(
                                        "https://{}:{}",
                                        spec.cluster_ip
                                            .unwrap_or(format!("{}:{}", name, target_ns)),
                                        node_port
                                    ))
                                } else {
                                    Ok(format!(
                                        "https://{}:{}",
                                        spec.cluster_ip
                                            .unwrap_or(format!("{}:{}", name, target_ns)),
                                        svc_port.port
                                    ))
                                }
                            } else {
                                Err(format!(
                                    "Port {} not found in service {}",
                                    target_port, name
                                ))
                            }
                        } else if let Some(port_name) = port_name {
                            if let Some(svc_port) = ports
                                .iter()
                                .find(|svc_port| svc_port.name.as_deref() == Some(port_name))
                            {
                                if let Some(node_port) = svc_port.node_port {
                                    Ok(format!(
                                        "https://{}:{}",
                                        spec.cluster_ip
                                            .unwrap_or(format!("{}:{}", name, target_ns)),
                                        node_port
                                    ))
                                } else {
                                    Ok(format!(
                                        "https://{}:{}",
                                        spec.cluster_ip
                                            .unwrap_or(format!("{}:{}", name, target_ns)),
                                        svc_port.port
                                    ))
                                }
                            } else {
                                Err(format!(
                                    "Port name {} not found in service {}",
                                    port_name, name
                                ))
                            }
                        } else if let Some(p) = ports.first() {
                            if let Some(node_port) = p.node_port {
                                Ok(format!(
                                    "https://{}:{}",
                                    spec.cluster_ip.unwrap_or(format!("{}:{}", name, target_ns)),
                                    node_port
                                ))
                            } else {
                                Ok(format!(
                                    "https://{}:{}",
                                    spec.cluster_ip.unwrap_or(format!("{}:{}", name, target_ns)),
                                    p.port
                                ))
                            }
                        } else {
                            Err(format!("No ports found in service {}", name))
                        }
                    } else {
                        Err(format!("No ports found in service {}", name))
                    }
                } else {
                    Err(format!("No spec found for service {}", name))
                }
            }
            Service::ExternalService { url } => Ok(url.clone()),
        }
    }
}
