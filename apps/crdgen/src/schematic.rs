//! This project's schematic kinds, drawn by the docs' CRD IDE and never
//! applied: the clusters a `ProxyKubeApi` gives access to and the ways
//! people reach them. Their doc comments are the IDE's descriptions.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use weebo_kit_api::links::{LinkError, Roles, Schematic, Scope, schematic};

/// The group of this project's schematic kinds.
pub const GROUP: &str = "schematic.weebo.si.rs";

/// A Kubernetes cluster behind a `ProxyKubeApi`, named by its API server's
/// URL (an `ExternalService` makes one).
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KubeClusterSpec {
    /// `talos`, `openshift`, `k3s`, `eks`…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distribution: Option<String>,
    /// Free text, for the drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// How people reach a cluster through the proxy.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AccessType {
    /// The web dashboard (`expose_via_dashboard`).
    Dashboard,
    /// `kubectl` with the `proxyauth` plugin.
    KubectlPlugin,
    /// The mediated OAuth 2.0 server (`expose_oauth_authorization_server`).
    Oauth,
    /// Straight to the proxied API, with a token.
    Direct,
}

/// A name-only reference.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct NameRef {
    pub name: String,
}

/// A way to reach a `ProxyKubeApi`: who, through what, with which group.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccessMethodSpec {
    #[serde(rename = "type")]
    pub r#type: AccessType,
    /// The `ProxyKubeApi` it goes through.
    #[schemars(extend("x-weebo-link" = { "to": [{ "group": "weebo.si.rs", "kind": "ProxyKubeApi" }] }))]
    pub proxy_ref: NameRef,
    /// The issuer whose tokens it uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("x-weebo-link" = { "to": [{ "group": "schematic.weebo.io", "kind": "OidcIssuer" }] }))]
    pub issuer_ref: Option<NameRef>,
    /// The OIDC group it requires (`dashboard_group`, `proxy_group`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Free text, for the drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub fn own(roles: &Roles) -> Result<BTreeMap<String, Schematic>, LinkError> {
    Ok(BTreeMap::from([
        (
            "KubeCluster".to_string(),
            schematic::<KubeClusterSpec>(GROUP, Scope::Cluster, roles)?,
        ),
        (
            "AccessMethod".to_string(),
            schematic::<AccessMethodSpec>(GROUP, Scope::Namespaced, roles)?,
        ),
    ]))
}
