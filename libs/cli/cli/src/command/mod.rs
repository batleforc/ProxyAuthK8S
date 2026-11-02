//! The clap command surface: the global flags and the subcommand tree.
//!
//! Only the *shape* of the CLI lives here. Turning a parsed command into a call
//! on [`CliCtx`](crate::ctx::CliCtx) is `dispatch.rs`, and the work itself lives
//! in the per-command modules (`login`, `logout`, `get`, `context`, `config_cmd`).

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::{config_cmd::ConfigCommands, ctx::ContextFormat};

mod dispatch;

/// Log in to Kubernetes clusters exposed through a ProxyAuthK8S server.
#[derive(Parser, Debug, Clone)]
#[command(
    name = "kubectl-proxyauth",
    // What users type: the binary is run as a kubectl plugin.
    bin_name = "kubectl proxyauth",
    version,
    about,
    long_about = "Log in to Kubernetes clusters exposed through a ProxyAuthK8S server.\n\n\
                  Start with `kubectl proxyauth login --server-url https://<server>`, list the \
                  clusters with `kubectl proxyauth get`, then `kubectl proxyauth login <cluster>`.",
    arg_required_else_help = true,
    after_help = "Made with ❤️  and too much ☕ by Batleforc",
    before_help = include_str!("../../../../../docs/public/banner.art"),
)]
pub struct Cli {
    /// Namespace of the cluster (default: the server's default namespace).
    #[arg(
        short,
        long,
        global = true,
        value_name = "NAMESPACE",
        default_value = "",
        hide_default_value = true
    )]
    pub namespace: String,

    /// Kubeconfig file (default: `$KUBECONFIG`, else `~/.kube/config`).
    #[arg(short, global = true, long, value_name = "FILE")]
    pub kubeconfig: Option<PathBuf>,

    /// CLI configuration file (default: `~/.kube/proxyauth_config.yaml`).
    #[arg(short, global = true, long, value_name = "FILE")]
    pub proxy_auth_config: Option<PathBuf>,

    /// More logs: -v for debug, -vv for trace.
    #[arg(short,global = true, long, action = clap::ArgAction::Count)]
    pub verbose: Option<u8>,

    /// ProxyAuthK8S server URL (default: the server of the last login).
    #[arg(
        short,
        long,
        global = true,
        value_name = "URL",
        default_value = "",
        hide_default_value = true
    )]
    pub server_url: String,

    /// Output format of `get`, `context` and `config get`.
    #[arg(
        short,
        global = true,
        long,
        value_name = "FORMAT",
        default_value = "table"
    )]
    pub format: ContextFormat,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// List the clusters visible to you on the server, or one by name.
    Get {
        /// Get a specific cluster by name
        cluster_name: Option<String>,
    },
    /// Show your login state on each server (local only, no network call).
    Status,
    /// Log in to the server (no argument) or to one of its clusters.
    Login {
        /// Cluster name to login to
        cluster_name: Option<String>,
        /// Token to use instead of the SSO login (scripts, CI). Prefer the
        /// prompt: a token on the command line shows in `ps` and history.
        #[arg(short, long, value_name = "TOKEN")]
        token: Option<String>,
        /// PEM file of the CA that signed the `ProxyAuthK8S` server's TLS
        /// certificate, when the system does not trust it (self-signed or
        /// internal CA). Saved for the server and written to the kubeconfig.
        #[arg(long, value_name = "FILE")]
        certificate_authority: Option<PathBuf>,
        /// Browser for the SSO login, instead of the system's default one
        /// (e.g. a corporate browser the identity provider requires): a
        /// program name or full path, `none` to only print the URL, or
        /// `default` to go back to the system browser. Saved for the server,
        /// so later logins through it use it too. `PROXYAUTH_BROWSER`
        /// overrides the saved one for a single run.
        #[arg(long, value_name = "PROGRAM")]
        browser: Option<String>,
        /// Argument passed to `--browser` (repeatable). `{url}` is replaced by
        /// the login URL; without it the URL is passed last.
        #[arg(
            long = "browser-arg",
            value_name = "ARG",
            requires = "browser",
            allow_hyphen_values = true
        )]
        browser_args: Vec<String>,
    },
    /// Log out of the server (no argument) or of one of its clusters.
    Logout {
        /// Cluster name to logout from
        cluster_name: Option<String>,
    },
    /// Manage cached authentication tokens
    Cache {
        #[command(subcommand)]
        command: CacheCommands,
    },
    /// Print a cluster token as a kubectl ExecCredential (called by kubectl).
    GetToken {
        /// Cluster name to retrieve the token for
        cluster_name: Option<String>,
    },
    /// Show the current kubectl context, or the one named.
    #[command(alias = "ctx")]
    Context {
        /// Context to show
        context_name: Option<String>,
        /// Deprecated: use `get-contexts`
        #[arg(short, long, hide = true, action = clap::ArgAction::SetTrue)]
        list: bool,
        /// Deprecated: use `use-context <NAME>`
        // No `short`: `-s` is already the global `--server-url`.
        #[arg(long, hide = true, action = clap::ArgAction::SetTrue)]
        set: bool,
    },
    /// Switch kubectl to another context (like `kubectl config use-context`).
    UseContext {
        /// Context name, e.g. `<namespace>-<cluster>-context` after a login
        context_name: String,
    },
    /// List the kubeconfig contexts, flagging those served by ProxyAuthK8S.
    GetContexts,
    /// Show or change the CLI configuration (known servers, defaults).
    #[command(arg_required_else_help = true)]
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommands>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum CacheCommands {
    /// Clear all cached authentication tokens
    Clear,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn login_browser(args: &[&str]) -> Result<(Option<String>, Vec<String>), clap::Error> {
        let cli = Cli::try_parse_from(
            ["kubectl-proxyauth", "login", "prod"]
                .into_iter()
                .chain(args.iter().copied()),
        )?;
        match cli.command {
            Some(Commands::Login {
                browser,
                browser_args,
                ..
            }) => Ok((browser, browser_args)),
            other => panic!("expected a login, got {other:?}"),
        }
    }

    #[test]
    fn login_takes_a_browser_and_its_arguments() {
        assert_eq!(login_browser(&[]).unwrap(), (None, vec![]));
        assert_eq!(
            login_browser(&[
                "--browser",
                "/opt/corp/browser",
                "--browser-arg",
                "--profile-directory=Work",
                "--browser-arg=--new-window",
            ])
            .unwrap(),
            (
                Some("/opt/corp/browser".to_string()),
                vec![
                    "--profile-directory=Work".to_string(),
                    "--new-window".to_string()
                ]
            )
        );
    }

    #[test]
    fn the_command_tree_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
        assert_eq!(Cli::command().get_bin_name(), Some("kubectl proxyauth"));
    }

    #[test]
    fn the_removed_context_flag_is_refused() {
        // `-c/--context` was parsed then ignored; it now fails loudly.
        for flag in ["-c", "--context"] {
            assert!(Cli::try_parse_from(["kubectl-proxyauth", "get", flag, "x"]).is_err());
        }
    }

    #[test]
    fn set_def_flag_combinations_are_checked_by_clap() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(
                ["kubectl-proxyauth", "config", "set-def"]
                    .into_iter()
                    .chain(args.iter().copied()),
            )
        };
        assert!(parse(&["-d", "https://a"]).is_ok());
        assert!(parse(&["-s", "https://a", "-n", "ns"]).is_ok());
        assert!(parse(&["-s", "https://a"]).is_err());
        assert!(parse(&["-n", "ns"]).is_err());
        assert!(parse(&["-d", "https://a", "-s", "https://b", "-n", "ns"]).is_err());
    }

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("kubectl-proxyauth").chain(args.iter().copied()))
    }

    #[test]
    fn config_takes_kubectl_style_verbs() {
        match parse(&["config", "use-server", "https://a"])
            .unwrap()
            .command
        {
            Some(Commands::Config {
                command: Some(ConfigCommands::UseServer { url }),
            }) => assert_eq!(url, "https://a"),
            other => panic!("expected use-server, got {other:?}"),
        }
        // `-n` stays the global flag: the positional namespace has its own id.
        let cli = parse(&["config", "set-namespace", "https://a", "team-a", "-n", "x"]).unwrap();
        assert_eq!(cli.namespace, "x");
        match cli.command {
            Some(Commands::Config {
                command: Some(ConfigCommands::SetNamespace { url, namespace }),
            }) => assert_eq!((url.as_str(), namespace.as_str()), ("https://a", "team-a")),
            other => panic!("expected set-namespace, got {other:?}"),
        }
        assert!(parse(&["config", "set-namespace", "https://a"]).is_err());
    }

    #[test]
    fn contexts_take_kubectl_style_verbs_and_the_old_flags_still_parse() {
        assert!(matches!(
            parse(&["use-context", "team-a-prod-context"]).unwrap().command,
            Some(Commands::UseContext { context_name }) if context_name == "team-a-prod-context"
        ));
        assert!(parse(&["use-context"]).is_err());
        assert!(matches!(
            parse(&["get-contexts"]).unwrap().command,
            Some(Commands::GetContexts)
        ));
        // Deprecated, kept so existing scripts keep working.
        assert!(matches!(
            parse(&["ctx", "--set", "c"]).unwrap().command,
            Some(Commands::Context { set: true, .. })
        ));
        assert!(matches!(
            parse(&["ctx", "--list"]).unwrap().command,
            Some(Commands::Context { list: true, .. })
        ));
    }

    #[test]
    fn a_browser_argument_needs_a_browser() {
        assert!(login_browser(&["--browser-arg", "--kiosk"]).is_err());
    }
}
