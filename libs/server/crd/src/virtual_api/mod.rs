//! Declarative activation of the virtual APIs the proxy can synthesise.
//!
//! A virtual API is an API the target cluster does not have: the proxy answers
//! discovery for it and translates requests and responses onto an API the
//! cluster does have. The kinds are a closed enum so a cluster operator cannot
//! ask for a mapper the binary does not implement.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::certificate::CertSource;
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
    /// A least-privilege bearer token (`list` only on `namespaces`, nothing
    /// else) used to enumerate candidate namespaces so `OpenShiftProject`'s
    /// `LIST projects` can be filtered to what the caller can individually
    /// `get`, matching `OpenShift`'s per-project visibility model.
    ///
    /// Unset by default: `LIST projects` then behaves as a plain,
    /// unfiltered `LIST namespaces`. Only meaningful for
    /// `VirtualApiKind::OpenShiftProject`.
    #[serde(default)]
    pub list_fallback_token: Option<CertSource>,
}

impl VirtualApiConfiguration {
    #[must_use]
    pub fn new(kind: VirtualApiKind) -> Self {
        Self {
            kind,
            enabled: true,
            list_fallback_token: None,
        }
    }

    /// Reject configuration combinations no mapper can act on.
    ///
    /// `list_fallback_token` is only ever read for
    /// `VirtualApiKind::OpenShiftProject` (see `list_fallback::configured_token`
    /// in the API crate); setting it on another kind would otherwise be
    /// silently ignored, with no feedback that the configuration does
    /// nothing.
    pub fn validate(&self) -> Result<(), String> {
        if self.list_fallback_token.is_some() && self.kind != VirtualApiKind::OpenShiftProject {
            return Err(format!(
                "list_fallback_token is only meaningful for VirtualApiKind::OpenShiftProject, \
                 not {:?}",
                self.kind
            ));
        }
        Ok(())
    }
}

/// The kinds enabled on a spec, deduplicated and in declaration order.
#[must_use]
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
            list_fallback_token: None,
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

    #[test]
    fn list_fallback_token_defaults_to_none_when_omitted() {
        let configuration: VirtualApiConfiguration =
            serde_json::from_value(serde_json::json!({ "kind": "OpenShiftProject" }))
                .expect("configuration should deserialize");
        assert!(configuration.list_fallback_token.is_none());

        let configuration = VirtualApiConfiguration::new(VirtualApiKind::OpenShiftProject);
        assert!(configuration.list_fallback_token.is_none());
    }

    #[test]
    fn list_fallback_token_is_valid_on_open_shift_project() {
        let mut configuration = VirtualApiConfiguration::new(VirtualApiKind::OpenShiftProject);
        configuration.list_fallback_token = Some(CertSource::Cert("dGVzdA==".to_string()));
        assert!(configuration.validate().is_ok());
    }

    #[test]
    fn no_list_fallback_token_is_always_valid() {
        assert!(
            VirtualApiConfiguration::new(VirtualApiKind::OpenShiftProject)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn list_fallback_token_deserializes_when_present() {
        let configuration: VirtualApiConfiguration = serde_json::from_value(serde_json::json!({
            "kind": "OpenShiftProject",
            "list_fallback_token": { "Cert": "dGVzdA==" },
        }))
        .expect("configuration should deserialize");
        assert!(matches!(
            configuration.list_fallback_token,
            Some(CertSource::Cert(_))
        ));
    }
}
