use kube::api::Patch;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize, Deserialize, Clone, JsonSchema, Default, Debug)]
pub struct ProxyKubeApiStatus {
    pub exposed: bool,
    pub path: Option<String>,
    pub error: Option<String>,
    /// Rules whose meaning a port or verb restriction changes (see
    /// `SecurityConfiguration::warnings`). They are still applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl ProxyKubeApiStatus {
    #[must_use]
    pub fn new(exposed: bool, path: Option<String>, error: Option<String>) -> Self {
        Self {
            exposed,
            path,
            error,
            warnings: Vec::new(),
        }
    }
    #[must_use]
    pub fn patch(&self) -> Patch<Value> {
        Patch::Apply(json!({
            "apiVersion": "weebo.si.rs/v1",
            "kind": "ProxyKubeApi",
            "status": &self
        }))
    }
    #[must_use]
    pub fn equal(&self, other: &ProxyKubeApiStatus) -> bool {
        self.exposed == other.exposed
            && self.path == other.path
            && self.error == other.error
            && self.warnings == other.warnings
    }
}
