//! The exec policy of a rule (RFC 0004): which commands, containers and
//! streams a `pods/{name}/exec` or `/attach` may use.
//!
//! A request is accepted when **one** rule accepts it entirely (its command,
//! its container and its streams together): the fields of different rules
//! never combine. The policy is checked on the query, before any byte reaches
//! the cluster; the streams are checked again on the tunnel by the api crate.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::path_matcher::segment_matches;

/// At most this many command patterns per rule.
pub const MAX_COMMANDS: usize = 64;
/// At most this many elements per command pattern.
pub const MAX_COMMAND_ELEMENTS: usize = 32;
/// At most this many container patterns per rule.
pub const MAX_CONTAINERS: usize = 32;
/// At most this many bytes per command element or container pattern.
pub const MAX_ELEMENT_BYTES: usize = 512;

/// Shells and interpreters: a pattern starting with one of them and holding a
/// wildcard runs anything (`["sh", "-c", "*"]`), so it is warned about.
const INTERPRETERS: [&str; 15] = [
    "sh", "bash", "ash", "zsh", "dash", "busybox", "env", "python*", "perl", "ruby", "node",
    "nsenter", "chroot", "xargs", "find",
];

/// Commands that read stdin: allowing stdin with one of them hands it a
/// program to run (`tar -xf -` writes files, a shell runs what it reads).
const STDIN_READERS: [&str; 6] = ["sh", "bash", "ash", "zsh", "dash", "tar"];

/// CEL rule mirroring [`CommandPattern::validate`]: `**` only last, once.
const DOUBLE_STAR_RULE: &str = concat!(
    "self.all(e, e != '**')",
    " || (self[size(self) - 1] == '**' && self.filter(e, e == '**').size() == 1)"
);

/// One element of a command pattern.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(transparent)]
pub struct CommandElement(#[schemars(length(max = 512))] pub String);

/// A command line, element by element: `*` covers a whole element or part of
/// one, `**` (last only) covers zero or more elements. No path normalisation:
/// `ls` and `/bin/ls` are two commands.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(transparent)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": DOUBLE_STAR_RULE,
        "message": "\"**\" may only be the last element of a command",
    }),
]))]
pub struct CommandPattern(#[schemars(length(min = 1, max = 32))] pub Vec<CommandElement>);

impl CommandPattern {
    fn elements(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|element| element.0.as_str())
    }

    /// Whether `argv` is a command this pattern allows.
    #[must_use]
    pub fn matches(&self, argv: &[String]) -> bool {
        let pattern: Vec<&str> = self.elements().collect();
        match pattern.split_last() {
            Some((&"**", head)) => {
                argv.len() >= head.len()
                    && head
                        .iter()
                        .zip(argv)
                        .all(|(element, arg)| segment_matches(element, arg))
            }
            _ => {
                pattern.len() == argv.len()
                    && pattern
                        .iter()
                        .zip(argv)
                        .all(|(element, arg)| segment_matches(element, arg))
            }
        }
    }

    /// Mirrors the CEL rules: not empty, bounded, `**` only last.
    pub fn validate(&self) -> Result<(), String> {
        let elements: Vec<&str> = self.elements().collect();
        if elements.is_empty() {
            return Err("a command pattern cannot be empty".to_string());
        }
        if elements.len() > MAX_COMMAND_ELEMENTS {
            return Err(format!(
                "a command pattern has more than {MAX_COMMAND_ELEMENTS} elements"
            ));
        }
        if let Some(long) = elements.iter().find(|e| e.len() > MAX_ELEMENT_BYTES) {
            return Err(format!(
                "command element \"{long}\" is longer than {MAX_ELEMENT_BYTES} bytes"
            ));
        }
        let stars = elements.iter().filter(|e| **e == "**").count();
        if stars > 1 || (stars == 1 && elements.last() != Some(&"**")) {
            return Err("\"**\" may only be the last element of a command".to_string());
        }
        Ok(())
    }

    /// Whether this pattern makes the allow-list "anything": it starts with
    /// an interpreter (or a wildcard) and holds a wildcard.
    #[must_use]
    pub fn runs_anything(&self) -> bool {
        let Some(first) = self.elements().next() else {
            return false;
        };
        let program = first.rsplit('/').next().unwrap_or(first);
        let interpreter = first.contains('*')
            || INTERPRETERS
                .iter()
                .any(|known| segment_matches(known, program));
        interpreter && self.elements().any(|e| e.contains('*'))
    }

    fn reads_stdin(&self) -> bool {
        self.elements().next().is_some_and(|first| {
            let program = first.rsplit('/').next().unwrap_or(first);
            STDIN_READERS.contains(&program)
        })
    }
}

