//! `pods/{name}/exec` and `/attach` (RFC 0004): which paths they are, what
//! their query asks for, and how the audit records it.
//!
//! The query is read the way the apiserver reads it into `PodExecOptions`:
//! one percent-decode, `+` as a space, every `command` in order, and the
//! booleans of `Convert_Slice_string_To_bool` (the first value counts; false
//! when absent, `0` or `false` whatever the case; anything else, the empty
//! value included, is true). A duplicated parameter or a `;` is refused: the
//! proxy never reads a request differently from the apiserver.

use crd::security::path_matcher::percent_decode_once;

/// At most this many `command` parameters.
const MAX_COMMAND_ELEMENTS: usize = 32;

/// At most this many bytes in one `command` element.
const MAX_COMMAND_ELEMENT_BYTES: usize = 4 * 1024;

/// What an exec or attach query asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ExecRequest {
    /// Every `command`, in order (empty for an attach).
    pub argv: Vec<String>,
    pub container: Option<String>,
    pub stdin: bool,
    pub stdout: bool,
    pub stderr: bool,
    pub tty: bool,
}

/// The decoded segments of an upstream path.
fn segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .map(percent_decode_once)
        .collect()
}

/// The subresource of a `pods/{name}/{subresource}` path, if it is one.
fn pod_subresource(path: &str) -> Option<String> {
    match segments(path).as_slice() {
        [.., pods, _, subresource] if pods == "pods" => Some(subresource.clone()),
        _ => None,
    }
}

/// Whether `path` is `pods/{name}/exec` or `pods/{name}/attach`.
pub(super) fn is_exec_or_attach_path(path: &str) -> bool {
    matches!(pod_subresource(path).as_deref(), Some("exec" | "attach"))
}

/// The `kind` label of the session duration histogram for a tunnelled path.
pub(super) fn session_kind(path: &str) -> &'static str {
    match pod_subresource(path).as_deref() {
        Some("exec") => "exec",
        Some("attach") => "attach",
        Some("portforward") => "portforward",
        _ if segments(path).iter().any(|segment| segment == "proxy") => "proxy",
        _ => "other",
    }
}

/// The apiserver's reading of a boolean query parameter.
fn query_bool(value: &str) -> bool {
    !(value == "0" || value.eq_ignore_ascii_case("false"))
}

/// Read an exec or attach query string.
///
/// # Errors
///
/// A message for the client when the query has a `;`, a duplicated
/// `container` or boolean, more than [`MAX_COMMAND_ELEMENTS`] commands, or a
/// command element longer than [`MAX_COMMAND_ELEMENT_BYTES`].
pub(super) fn parse_exec_query(query: &str) -> Result<ExecRequest, String> {
    if query.contains(';') {
        return Err("a ';' in the exec query is not accepted".to_string());
    }
    let mut request = ExecRequest::default();
    let mut seen: Vec<String> = Vec::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode_once(&key.replace('+', " "));
        let value = percent_decode_once(&value.replace('+', " "));
        if key == "command" {
            if request.argv.len() == MAX_COMMAND_ELEMENTS {
                return Err(format!("more than {MAX_COMMAND_ELEMENTS} command elements"));
            }
            if value.len() > MAX_COMMAND_ELEMENT_BYTES {
                return Err(format!(
                    "a command element is longer than {MAX_COMMAND_ELEMENT_BYTES} bytes"
                ));
            }
            request.argv.push(value);
            continue;
        }
        let field = match key.as_str() {
            "container" | "stdin" | "stdout" | "stderr" | "tty" => key.clone(),
            _ => continue,
        };
        if seen.contains(&field) {
            return Err(format!("the exec query repeats '{field}'"));
        }
        seen.push(field);
        match key.as_str() {
            "container" => request.container = Some(value),
            "stdin" => request.stdin = query_bool(&value),
            "stdout" => request.stdout = query_bool(&value),
            "stderr" => request.stderr = query_bool(&value),
            _ => request.tty = query_bool(&value),
        }
    }
    Ok(request)
}

