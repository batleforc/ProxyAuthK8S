use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// CEL rule on a `verbs` list mirroring [`validate_verbs`]: `*` stands alone.
/// Emptiness is refused by the schema's `minItems`, and unknown values by the
/// enum.
pub(crate) const VERBS_RULE: &str = "!self.exists(v, v == '*') || size(self) == 1";

/// A Kubernetes verb, as the apiserver computes it for a request (see
/// [`super::request_info`]), or `*` for every verb.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Verb {
    Get,
    List,
    Watch,
    Create,
    Update,
    Patch,
    Delete,
    Deletecollection,
    /// Every verb.
    #[serde(rename = "*")]
    All,
}

impl Verb {
    /// The reads a rule without `verbs` grants to a verb-restricted caller.
    pub const READ: [Verb; 3] = [Verb::Get, Verb::List, Verb::Watch];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Verb::Get => "get",
            Verb::List => "list",
            Verb::Watch => "watch",
            Verb::Create => "create",
            Verb::Update => "update",
            Verb::Patch => "patch",
            Verb::Delete => "delete",
            Verb::Deletecollection => "deletecollection",
            Verb::All => "*",
        }
    }

    /// Whether a rule granting `self` allows a request of verb `requested`.
    #[must_use]
    pub fn grants(self, requested: Verb) -> bool {
        self == Verb::All || self == requested
    }
}

/// Reject an empty list, or `*` next to other verbs (mirrors the CEL rule).
pub(crate) fn validate_verbs(verbs: &[Verb]) -> Result<(), String> {
    if verbs.is_empty() {
        return Err("verbs must not be empty: omit it, or list the verbs to allow".to_string());
    }
    if verbs.contains(&Verb::All) && verbs.len() > 1 {
        return Err("verbs: \"*\" already allows every verb and must stand alone".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_serialize_in_lower_case_and_star() {
        let verbs: Vec<Verb> =
            serde_json::from_value(serde_json::json!(["get", "deletecollection", "*"])).unwrap();
        assert_eq!(verbs, [Verb::Get, Verb::Deletecollection, Verb::All]);
        assert_eq!(serde_json::to_value(Verb::All).unwrap(), "*");
        assert!(serde_json::from_value::<Verb>(serde_json::json!("GET")).is_err());
        assert!(serde_json::from_value::<Verb>(serde_json::json!("proxy")).is_err());
    }

    #[test]
    fn validation_refuses_empty_and_mixed_star() {
        assert!(validate_verbs(&[]).is_err());
        assert!(validate_verbs(&[Verb::All, Verb::Get]).is_err());
        assert!(validate_verbs(&[Verb::All]).is_ok());
        assert!(validate_verbs(&[Verb::Get, Verb::List]).is_ok());
    }
}
