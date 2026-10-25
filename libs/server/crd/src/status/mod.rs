use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use weebo_kit_api::Condition;

#[derive(Serialize, Deserialize, Clone, JsonSchema, Default, Debug)]
pub struct ProxyKubeApiStatus {
    pub exposed: bool,
    pub path: Option<String>,
    pub error: Option<String>,
    /// Rules whose meaning a port or verb restriction changes (see
    /// `SecurityConfiguration::warnings`). They are still applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// The agent tunnels of a `service: Agent` cluster (RFC 0012), from the
    /// gateway pods' presences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentStatus>,
    /// `Ready`, then one condition per advisory (RFC 0011). Written by the
    /// controller's status publication, never in the Redis copy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
    /// The generation last reconciled successfully.
    #[serde(
        default,
        rename = "observedGeneration",
        skip_serializing_if = "Option::is_none"
    )]
    pub observed_generation: Option<i64>,
}

/// What the gateway pods report about one cluster's agent. Only values that
/// change with the tunnels themselves: a status write re-triggers a
/// reconcile, so no timestamp lives here.
#[derive(Serialize, Deserialize, Clone, JsonSchema, Default, Debug, PartialEq, Eq)]
pub struct AgentStatus {
    /// Open tunnels, every gateway pod together.
    pub tunnels: u32,
    /// The gateway pods holding them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gateways: Vec<String>,
    /// The agents' versions, as they announce them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<String>,
    /// SHA-256 of the remote `kube-root-ca.crt`, as the agents report it.
    #[serde(
        default,
        rename = "caFingerprints",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub ca_fingerprints: Vec<String>,
    /// The controller's version, the one the agents' are compared to
    /// (RFC 0013, decision 4).
    #[serde(
        default,
        rename = "proxyVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub proxy_version: Option<String>,
    /// The open tunnels by announced version and negotiated protocol.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentCount>,
    /// Why an agent's tunnel was refused, while the refusal is recent
    /// (RFC 0013 §4.1): only a protocol refusal is kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<AgentRefusal>,
}

/// Open tunnels of one announced version and negotiated protocol.
#[derive(
    Serialize, Deserialize, Clone, JsonSchema, Default, Debug, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct AgentCount {
    /// As the agent announces it, `invalid` when it is not a SemVer.
    pub version: String,
    pub protocol: u32,
    pub tunnels: u32,
}

/// A gateway pod's refusal of an agent's tunnel, for 5 min after the last
/// attempt.
#[derive(Serialize, Deserialize, Clone, JsonSchema, Default, Debug, PartialEq, Eq)]
pub struct AgentRefusal {
    /// `protocol`.
    pub reason: String,
    /// The agent's version, `invalid` when it is not a SemVer.
    pub version: String,
    /// The protocols the agent speaks, `min-max`.
    pub protocols: String,
}

impl ProxyKubeApiStatus {
    #[must_use]
    pub fn new(exposed: bool, path: Option<String>, error: Option<String>) -> Self {
        Self {
            exposed,
            path,
            error,
            ..Self::default()
        }
    }

    /// The fields of this status a merge patch carries, next to the
    /// conditions the publication adds. A key missing from a merge patch is
    /// left in place, so an empty `warnings` is sent as `null`, which drops it.
    #[must_use]
    pub fn fields(&self) -> Map<String, Value> {
        let warnings = if self.warnings.is_empty() {
            Value::Null
        } else {
            json!(self.warnings)
        };
        Map::from_iter([
            ("exposed".to_string(), json!(self.exposed)),
            ("path".to_string(), json!(self.path)),
            ("error".to_string(), json!(self.error)),
            ("warnings".to_string(), warnings),
            ("agent".to_string(), json!(self.agent)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_warnings_list_is_sent_as_null_so_the_merge_patch_drops_it() {
        let fields = ProxyKubeApiStatus::new(true, Some("/clusters/a/b".into()), None).fields();
        assert_eq!(fields["warnings"], Value::Null);
        assert_eq!(fields["error"], Value::Null);
        assert_eq!(fields["exposed"], true);
        assert_eq!(fields["path"], "/clusters/a/b");
    }

    #[test]
    fn warnings_are_sent_as_a_list() {
        let mut status = ProxyKubeApiStatus::new(true, None, None);
        status.warnings = vec!["rule 0 covers nodes/".into()];
        assert_eq!(status.fields()["warnings"], json!(["rule 0 covers nodes/"]));
    }

    #[test]
    fn the_fields_never_carry_the_conditions() {
        let mut status = ProxyKubeApiStatus::new(true, None, None);
        status.conditions = vec![Condition::default()];
        status.observed_generation = Some(3);
        let fields = status.fields();
        assert!(!fields.contains_key("conditions"));
        assert!(!fields.contains_key("observedGeneration"));
    }

    #[test]
    fn a_status_without_conditions_serializes_as_before() {
        let json = serde_json::to_value(ProxyKubeApiStatus::new(false, None, None))
            .expect("status serializes");
        assert_eq!(
            json,
            json!({ "exposed": false, "path": null, "error": null })
        );
    }
}
