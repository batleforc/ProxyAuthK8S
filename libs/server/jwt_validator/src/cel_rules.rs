//! CEL evaluation for the claim rules, claim mappings and user rules.
//!
//! Mirrors the variables the apiserver's structured authentication configuration
//! exposes: `claims` (the verified token payload) for
//! `claim_validation_rules`/`claim_mappings`, and `user` (the mapped identity)
//! for `user_validation_rules`.
//!
//! The Kubernetes CEL extension library is registered through [`kube_cel`], so
//! an expression an operator copied out of their apiserver configuration behaves
//! the same here — `strings`, `lists`, `sets`, `regex`, `urls` and the rest are
//! all present rather than being a subset an operator has to discover by trial.

use kube_cel::KubeCelExt;
use kube_cel::cel::{Context, Program, Value};

use crate::error::JwtValidationError;

/// The identity a token maps to, before it becomes an api-level `User`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct MappedUser {
    pub username: String,
    pub groups: Vec<String>,
    pub uid: String,
    pub extra: std::collections::BTreeMap<String, Vec<String>>,
}

/// Compiled programs, keyed by their source text.
///
/// Every claim rule, claim mapping and user rule of the matching authenticator
/// is evaluated on *every* proxied request in `JwtAuthenticators` mode, and
/// parsing the same handful of fixed expressions each time is pure waste. The
/// map is bounded by the number of distinct expressions an operator has written
/// across the cluster's `ProxyKubeApi` resources.
static PROGRAMS: std::sync::LazyLock<
    std::sync::RwLock<std::collections::HashMap<String, std::sync::Arc<Program>>>,
> = std::sync::LazyLock::new(|| std::sync::RwLock::new(std::collections::HashMap::new()));

/// The compiled form of `expression`, compiling and memoising it on first use.
fn program_for(expression: &str) -> Result<std::sync::Arc<Program>, JwtValidationError> {
    // Poisoning is ignored: nothing here can leave the map inconsistent, and a
    // panic elsewhere must not take authentication down with it.
    if let Some(program) = PROGRAMS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(expression)
    {
        return Ok(std::sync::Arc::clone(program));
    }

    let program = std::sync::Arc::new(Program::compile(expression).map_err(|e| {
        JwtValidationError::ExpressionCompile {
            expression: expression.to_string(),
            reason: e.to_string(),
        }
    })?);
    PROGRAMS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(expression.to_string(), std::sync::Arc::clone(&program));
    Ok(program)
}

/// Compile and evaluate `expression` with `claims` bound.
fn evaluate(
    expression: &str,
    variable: &str,
    value: &serde_json::Value,
) -> Result<Value, JwtValidationError> {
    let program = program_for(expression)?;
    let mut context = Context::default().with_all();
    let bound =
        kube_cel::cel::to_value(value).map_err(|e| JwtValidationError::ExpressionEvaluate {
            expression: expression.to_string(),
            reason: format!("could not bind {variable}: {e}"),
        })?;
    context.add_variable_from_value(variable, bound);
    program
        .execute(&context)
        .map_err(|e| JwtValidationError::ExpressionEvaluate {
            expression: expression.to_string(),
            reason: e.to_string(),
        })
}

/// Evaluate an expression that must yield a bool.
pub fn evaluate_bool(
    expression: &str,
    variable: &str,
    value: &serde_json::Value,
) -> Result<bool, JwtValidationError> {
    match evaluate(expression, variable, value)? {
        Value::Bool(result) => Ok(result),
        other => Err(JwtValidationError::ExpressionType {
            expression: expression.to_string(),
            got: type_name(&other).to_string(),
            want: "bool".to_string(),
        }),
    }
}

/// Evaluate an expression that must yield a string.
pub fn evaluate_string(
    expression: &str,
    variable: &str,
    value: &serde_json::Value,
) -> Result<String, JwtValidationError> {
    match evaluate(expression, variable, value)? {
        Value::String(result) => Ok(result.to_string()),
        other => Err(JwtValidationError::ExpressionType {
            expression: expression.to_string(),
            got: type_name(&other).to_string(),
            want: "string".to_string(),
        }),
    }
}