/// What an exec or attach request asks for, read from its query the way the
/// apiserver reads `PodExecOptions` (the api crate parses it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecRequest {
    /// Every `command`, in order (empty for an attach).
    pub argv: Vec<String>,
    pub container: Option<String>,
    pub stdin: bool,
    pub stdout: bool,
    pub stderr: bool,
    pub tty: bool,
}

/// Why a rule refused an exec or attach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecDenial {
    /// What was refused, a fixed word for the audit: `command`,
    /// `container`, `stdin`, `tty`, `attach` or `no_rule`. Never the command
    /// itself, which `mask_exec_secrets` may have to hide.
    pub reason: &'static str,
    /// The message for the client.
    pub message: String,
}

impl ExecDenial {
    fn new(reason: &'static str, message: String) -> Self {
        Self { reason, message }
    }
}

/// The exec policy of one rule.
///
/// Absent from every rule a caller is subject to: exec is not restricted (it
/// becomes closed in v1.0.0). `exec: {}`: exec is possible but no command is,
/// nor stdin, tty or attach — once declared, the defaults are closed.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq, Eq)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "!(has(self.tty) && self.tty) || (has(self.stdin) && self.stdin)",
        "message": "tty needs stdin",
    }),
]))]
pub struct ExecPolicyConfiguration {
    /// Commands this rule allows, element by element (`["ls", "**"]`);
    /// absent or empty: none
    ///
    /// A pattern starting with a shell or an interpreter and holding a
    /// wildcard allows anything, and is reported in `status.warnings`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 64))]
    pub commands: Option<Vec<CommandPattern>>,
    /// Containers this rule allows (`*` allowed in a pattern); absent: all,
    /// empty: none
    ///
    /// Under a container restriction, a request must name its container
    /// (`kubectl exec -c`); kubectl always does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 32))]
    pub containers: Option<Vec<CommandElement>>,
    /// Whether stdin may be attached
    /// default: false
    #[serde(default)]
    pub stdin: bool,
    /// Whether a tty may be allocated (needs `stdin`)
    /// default: false
    #[serde(default)]
    pub tty: bool,
    /// Whether this rule allows `pods/{name}/attach` (its `commands` are
    /// ignored there)
    /// default: false
    #[serde(default)]
    pub attach: bool,
}

impl ExecPolicyConfiguration {
    /// Mirrors the CEL rules.
    pub fn validate(&self) -> Result<(), String> {
        let commands = self.commands.as_deref().unwrap_or_default();
        if commands.len() > MAX_COMMANDS {
            return Err(format!("more than {MAX_COMMANDS} command patterns"));
        }
        for command in commands {
            command.validate()?;
        }
        let containers = self.containers.as_deref().unwrap_or_default();
        if containers.len() > MAX_CONTAINERS {
            return Err(format!("more than {MAX_CONTAINERS} container patterns"));
        }
        if let Some(long) = containers.iter().find(|c| c.0.len() > MAX_ELEMENT_BYTES) {
            return Err(format!(
                "container pattern \"{}\" is longer than {MAX_ELEMENT_BYTES} bytes",
                long.0
            ));
        }
        if self.tty && !self.stdin {
            return Err("tty needs stdin".to_string());
        }
        Ok(())
    }

    /// Whether this rule accepts `request` entirely: an exec's command (or
    /// the attach flag), its container and its streams.
    ///
    /// # Errors
    ///
    /// What was refused, and a message for the client.
    pub fn accepts(&self, request: &ExecRequest, attach: bool) -> Result<(), ExecDenial> {
        if attach {
            if !self.attach {
                return Err(ExecDenial::new(
                    "attach",
                    "attach is not allowed on this cluster".to_string(),
                ));
            }
        } else if !self
            .commands
            .iter()
            .flatten()
            .any(|command| command.matches(&request.argv))
        {
            return Err(ExecDenial::new(
                "command",
                format!(
                    "command [{}] is not allowed on this cluster",
                    request.argv.join(" ")
                ),
            ));
        }
        if let Some(containers) = &self.containers {
            let Some(container) = &request.container else {
                return Err(ExecDenial::new(
                    "container",
                    "specify -c <container>: the containers are restricted".to_string(),
                ));
            };
            if !containers
                .iter()
                .any(|pattern| segment_matches(&pattern.0, container))
            {
                return Err(ExecDenial::new(
                    "container",
                    format!("container {container} is not allowed on this cluster"),
                ));
            }
        }
        if request.stdin && !self.stdin {
            return Err(ExecDenial::new(
                "stdin",
                "stdin is not allowed on this cluster".to_string(),
            ));
        }
        if request.tty && !self.tty {
            return Err(ExecDenial::new(
                "tty",
                "tty is not allowed on this cluster".to_string(),
            ));
        }
        Ok(())
    }

