use crate::default::default_enabled;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::namespaced_access_configuration::NamespacedAccessConfiguration;

/// Allowed crd configuration, used in conjunction with the allowed_paths configuration
/// /apis/{group}/{version}/namespaces/{namespace}/{kind}/
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct AllowedCrdConfiguration {
    /// The group of the crd
    pub group: String,
    /// The version of the crd
    pub version: String,
    /// The kind of the crd
    pub kind: String,
    /// Wether or not the kind has a specific plural form, if true, the plural form will be used in the path instead of the kind
    /// for example, if the kind is "MyResource" and the plural form is "MyResources"
    /// the path will be /apis/{group}/{version}/namespaces/{namespace}/myresources/
    /// instead of       /apis/{group}/{version}/namespaces/{namespace}/myresource/
    /// In case of cluster-scoped crd
    /// the path will be /apis/{group}/{version}/myresources/
    /// instead of       /apis/{group}/{version}/myresource/
    pub plural: Option<String>,

    /// Wether or not the ressource is namespaced, if true, the namespace access rules will be applied to this resource
    pub namespace: NamespacedAccessConfiguration,
    /// Namespaced or not
    /// default: true
    #[serde(default = "default_enabled")]
    pub namespaced: bool,
}
