//! The clap command surface: the global flags and the subcommand tree.
//!
//! Only the *shape* of the CLI lives here. Turning a parsed command into a call
//! on [`CliCtx`](crate::ctx::CliCtx) is `dispatch.rs`, and the work itself lives
//! in the per-command modules (`login`, `logout`, `get`, `context`, `config_cmd`).

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::{config_cmd::ConfigCommands, ctx::ContextFormat};

mod dispatch;

/// Kubectl `ProxyAuth` CLI
#[derive(Parser, Debug, Clone)]
#[command(
    name = "Kubectl_ProxyAuthK8S",
    version,
    about,
    long_about = "A command-line tool to interact with ProxyAuthK8S for managing authentication to Kubernetes clusters.",
    arg_required_else_help = true,
    after_help = "Made with ❤️  and too much ☕ by Batleforc",
    before_help = include_str!("../../../../../.docs/public/banner.art"),
)]
pub struct Cli {
    /// Namespace to search within
    /// If not provided, uses the default namespace
    #[arg(
        short,
        long,
        global = true,
        value_name = "NAMESPACE",
        default_value = ""
    )]
    pub namespace: String,

    /// Path to the kubeconfig file
    /// If not provided, uses the default kubeconfig location
    /// Default location is `$KUBECONFIG` env var or `$HOME/.kube/config`
    #[arg(short, global = true, long, value_name = "FILE")]
    pub kubeconfig: Option<PathBuf>,

    /// CLI configuration file path
    /// If not provided, uses the default configuration location
    /// Default location is `$HOME/.kube/proxyauth_config.yaml
    #[arg(short, global = true, long, value_name = "FILE")]
    pub proxy_auth_config: Option<PathBuf>,

    /// Context to use/override from kubeconfig
    #[arg(short, global = true, long, value_name = "CONTEXT")]
    pub context: Option<String>,
    /// Verbosity level
    /// By default, logging is set to 'info'
    /// Level are as follows:
    /// -v : debug
    /// -vv : trace
    /// -vvv : all logs including very verbose logs
    #[arg(short,global = true, long, action = clap::ArgAction::Count)]
    pub verbose: Option<u8>,

    /// `ProxyAuthK8S` server URL
    #[arg(short, long, global = true, value_name = "URL", default_value = "")]
    pub server_url: String,

    /// Output format
    /// Specify the output format (e.g., json, yaml, table)
    /// Default is `table`
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
    /// Get auth clusters
    /// If no flags are provided, lists all clusters
    Get {
        /// Get a specific cluster by name
        cluster_name: Option<String>,
    },
    /// Login either to `ProxyAuthK8S` server or to a specific cluster
    Login {
        /// Cluster name to login to
        cluster_name: Option<String>,
        /// Optional token for authentication
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
    /// Logout either from `ProxyAuthK8S` server or from a specific cluster
    Logout {
        /// Cluster name to logout from
        cluster_name: Option<String>,
    },
    /// Manage cached authentication tokens
    Cache {
        #[command(subcommand)]
        command: CacheCommands,
    },
    /// Retrieve the current authentication token for a specific cluster
    GetToken {
        /// Cluster name to retrieve the token for
        cluster_name: Option<String>,
    },
    /// Handle Kubectl contexts
    #[command(alias = "ctx")]
    Context {
        /// Get the context for a specific cluster
        context_name: Option<String>,
        /// List all available contexts
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        list: bool,
        /// Set the current context to the specified cluster
        // No `short`: `-s` is already the global `--server-url`; clap rejects the
        // duplicate short within this subcommand. Long-only `--set`.
        #[arg(long, action = clap::ArgAction::SetTrue)]
        set: bool,
    },
    /// Configuration management
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
    fn a_browser_argument_needs_a_browser() {
        assert!(login_browser(&["--browser-arg", "--kiosk"]).is_err());
    }
}
