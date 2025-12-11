use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::{config::ConfigCommands, ctx::ContextFormat};

pub mod cli_config;
pub mod config;
pub mod context;
pub mod ctx;
pub mod error;

/// Kubectl ProxyAuth CLI
#[derive(Parser, Debug, Clone)]
#[command(
    name = "Kubectl_ProxyAuthK8S",
    version,
    about,
    long_about = "A command-line tool to interact with ProxyAuthK8S for managing authentication to Kubernetes clusters.",
    arg_required_else_help = true,
    after_help = "Made with ❤️  and too much ☕ by Batleforc",
    before_help = include_str!("../../../banner.art"),
)]
pub struct Cli {
    /// Namespace to search within
    /// If not provided, uses the default namespace
    #[arg(
        short,
        long,
        global = true,
        value_name = "NAMESPACE",
        default_value = "default"
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

    /// ProxyAuthK8S server URL
    /// If not provided, uses the default URL http://localhost:8080
    #[arg(
        short,
        long,
        global = true,
        value_name = "URL",
        default_value = "http://localhost:8080"
    )]
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
    /// Login either to ProxyAuthK8S server or to a specific cluster
    Login {
        /// Cluster name to login to
        cluster_name: Option<String>,
        /// Optional token for authentication
        #[arg(short, long, value_name = "TOKEN")]
        token: Option<String>,
    },
    /// Logout either from ProxyAuthK8S server or from a specific cluster
    Logout {
        /// Cluster name to logout from
        cluster_name: Option<String>,
    },
    /// Clear cached authentication tokens
    Cache {
        /// Clear all cached tokens
        clear: bool,
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
