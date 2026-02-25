use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Namespace rule kind, either :
/// - A vec of allowed namespaces, if the user is only allowed to access a specific set of namespaces
/// - A vec of denied namespaces, if the user is allowed to access all namespaces except a specific set of namespaces
/// - A Parametised rule, if the user is allowed to access namespaces that match a certain pattern, for example namespaces that start with "dev-{{username}}"
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum NamespacedAccessRuleKind {
    AllowedNamespaces(Vec<String>),
    DeniedNamespaces(Vec<String>),
    /// The parametised rule is a string that can contain the {{username}} parameter
    /// which will be replaced by the username of the user making the request, and the {{group}} parameter
    /// which will be replaced by the groups of the user making the request
    /// allowed parameters are : {{username}} and {{group}}
    ParametisedRule(String),
}