/// Evaluate an expression that may yield a string or a list of strings.
///
/// `groups` and every `extra` value are multi-valued in Kubernetes, and the
/// apiserver accepts either shape there; a single string is treated as a
/// one-element list rather than an error.
pub fn evaluate_string_list(
    expression: &str,
    variable: &str,
    value: &serde_json::Value,
) -> Result<Vec<String>, JwtValidationError> {
    match evaluate(expression, variable, value)? {
        Value::String(result) => Ok(vec![result.to_string()]),
        Value::List(items) => items
            .iter()
            .map(|item| match item {
                Value::String(item) => Ok(item.to_string()),
                other => Err(JwtValidationError::ExpressionType {
                    expression: expression.to_string(),
                    got: type_name(other).to_string(),
                    want: "string".to_string(),
                }),
            })
            .collect(),
        Value::Null => Ok(Vec::new()),
        other => Err(JwtValidationError::ExpressionType {
            expression: expression.to_string(),
            got: type_name(&other).to_string(),
            want: "string or list of strings".to_string(),
        }),
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::List(_) => "list",
        Value::Map(_) => "map",
        Value::Function(..) => "function",
        Value::Int(_) => "int",
        Value::UInt(_) => "uint",
        Value::Float(_) => "double",
        Value::String(_) => "string",
        Value::Bytes(_) => "bytes",
        Value::Bool(_) => "bool",
        Value::Null => "null",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::{evaluate_bool, evaluate_string, evaluate_string_list};
    use serde_json::json;

    fn claims() -> serde_json::Value {
        json!({
            "iss": "https://issuer.example.com",
            "sub": "1234",
            "email": "alice@example.com",
            "email_verified": true,
            "hd": "example.com",
            "groups": ["dev", "ops"],
            "roles": "single-role",
        })
    }

    #[test]
    fn a_claim_comparison_evaluates() {
        assert!(evaluate_bool("claims.hd == 'example.com'", "claims", &claims()).unwrap());
        assert!(!evaluate_bool("claims.hd == 'other.com'", "claims", &claims()).unwrap());
    }

    #[test]
    fn the_kubernetes_extension_library_is_available() {
        // `startsWith` is stdlib, but the k8s extension functions (strings,
        // lists, sets, regex, urls…) are what an operator's apiserver rules
        // actually use — an expression copied from there must not fail here.
        assert!(
            evaluate_bool("claims.email.endsWith('@example.com')", "claims", &claims()).unwrap()
        );
        assert!(
            evaluate_bool("sets.contains(claims.groups, ['dev'])", "claims", &claims()).unwrap()
        );
        assert!(evaluate_bool("claims.groups.isSorted()", "claims", &claims()).unwrap());
    }

    #[test]
    fn a_string_mapping_evaluates() {
        assert_eq!(
            evaluate_string("claims.email", "claims", &claims()).unwrap(),
            "alice@example.com"
        );
        assert_eq!(
            evaluate_string("claims.sub + ':external'", "claims", &claims()).unwrap(),
            "1234:external"
        );
    }

    /// Kubernetes treats groups and every `extra` value as multi-valued, and
    /// accepts a bare string there as a one-element list.
    #[test]
    fn a_list_mapping_accepts_both_shapes() {
        assert_eq!(
            evaluate_string_list("claims.groups", "claims", &claims()).unwrap(),
            vec!["dev", "ops"]
        );
        assert_eq!(
            evaluate_string_list("claims.roles", "claims", &claims()).unwrap(),
            vec!["single-role"]
        );
    }

    #[test]
    fn a_non_boolean_rule_is_a_typed_error_not_a_silent_pass() {
        let err = evaluate_bool("claims.email", "claims", &claims()).unwrap_err();
        assert!(
            matches!(err, crate::error::JwtValidationError::ExpressionType { ref want, .. } if want == "bool"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn an_expression_that_does_not_compile_is_reported_as_such() {
        let err = evaluate_bool("claims.hd ==", "claims", &claims()).unwrap_err();
        assert!(
            matches!(
                err,
                crate::error::JwtValidationError::ExpressionCompile { .. }
            ),
            "unexpected error: {err}"
        );
    }

    /// A missing claim must not be treated as an empty string that then passes a
    /// comparison — it has to surface as an evaluation failure.
    #[test]
    fn referencing_a_missing_claim_fails_rather_than_defaulting() {
        let err = evaluate_bool("claims.nonexistent == ''", "claims", &claims()).unwrap_err();
        assert!(
            matches!(
                err,
                crate::error::JwtValidationError::ExpressionEvaluate { .. }
            ),
            "unexpected error: {err}"
        );
    }

    /// CEL's `has()` is the supported way to write an optional-claim rule.
    #[test]
    fn has_guards_an_optional_claim() {
        assert!(evaluate_bool("has(claims.hd)", "claims", &claims()).unwrap());
        assert!(!evaluate_bool("has(claims.nonexistent)", "claims", &claims()).unwrap());
    }
}