/// `argv` with what looks like a secret replaced by `***`: `-p<x>`,
/// `--password=<x>`, `--token=<x>`, and the element after `-p`, `--password`
/// or `--token`. A heuristic (`mask_exec_secrets`), not a guarantee.
pub(super) fn mask_secrets(argv: &[String]) -> Vec<String> {
    let mut masked = Vec::with_capacity(argv.len());
    let mut next_is_secret = false;
    for element in argv {
        if next_is_secret {
            masked.push("***".to_string());
            next_is_secret = false;
            continue;
        }
        match element.as_str() {
            "-p" | "--password" | "--token" => {
                next_is_secret = true;
                masked.push(element.clone());
            }
            e if e.starts_with("--password=") || e.starts_with("--token=") => {
                let (flag, _) = e.split_once('=').unwrap_or((e, ""));
                masked.push(format!("{flag}=***"));
            }
            e if e.starts_with("-p") && e.len() > 2 && !e.starts_with("--") => {
                masked.push("-p***".to_string());
            }
            _ => masked.push(element.clone()),
        }
    }
    masked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn exec_and_attach_paths_are_recognised() {
        assert!(is_exec_or_attach_path(
            "/api/v1/namespaces/dev/pods/web/exec"
        ));
        assert!(is_exec_or_attach_path(
            "/api/v1/namespaces/dev/pods/web/attach"
        ));
        assert!(is_exec_or_attach_path(
            "/api/v1/namespaces/dev/pods/web/ex%65c"
        ));
        assert!(!is_exec_or_attach_path(
            "/api/v1/namespaces/dev/pods/web/log"
        ));
        assert!(!is_exec_or_attach_path("/api/v1/namespaces/dev/pods/exec"));
    }

    #[test]
    fn session_kinds_follow_the_path() {
        assert_eq!(session_kind("/api/v1/namespaces/d/pods/w/exec"), "exec");
        assert_eq!(session_kind("/api/v1/namespaces/d/pods/w/attach"), "attach");
        assert_eq!(
            session_kind("/api/v1/namespaces/d/pods/w/portforward"),
            "portforward"
        );
        assert_eq!(
            session_kind("/api/v1/namespaces/d/services/s:80/proxy/x"),
            "proxy"
        );
        assert_eq!(session_kind("/api/v1/namespaces/d/pods"), "other");
    }

    #[test]
    fn the_query_is_read_like_the_apiserver_does() {
        let request = parse_exec_query(
            "command=ls&command=%2Ftmp&command=a+b&container=app&stdout=true&stderr=1&stdin=",
        )
        .unwrap();
        assert_eq!(request.argv, argv(&["ls", "/tmp", "a b"]));
        assert_eq!(request.container.as_deref(), Some("app"));
        assert!(request.stdout);
        assert!(request.stderr);
        // An empty value is true for the apiserver.
        assert!(request.stdin);
        assert!(!request.tty);
    }

    #[test]
    fn false_is_only_absent_zero_or_false() {
        for value in ["0", "false", "FALSE", "False"] {
            assert!(
                !parse_exec_query(&format!("tty={value}")).unwrap().tty,
                "{value}"
            );
        }
        for value in ["1", "true", "yes", "", "no"] {
            assert!(
                parse_exec_query(&format!("tty={value}")).unwrap().tty,
                "{value}"
            );
        }
        assert!(!parse_exec_query("command=ls").unwrap().tty);
    }

    #[test]
    fn duplicates_and_semicolons_are_refused() {
        assert!(parse_exec_query("container=app&container=sidecar").is_err());
        assert!(parse_exec_query("stdin=false&stdin=true").is_err());
        assert!(parse_exec_query("command=ls;command=sh").is_err());
    }

    #[test]
    fn commands_are_bounded() {
        let many = vec!["command=x"; MAX_COMMAND_ELEMENTS + 1].join("&");
        assert!(parse_exec_query(&many).is_err());
        let long = format!("command={}", "x".repeat(MAX_COMMAND_ELEMENT_BYTES + 1));
        assert!(parse_exec_query(&long).is_err());
    }

    #[test]
    fn secrets_are_masked() {
        assert_eq!(
            mask_secrets(&argv(&["mysql", "-pS3cret", "-u", "root"])),
            argv(&["mysql", "-p***", "-u", "root"])
        );
        assert_eq!(
            mask_secrets(&argv(&[
                "tool",
                "--token",
                "abc",
                "--password=x",
                "--token=y"
            ])),
            argv(&["tool", "--token", "***", "--password=***", "--token=***"])
        );
        assert_eq!(mask_secrets(&argv(&["ls", "-la"])), argv(&["ls", "-la"]));
    }
}
