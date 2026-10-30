//! Where a `FrontProxy` cluster's client certificate lives (RFC 0014,
//! decisions 49, 56 and 57): a `kubernetes.io/tls` Secret in the proxy's own
//! namespace, named after the cluster's `ns/name`, labelled as the proxy's
//! and bound by an annotation to the target it was issued for. Whatever PKI
//! the admin uses writes it; the proxy only reads it.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Secret;
use kube::{Client, ResourceExt as _};

use crate::ProxyKubeApi;
use crate::service::Service;

/// The label every Secret the proxy reads in its namespace carries.
pub const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
pub const MANAGED_BY_VALUE: &str = "proxyauthk8s";
/// The target the certificate was issued for, as [`canonical_target`] writes it.
pub const TARGET_ANNOTATION: &str = "proxyauth.weebo.si.rs/front-proxy-target";

/// Why the certificate of a `FrontProxy` cluster cannot be used. The messages
/// name the Secret and what to do, never its content.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrontProxyCertError {
    #[error("the Secret name proxyauth-frontproxy-<ns>.<name> is longer than 253 characters ({0})")]
    NameTooLong(usize),
    #[error(
        "Secret {namespace}/{name} is missing or unreadable: the proxy administrator creates it (kubernetes.io/tls, label {MANAGED_BY_LABEL}={MANAGED_BY_VALUE})"
    )]
    Missing { namespace: String, name: String },
    #[error(
        "Secret {namespace}/{name} lacks the label {MANAGED_BY_LABEL}={MANAGED_BY_VALUE}: it is never used until the proxy administrator labels it"
    )]
    NotOwned { namespace: String, name: String },
    #[error("Secret {namespace}/{name} has no {key}")]
    MissingKey {
        namespace: String,
        name: String,
        key: &'static str,
    },
    #[error(
        "Secret {namespace}/{name} was issued for {found:?}, this cluster targets {expected:?}: set its {TARGET_ANNOTATION} annotation once the certificate is meant for this target"
    )]
    TargetMismatch {
        namespace: String,
        name: String,
        expected: String,
        found: Option<String>,
    },
}

impl FrontProxyCertError {
    /// The `Ready` reason of RFC 0014 §4.3.
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NameTooLong(_) => "ClientCertNameTooLong",
            Self::TargetMismatch { .. } => "ClientCertTargetMismatch",
            Self::Missing { .. } | Self::NotOwned { .. } | Self::MissingKey { .. } => {
                "ClientCertMissing"
            }
        }
    }
}

/// `proxyauth-frontproxy-<ns>.<name>`: unambiguous, a namespace name never
/// holding a dot, and computable before the cluster exists.
///
/// # Errors
///
/// [`FrontProxyCertError::NameTooLong`] past the 253 characters of a name.
pub fn secret_name(proxy: &ProxyKubeApi) -> Result<String, FrontProxyCertError> {
    let name = format!(
        "proxyauth-frontproxy-{}.{}",
        proxy.namespace().unwrap_or_default(),
        proxy.name_any()
    );
    if name.len() > 253 {
        return Err(FrontProxyCertError::NameTooLong(name.len()));
    }
    Ok(name)
}

/// The target a `FrontProxy` certificate is bound to, as the admin writes it
/// in [`TARGET_ANNOTATION`] and the status shows it (`clientCertTarget`).
#[must_use]
pub fn canonical_target(proxy: &ProxyKubeApi) -> String {
    match &proxy.spec.service {
        Service::ExternalService { url } => format!("external:{}", url.trim_end_matches('/')),
        Service::KubernetesService {
            name,
            namespace,
            port,
            port_name,
        } => {
            let namespace = namespace
                .clone()
                .unwrap_or_else(|| proxy.namespace().unwrap_or_default());
            let port =
                port.map_or_else(|| port_name.clone().unwrap_or_default(), |p| p.to_string());
            format!("service:{namespace}/{name}:{port}")
        }
        Service::Agent(_) => format!("agent:{}", proxy.to_path()),
    }
}

/// The PEM certificate and key of `proxy`'s `FrontProxy` Secret, read from
/// `proxy_namespace`, after the label and target checks. Closed: any doubt is
/// an error, never a connection without the certificate.
///
/// # Errors
///
/// A [`FrontProxyCertError`] naming what is wrong.
pub async fn load(
    client: Client,
    proxy_namespace: &str,
    proxy: &ProxyKubeApi,
) -> Result<(String, String), FrontProxyCertError> {
    let name = secret_name(proxy)?;
    let secrets: kube::Api<Secret> = kube::Api::namespaced(client, proxy_namespace);
    let secret = secrets
        .get(&name)
        .await
        .map_err(|_| FrontProxyCertError::Missing {
            namespace: proxy_namespace.to_string(),
            name: name.clone(),
        })?;
    check(&secret, proxy_namespace, &name, &canonical_target(proxy))
}

