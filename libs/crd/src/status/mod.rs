use kube::api::Patch;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize, Clone, JsonSchema, Default, Debug)]
pub struct ProxyKubeApiStatus {
    pub exposed: bool,
    pub path: Option<String>,
    pub error: Option<String>,
}

impl ProxyKubeApiStatus {
    pub fn new(exposed: bool, path: Option<String>, error: Option<String>) -> Self {
        Self {
            exposed,
            path,
            error,
        }
    }
    pub fn get_patch(&self) -> Patch<Value> {
        Patch::Apply(json!({
            "apiVersion": "weebo.si.rs/v1",
            "kind": "ProxyKubeApi",
            "status": &self
        }))
    }
}
