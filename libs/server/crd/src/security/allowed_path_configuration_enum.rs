use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::allowed_crd_configuration::AllowedCrdConfiguration;
use super::allowed_path_configuration::AllowedPathConfiguration;
use super::port_range::PortPolicy;
use super::request_info::RequestInfo;
use super::verb::Verb;

/// How an allowed resource is described.
///
/// - `Path` matches on the raw upstream path (with `*`/`**`/`{{...}}` semantics);
/// - `Crd` matches on group/version/kind and applies the per-namespace access
///   rules, translating to a path via [`super::AllowedCrdConfiguration`].
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum AllowedPathConfigurationEnum {
    Path(AllowedPathConfiguration),
    Crd(AllowedCrdConfiguration),
}

impl AllowedPathConfigurationEnum {
    /// Check whether an upstream request path is allowed by this rule.
    #[must_use]
    pub fn matches(&self, path: &str, username: &str, groups: &[String]) -> bool {
        match self {
            AllowedPathConfigurationEnum::Path(config) => config.matches(path, username, groups),
            AllowedPathConfigurationEnum::Crd(config) => config.matches(path, username, groups),
        }
    }

    /// The ports this rule grants, `None` when it lists none.
    #[must_use]
    pub fn port_policy(&self) -> Option<PortPolicy> {
        match self {
            AllowedPathConfigurationEnum::Path(config) => config.port_policy(),
            AllowedPathConfigurationEnum::Crd(config) => config.port_policy(),
        }
    }

    /// The `verbs` this rule declares, if any.
    #[must_use]
    pub fn verbs(&self) -> Option<&[Verb]> {
        match self {
            AllowedPathConfigurationEnum::Path(config) => config.verbs.as_deref(),
            AllowedPathConfigurationEnum::Crd(config) => config.verbs.as_deref(),
        }
    }

    /// Whether this rule can apply to the caller: a `Path` rule has no
    /// subject, a `Crd` one whose parametised namespace expands to nothing for
    /// the caller reaches nothing.
    fn applies_to(&self, username: &str, groups: &[String]) -> bool {
        match self {
            AllowedPathConfigurationEnum::Path(_) => true,
            AllowedPathConfigurationEnum::Crd(config) => config.applies_to(username, groups),
        }
    }

    /// Whether this rule restricts the verbs of the caller: it lists verbs and
    /// can apply to them.
    #[must_use]
    pub fn restricts_verbs(&self, username: &str, groups: &[String]) -> bool {
        self.verbs().is_some() && self.applies_to(username, groups)
    }

    /// Whether this rule allows the verb of `info` (RFC 0006 §5.2): its
    /// `verbs`, else every verb, or only the reads when the caller is
    /// `restricted`; plus the `create` its capabilities imply (`allowed_ports`
    /// on `portforward`).
    #[must_use]
    pub fn allows_verb(&self, info: &RequestInfo, restricted: bool) -> bool {
        let declared = match self.verbs() {
            Some(verbs) => verbs.iter().any(|verb| verb.grants(info.verb)),
            None => !restricted || Verb::READ.contains(&info.verb),
        };
        declared
            || (info.verb == Verb::Create
                && info.subresource == "portforward"
                && self.port_policy().is_some())
    }

    /// Whether this rule restricts the ports of `username`: it lists ports and
    /// can apply to the caller. A `Path` rule has no subject, so it applies to
    /// everyone.
    #[must_use]
    pub fn restricts_ports(&self, username: &str, groups: &[String]) -> bool {
        self.port_policy().is_some() && self.applies_to(username, groups)
    }

    /// Validate the rule at reconcile time (mirrors the CEL admission rules).
    pub fn validate(&self) -> Result<(), String> {
        match self {
            AllowedPathConfigurationEnum::Path(config) => config.validate(),
            AllowedPathConfigurationEnum::Crd(config) => config.validate(),
        }
    }
}
