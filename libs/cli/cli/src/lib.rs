//! Core logic for the `kubectl_proxyauth` CLI.
//!
//! Implements the command handlers (login, logout, config, context, get) and
//! the persisted configuration model that the thin `kubectl_proxyauth` binary
//! dispatches to.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tracing::{debug, warn};

use crate::{
    config_cmd::ConfigCommands,
    ctx::{CliCtx, ContextFormat},
};

pub mod cli_config;
pub mod config_cmd;
pub mod context;
pub mod ctx;
pub mod error;
pub mod get;
pub mod helper;
pub mod login;
pub mod logout;
pub mod output;

/// Kubectl `ProxyAuth` CLI
#[derive(Parser, Debug, Clone)]
#[command(
    name = "Kubectl_ProxyAuthK8S",
    version,
    about,
    long_about = "A command-line tool to interact with ProxyAuthK8S for managing authentication to Kubernetes clusters.",
    arg_required_else_help = true,
    after_help = "Made with ❤️  and too much ☕ by Batleforc",
    before_help = include_str!("../../../../.docs/public/banner.art"),
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
    Context {
        /// Get the context for a specific cluster
        context_name: Option<String>,
        /// List all available contexts
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        list: bool,
        /// Set the current context to the specified cluster
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
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

impl Cli {
    pub async fn run_cli(&mut self, mut ctx: CliCtx) -> std::process::ExitCode {
        // Match and execute the appropriate command
        match &self.command {
            Some(Commands::Get { cluster_name }) => {
                debug!("Getting cluster info for: {:?}", cluster_name);
                ctx.handle_get_clusters(cluster_name.clone()).await;
            }
            Some(Commands::Login {
                cluster_name,
                token,
            }) => {
                // Never log the token value; only whether one was supplied.
                debug!(
                    "Logging in to cluster: {:?} (token provided: {})",
                    cluster_name,
                    token.is_some()
                );
                ctx.handle_login(cluster_name.clone(), token.clone()).await;
            }
            Some(Commands::Logout { cluster_name }) => {
                debug!("Logging out from cluster: {:?}", cluster_name);
                ctx.handle_logout(cluster_name.clone());
            }
            Some(Commands::Cache { command }) => match command {
                CacheCommands::Clear => {
                    debug!("Clearing all cached tokens");
                    ctx.handle_cache_clear();
                }
            },
            Some(Commands::GetToken { cluster_name }) => {
                debug!("Getting token for cluster: {:?}", cluster_name);
                // Propagate a non-zero exit code so `kubectl` sees the exec-credential
                // plugin failed instead of treating a 0 exit as "no credential".
                if ctx.handle_get_token(cluster_name.clone()).await.is_err() {
                    return std::process::ExitCode::FAILURE;
                }
            }
            Some(Commands::Context {
                context_name,
                list,
                set,
            }) => {
                debug!(
                    "Handling context for cluster: {:?}, list: {}, set: {}",
                    context_name, list, set
                );
                ctx.handle_context(context_name.clone(), *list, *set);
            }
            Some(Commands::Config { command }) => {
                debug!("Handling config command: {:?}", command);
                if let Some(command) = command {
                    ctx.handle_config(command);
                } else {
                    warn!("No config subcommand provided. Use --help for more information.");
                }
            }
            None => {
                // If no subcommand is provided, you can show help or a default action
                warn!("No command provided. Use --help for more information.");
            }
        }
        std::process::ExitCode::SUCCESS
    }
}
