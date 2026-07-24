//! Declarative activation of the virtual APIs the proxy can synthesise.
//!
//! A virtual API is an API the target cluster does not have: the proxy answers
//! discovery for it and translates requests and responses onto an API the
//! cluster does have. The kinds are a closed enum so a cluster operator cannot
//! ask for a mapper the binary does not implement.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::default::default_enabled;

/// The virtual APIs this build knows how to synthesise.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtualApiKind {
    /// `project.openshift.io/v1` Projects mapped onto core Namespaces.
    OpenShiftProject,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct VirtualApiConfiguration {
    /// Which virtual API to expose
    pub kind: VirtualApiKind,
    /// Enable or disable this virtual API
    /// Default: true
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

impl VirtualApiConfiguration {
    pub fn new(kind: VirtualApiKind) -> Self {
        Self {
            kind,
            enabled: true,
        }
    }
}

/// The kinds enabled on a spec, deduplicated and in declaration order.
pub fn enabled_kinds(configurations: &[VirtualApiConfiguration]) -> Vec<VirtualApiKind> {
    let mut kinds: Vec<VirtualApiKind> = Vec::new();
    for configuration in configurations.iter().filter(|c| c.enabled) {
        if !kinds.contains(&configuration.kind) {
            kinds.push(configuration.kind);
        }
    }
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_kinds_skips_disabled_entries() {
        let configurations = vec![VirtualApiConfiguration {
            kind: VirtualApiKind::OpenShiftProject,
            enabled: false,
        }];
        assert!(enabled_kinds(&configurations).is_empty());
    }

    #[test]
    fn enabled_kinds_deduplicates() {
        let configurations = vec![
            VirtualApiConfiguration::new(VirtualApiKind::OpenShiftProject),
            VirtualApiConfiguration::new(VirtualApiKind::OpenShiftProject),
        ];
        assert_eq!(
            enabled_kinds(&configurations),
            vec![VirtualApiKind::OpenShiftProject]
        );
    }

    #[test]
    fn enabled_defaults_to_true_when_omitted() {
        let configuration: VirtualApiConfiguration =
            serde_json::from_value(serde_json::json!({ "kind": "OpenShiftProject" }))
                .expect("configuration should deserialize");
        assert!(configuration.enabled);
    }
}
