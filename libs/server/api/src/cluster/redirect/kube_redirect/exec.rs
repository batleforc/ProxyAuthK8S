//! `pods/{name}/exec` and `/attach` (RFC 0004): which paths they are, what
//! their query asks for, and how the audit records it.
//!
//! The query is read the way the apiserver reads it into `PodExecOptions`:
//! one percent-decode, `+` as a space, every `command` in order, and the
//! booleans of `Convert_Slice_string_To_bool` (the first value counts; false
//! when absent, `0` or `false` whatever the case; anything else, the empty
//! value included, is true). A duplicated parameter or a `;` is refused: the
//! proxy never reads a request differently from the apiserver.

use std::collections::BTreeMap;

use crd::security::ExecRequest;

use super::websocket::WebsocketDecoder;
use crd::security::path_matcher::percent_decode_once;

/// At most this many `command` parameters.
const MAX_COMMAND_ELEMENTS: usize = 32;

/// At most this many bytes in one `command` element.
const MAX_COMMAND_ELEMENT_BYTES: usize = 4 * 1024;

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

/// Whether `path` is `pods/{name}/attach`.
pub(super) fn is_attach_path(path: &str) -> bool {
    pod_subresource(path).as_deref() == Some("attach")
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
pub fn parse_exec_query(query: &str) -> Result<ExecRequest, String> {
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

/// The channel protocols an exec under restriction may use (RFC 0004 §5.3):
/// binary, so the channel of every message can be read.
const READABLE_PROTOCOLS: [&str; 2] = ["v5.channel.k8s.io", "v4.channel.k8s.io"];

/// Stdin, resize and close: the channels a client may write on.
const STDIN_CHANNEL: u8 = 0;
const RESIZE_CHANNEL: u8 = 4;
const CLOSE_CHANNEL: u8 = 255;

/// Whether the protocol the upstream accepted lets the filter read the
/// channels: `v5`/`v4` binary, no websocket extension.
///
/// # Errors
///
/// The reason, for the client's `403`.
pub(super) fn check_exec_protocol(
    protocol: Option<&str>,
    extensions: Option<&str>,
) -> Result<(), String> {
    if extensions.is_some_and(|e| !e.trim().is_empty()) {
        return Err("a websocket extension hides the exec channels".to_string());
    }
    match protocol.map(str::trim) {
        Some(protocol) if READABLE_PROTOCOLS.contains(&protocol) => Ok(()),
        Some(protocol) => Err(format!(
            "exec protocol {protocol} is not readable while exec is restricted; use v5.channel.k8s.io"
        )),
        None => Err(
            "an exec without a channel protocol is not readable while exec is restricted"
                .to_string(),
        ),
    }
}

/// The client half of the exec channel filter: refuses a message on a
/// channel the policy does not allow (stdin without `stdin`, resize without
/// `tty`, anything unknown), and counts the bytes of each channel.
pub struct ExecChannelFilter {
    decoder: WebsocketDecoder,
    stdin: bool,
    tty: bool,
    channel: Option<u8>,
    bytes: BTreeMap<u8, u64>,
    refused: Option<u8>,
}

impl ExecChannelFilter {
    #[must_use]
    pub fn new(stdin: bool, tty: bool) -> Self {
        Self {
            decoder: WebsocketDecoder::client(),
            stdin,
            tty,
            channel: None,
            bytes: BTreeMap::new(),
            refused: None,
        }
    }

    /// Read a chunk the client sends.
    ///
    /// # Errors
    ///
    /// The reason when the chunk opens a forbidden channel or is not a
    /// readable frame: the session must end without forwarding it.
    pub fn inspect(&mut self, chunk: &[u8]) -> Result<(), String> {
        let (stdin, tty) = (self.stdin, self.tty);
        let (channel, bytes, refused) = (&mut self.channel, &mut self.bytes, &mut self.refused);
        self.decoder.decode_messages(chunk, |data, start| {
            let payload = if start {
                let opened = data[0];
                let allowed = match opened {
                    STDIN_CHANNEL => stdin,
                    RESIZE_CHANNEL => tty,
                    CLOSE_CHANNEL => true,
                    _ => false,
                };
                if !allowed {
                    *refused = Some(opened);
                    return Err(format!("channel {opened} is not allowed on this exec"));
                }
                *channel = Some(opened);
                &data[1..]
            } else {
                data
            };
            if let Some(channel) = *channel {
                *bytes.entry(channel).or_default() += payload.len() as u64;
            }
            Ok(())
        })
    }

    /// The channel whose message ended the session, if one did.
    #[must_use]
    pub fn refused_channel(&self) -> Option<u8> {
        self.refused
    }

    /// The bytes the client sent on each channel.
    pub(super) fn bytes(&self) -> &BTreeMap<u8, u64> {
        &self.bytes
    }
}

/// The upstream half: counts the bytes of each channel the upstream sends,
/// and never blocks — a frame it cannot read only stops the counting.
pub(super) struct ExecChannelCounter {
    decoder: WebsocketDecoder,
    channel: Option<u8>,
    bytes: BTreeMap<u8, u64>,
    broken: bool,
}

impl ExecChannelCounter {
    pub(super) fn new() -> Self {
        Self {
            decoder: WebsocketDecoder::server(),
            channel: None,
            bytes: BTreeMap::new(),
            broken: false,
        }
    }

    /// Count a chunk the upstream sends.
    pub(super) fn count(&mut self, chunk: &[u8]) {
        if self.broken {
            return;
        }
        let (channel, bytes) = (&mut self.channel, &mut self.bytes);
        let read = self.decoder.decode_messages(chunk, |data, start| {
            let payload = if start {
                *channel = Some(data[0]);
                &data[1..]
            } else {
                data
            };
            if let Some(channel) = *channel {
                *bytes.entry(channel).or_default() += payload.len() as u64;
            }
            Ok(())
        });
        self.broken = read.is_err();
    }

    /// The bytes the upstream sent on each channel.
    pub(super) fn bytes(&self) -> &BTreeMap<u8, u64> {
        &self.bytes
    }
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

    fn client_message(channel: u8, payload: &[u8]) -> Vec<u8> {
        let mask = [9, 8, 7, 6];
        let mut body = vec![channel];
        body.extend_from_slice(payload);
        let mut frame = vec![0x82, 0x80 | body.len() as u8];
        frame.extend_from_slice(&mask);
        frame.extend(body.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        frame
    }

    /// T4: stdin data while stdin is not allowed ends the session.
    #[test]
    fn a_forbidden_channel_is_refused() {
        let mut filter = ExecChannelFilter::new(false, false);
        assert!(
            filter
                .inspect(&client_message(STDIN_CHANNEL, b"rm -rf /"))
                .is_err()
        );
        assert_eq!(filter.refused_channel(), Some(STDIN_CHANNEL));
        let mut filter = ExecChannelFilter::new(true, false);
        assert!(
            filter
                .inspect(&client_message(STDIN_CHANNEL, b"ls"))
                .is_ok()
        );
        assert!(
            filter
                .inspect(&client_message(RESIZE_CHANNEL, b"{}"))
                .is_err()
        );
        // Close is always allowed; stdout from a client is not.
        assert!(
            ExecChannelFilter::new(false, false)
                .inspect(&client_message(CLOSE_CHANNEL, &[0]))
                .is_ok()
        );
        assert!(
            ExecChannelFilter::new(true, true)
                .inspect(&client_message(1, b"x"))
                .is_err()
        );
    }

    #[test]
    fn channels_are_counted_each_way() {
        let mut filter = ExecChannelFilter::new(true, false);
        filter
            .inspect(&client_message(STDIN_CHANNEL, b"abc"))
            .unwrap();
        assert_eq!(filter.bytes().get(&STDIN_CHANNEL), Some(&3));
        let mut counter = ExecChannelCounter::new();
        counter.count(&[0x82, 4, 1, b'o', b'u', b't']);
        counter.count(&[0x82, 3, 2, b'e', b'r']);
        assert_eq!(counter.bytes().get(&1), Some(&3));
        assert_eq!(counter.bytes().get(&2), Some(&2));
    }

    /// T5: under restriction, only binary channel protocols without
    /// extension are readable.
    #[test]
    fn only_readable_protocols_are_accepted() {
        assert!(check_exec_protocol(Some("v5.channel.k8s.io"), None).is_ok());
        assert!(check_exec_protocol(Some("v4.channel.k8s.io"), Some("")).is_ok());
        assert!(check_exec_protocol(Some("v4.base64.channel.k8s.io"), None).is_err());
        assert!(check_exec_protocol(None, None).is_err());
        assert!(
            check_exec_protocol(Some("v5.channel.k8s.io"), Some("permessage-deflate")).is_err()
        );
    }

    /// T3: what the apiserver reads from a query (table generated from its
    /// own conversion helpers, `task gen:request-info-reference`) is what
    /// the proxy reads, or the proxy refuses the query outright (a `;` or a
    /// repeated parameter, where the apiserver would keep the first value).
    #[test]
    fn the_query_is_read_like_the_reference() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../../crd/tests/exec_query_reference.json"
        ))
        .unwrap();
        assert!(cases.len() > 30);
        for case in cases {
            let query = case["query"].as_str().unwrap();
            match parse_exec_query(query) {
                Ok(request) => {
                    let expected: Vec<String> = case["command"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap().to_string())
                        .collect();
                    assert_eq!(request.argv, expected, "{query}");
                    let container = case["container"].as_str().unwrap();
                    assert_eq!(
                        request.container.as_deref().unwrap_or_default(),
                        container,
                        "{query}"
                    );
                    for (field, ours) in [
                        ("stdin", request.stdin),
                        ("stdout", request.stdout),
                        ("stderr", request.stderr),
                        ("tty", request.tty),
                    ] {
                        assert_eq!(ours, case[field].as_bool().unwrap(), "{field} in {query}");
                    }
                }
                Err(_) => {
                    let repeated = ["container", "stdin", "stdout", "stderr", "tty"]
                        .iter()
                        .any(|key| {
                            query
                                .split('&')
                                .filter(|pair| pair.split('=').next() == Some(*key))
                                .count()
                                > 1
                        });
                    assert!(query.contains(';') || repeated, "refused: {query}");
                }
            }
        }
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