    /// Reconcile-time warnings about this policy, for rule `index`.
    #[must_use]
    pub fn warnings(&self, index: usize) -> Vec<String> {
        let commands = self.commands.as_deref().unwrap_or_default();
        let mut warnings = Vec::new();
        if commands.iter().any(CommandPattern::runs_anything) {
            warnings.push(format!(
                "rule {index} allows an interpreter with a wildcard: its exec allow-list allows anything"
            ));
        }
        if self.stdin && commands.iter().any(CommandPattern::reads_stdin) {
            warnings.push(format!(
                "rule {index} allows stdin to a command that reads it (a shell or tar)"
            ));
        }
        warnings
    }
}

/// The exec policy that applies to a request.
#[derive(Debug)]
pub enum ExecRules<'a> {
    /// The caller is not restricted (no rule they are subject to declares
    /// `exec`).
    Unrestricted,
    /// The rules that accept the path and declare `exec` (possibly none).
    Restricted(Vec<&'a ExecPolicyConfiguration>),
}

impl ExecRules<'_> {
    /// Whether some rule accepts `request` entirely.
    ///
    /// # Errors
    ///
    /// The reason of the closest rule (one that accepts the container, else
    /// the first), or that no rule allows exec here.
    pub fn check(&self, request: &ExecRequest, attach: bool) -> Result<(), ExecDenial> {
        let rules = match self {
            ExecRules::Unrestricted => return Ok(()),
            ExecRules::Restricted(rules) => rules,
        };
        let mut denials = Vec::new();
        for rule in rules {
            match rule.accepts(request, attach) {
                Ok(()) => return Ok(()),
                Err(denial) => {
                    let same_container = rule.containers.as_ref().is_none_or(|containers| {
                        request.container.as_ref().is_some_and(|container| {
                            containers.iter().any(|p| segment_matches(&p.0, container))
                        })
                    });
                    denials.push((same_container, denial));
                }
            }
        }
        let what = if attach { "attach" } else { "exec" };
        Err(denials
            .iter()
            .find(|(same, _)| *same)
            .or_else(|| denials.first())
            .map_or_else(
                || ExecDenial::new("no_rule", format!("no rule allows {what} on this path")),
                |(_, denial)| denial.clone(),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(elements: &[&str]) -> CommandPattern {
        CommandPattern(
            elements
                .iter()
                .map(|e| CommandElement((*e).to_string()))
                .collect(),
        )
    }

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn policy(commands: &[&[&str]], containers: Option<&[&str]>) -> ExecPolicyConfiguration {
        ExecPolicyConfiguration {
            commands: Some(commands.iter().map(|c| pattern(c)).collect()),
            containers: containers.map(|cs| {
                cs.iter()
                    .map(|c| CommandElement((*c).to_string()))
                    .collect()
            }),
            ..ExecPolicyConfiguration::default()
        }
    }

    fn exec(args: &[&str], container: Option<&str>) -> ExecRequest {
        ExecRequest {
            argv: argv(args),
            container: container.map(str::to_string),
            stdout: true,
            ..ExecRequest::default()
        }
    }

    #[test]
    fn commands_match_element_by_element() {
        assert!(pattern(&["ls", "**"]).matches(&argv(&["ls"])));
        assert!(pattern(&["ls", "**"]).matches(&argv(&["ls", "-la", "/tmp"])));
        assert!(pattern(&["cat", "/var/log/app/*"]).matches(&argv(&["cat", "/var/log/app/x.log"])));
        assert!(!pattern(&["cat", "/var/log/app/*"]).matches(&argv(&["cat", "/etc/shadow"])));
        assert!(pattern(&["/app/bin/healthcheck"]).matches(&argv(&["/app/bin/healthcheck"])));
        assert!(!pattern(&["/app/bin/healthcheck"]).matches(&argv(&["/app/bin/healthcheck", "x"])));
        // No normalisation.
        assert!(!pattern(&["ls", "**"]).matches(&argv(&["/bin/ls"])));
    }

    #[test]
    fn patterns_are_validated_like_the_cel_rules() {
        assert!(pattern(&[]).validate().is_err());
        assert!(pattern(&["**", "ls"]).validate().is_err());
        assert!(pattern(&["ls", "**", "**"]).validate().is_err());
        assert!(pattern(&["ls", "**"]).validate().is_ok());
        let tty_without_stdin = ExecPolicyConfiguration {
            tty: true,
            ..ExecPolicyConfiguration::default()
        };
        assert!(tty_without_stdin.validate().is_err());
    }

    /// The `**` CEL rule and `validate()` agree: a minimal evaluation of the
    /// rule's two branches on the same inputs.
    #[test]
    fn the_double_star_rule_agrees_with_validate() {
        for elements in [
            vec!["ls"],
            vec!["ls", "**"],
            vec!["**"],
            vec!["**", "ls"],
            vec!["ls", "**", "**"],
            vec!["a", "**", "b"],
        ] {
            let cel = elements.iter().all(|e| *e != "**")
                || (elements.last() == Some(&"**")
                    && elements.iter().filter(|e| **e == "**").count() == 1);
            assert_eq!(cel, pattern(&elements).validate().is_ok(), "{elements:?}");
        }
    }

    #[test]
    fn a_rule_accepts_command_container_and_streams_together() {
        let rule = policy(&[&["ls", "**"]], Some(&["app"]));
        assert!(
            rule.accepts(&exec(&["ls", "/tmp"], Some("app")), false)
                .is_ok()
        );
        assert!(rule.accepts(&exec(&["sh"], Some("app")), false).is_err());
        assert!(
            rule.accepts(&exec(&["ls"], Some("sidecar")), false)
                .is_err()
        );
        let missing = rule.accepts(&exec(&["ls"], None), false).unwrap_err();
        assert!(missing.message.contains("specify -c"), "{missing:?}");
        assert_eq!(missing.reason, "container");
        let mut with_stdin = exec(&["ls"], Some("app"));
        with_stdin.stdin = true;
        assert!(rule.accepts(&with_stdin, false).is_err());
    }

    #[test]
    fn empty_exec_allows_no_command_and_no_attach() {
        let rule = ExecPolicyConfiguration::default();
        assert!(rule.accepts(&exec(&["ls"], None), false).is_err());
        assert!(rule.accepts(&exec(&[], None), true).is_err());
    }

    #[test]
    fn attach_needs_the_flag_and_ignores_commands() {
        let rule = ExecPolicyConfiguration {
            attach: true,
            containers: Some(vec![CommandElement("app".into())]),
            ..ExecPolicyConfiguration::default()
        };
        assert!(rule.accepts(&exec(&[], Some("app")), true).is_ok());
        assert!(rule.accepts(&exec(&[], Some("sidecar")), true).is_err());
    }

    /// T8: two rules never combine (`ls` in `app`, `env` in `sidecar` allow
    /// neither `env` in `app` nor `ls` in `sidecar`).
    #[test]
    fn rules_never_combine() {
        let a = policy(&[&["ls"]], Some(&["app"]));
        let b = policy(&[&["env"]], Some(&["sidecar"]));
        let rules = ExecRules::Restricted(vec![&a, &b]);
        assert!(rules.check(&exec(&["ls"], Some("app")), false).is_ok());
        assert!(rules.check(&exec(&["env"], Some("sidecar")), false).is_ok());
        assert!(rules.check(&exec(&["env"], Some("app")), false).is_err());
        assert!(rules.check(&exec(&["ls"], Some("sidecar")), false).is_err());
        // The reason comes from the rule of the same container.
        let reason = rules
            .check(&exec(&["env"], Some("app")), false)
            .unwrap_err();
        assert!(reason.message.contains("command [env]"), "{reason:?}");
        assert_eq!(reason.reason, "command");
    }

    #[test]
    fn unrestricted_accepts_and_restricted_without_rules_refuses() {
        assert!(
            ExecRules::Unrestricted
                .check(&exec(&["sh"], None), false)
                .is_ok()
        );
        let reason = ExecRules::Restricted(vec![])
            .check(&exec(&["sh"], None), false)
            .unwrap_err();
        assert!(reason.message.contains("no rule allows exec"), "{reason:?}");
        assert_eq!(reason.reason, "no_rule");
    }

    #[test]
    fn interpreters_with_wildcards_are_warned_about() {
        assert!(pattern(&["sh", "-c", "*"]).runs_anything());
        assert!(pattern(&["/bin/bash", "**"]).runs_anything());
        assert!(pattern(&["python3", "**"]).runs_anything());
        assert!(pattern(&["*"]).runs_anything());
        // An exact interpreter command is an exact command.
        assert!(!pattern(&["sh", "-c", "/app/bin/rotate-logs"]).runs_anything());
        assert!(!pattern(&["ls", "**"]).runs_anything());
        let warnings = policy(&[&["sh", "-c", "*"]], None).warnings(3);
        assert!(warnings[0].contains("rule 3"), "{warnings:?}");
    }
}