/// [`load`] without the read: the label, the annotation and the two keys.
///
/// # Errors
///
/// A [`FrontProxyCertError`] naming what is wrong.
pub fn check(
    secret: &Secret,
    namespace: &str,
    name: &str,
    expected_target: &str,
) -> Result<(String, String), FrontProxyCertError> {
    let owned = secret
        .labels()
        .get(MANAGED_BY_LABEL)
        .is_some_and(|value| value == MANAGED_BY_VALUE);
    if !owned {
        return Err(FrontProxyCertError::NotOwned {
            namespace: namespace.to_string(),
            name: name.to_string(),
        });
    }
    let found = secret.annotations().get(TARGET_ANNOTATION).cloned();
    if found.as_deref() != Some(expected_target) {
        return Err(FrontProxyCertError::TargetMismatch {
            namespace: namespace.to_string(),
            name: name.to_string(),
            expected: expected_target.to_string(),
            found,
        });
    }
    let empty = BTreeMap::new();
    let data = secret.data.as_ref().unwrap_or(&empty);
    let value = |key: &'static str| {
        data.get(key)
            .and_then(|bytes| String::from_utf8(bytes.0.clone()).ok())
            .filter(|pem| !pem.trim().is_empty())
            .ok_or(FrontProxyCertError::MissingKey {
                namespace: namespace.to_string(),
                name: name.to_string(),
                key,
            })
    };
    Ok((value("tls.crt")?, value("tls.key")?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::ByteString;

    fn proxy(ns: &str, name: &str, service: serde_json::Value) -> ProxyKubeApi {
        let spec = serde_json::from_value(serde_json::json!({
            "cert": { "SystemRoots": true },
            "service": service,
        }))
        .expect("spec");
        let mut proxy = ProxyKubeApi::new(name, spec);
        proxy.metadata.namespace = Some(ns.to_string());
        proxy
    }

    fn secret(labelled: bool, target: Option<&str>) -> Secret {
        let mut secret = Secret::default();
        if labelled {
            secret.metadata.labels =
                Some([(MANAGED_BY_LABEL.to_string(), MANAGED_BY_VALUE.to_string())].into());
        }
        if let Some(target) = target {
            secret.metadata.annotations =
                Some([(TARGET_ANNOTATION.to_string(), target.to_string())].into());
        }
        secret.data = Some(
            [
                ("tls.crt".to_string(), ByteString(b"CERT".to_vec())),
                ("tls.key".to_string(), ByteString(b"KEY".to_vec())),
            ]
            .into(),
        );
        secret
    }

    /// Decision 56: `ns/name` never collides, a namespace holding no dot.
    #[test]
    fn the_secret_name_is_unambiguous() {
        let external = serde_json::json!({ "ExternalService": { "url": "https://k:6443" } });
        let a = secret_name(&proxy("team-a", "prod", external.clone())).unwrap();
        let b = secret_name(&proxy("team", "a-prod", external.clone())).unwrap();
        assert_eq!(a, "proxyauth-frontproxy-team-a.prod");
        assert_ne!(a, b);
        let long = "x".repeat(240);
        assert!(matches!(
            secret_name(&proxy("ns", &long, external.clone())),
            Err(FrontProxyCertError::NameTooLong(_))
        ));
        // 253 characters is a valid name, 254 is not.
        let fits = "x".repeat(253 - "proxyauth-frontproxy-ns.".len());
        assert_eq!(
            secret_name(&proxy("ns", &fits, external.clone()))
                .unwrap()
                .len(),
            253
        );
        assert!(matches!(
            secret_name(&proxy("ns", &format!("{fits}x"), external)),
            Err(FrontProxyCertError::NameTooLong(254))
        ));
    }

    #[test]
    fn the_target_is_the_service_in_a_canonical_form() {
        let external = proxy(
            "a",
            "c",
            serde_json::json!({ "ExternalService": { "url": "https://k:6443/" } }),
        );
        assert_eq!(canonical_target(&external), "external:https://k:6443");
        let service = proxy(
            "a",
            "c",
            serde_json::json!({ "KubernetesService": { "name": "kubernetes", "namespace": "default", "port": 443 } }),
        );
        assert_eq!(canonical_target(&service), "service:default/kubernetes:443");
    }

    /// T18, T29: unlabelled, or issued for another target, never used.
    #[test]
    fn a_secret_without_the_label_or_for_another_target_is_never_used() {
        let target = "external:https://k:6443";
        assert_eq!(
            check(&secret(true, Some(target)), "proxyauth", "s", target),
            Ok(("CERT".to_string(), "KEY".to_string()))
        );
        assert!(matches!(
            check(&secret(false, Some(target)), "proxyauth", "s", target),
            Err(FrontProxyCertError::NotOwned { .. })
        ));
        for annotation in [None, Some("external:https://other:6443")] {
            let err = check(&secret(true, annotation), "proxyauth", "s", target).unwrap_err();
            assert_eq!(err.reason(), "ClientCertTargetMismatch");
            assert!(!err.to_string().contains("CERT"));
        }
    }
}
