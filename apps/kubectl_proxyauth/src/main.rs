use std::path::PathBuf;

use clap::{Parser, Subcommand};
use cli_trace::init_tracing;
use tracing::{debug, info, warn};

use crate::ctx::CliCtx;

pub mod cli_config;
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
    arg_required_else_help = true
)]
struct Cli {
    /// Namespace to search within
    /// If not provided, uses the default namespace
    #[arg(short, long, value_name = "NAMESPACE", default_value = "default")]
    namespace: String,

    /// Path to the kubeconfig file
    /// If not provided, uses the default kubeconfig location
    /// Default location is `$KUBECONFIG` env var or `$HOME/.kube/config`
    #[arg(short, long, value_name = "FILE")]
    kubeconfig: Option<PathBuf>,

    /// CLI configuration file path
    /// If not provided, uses the default configuration location
    /// Default location is `$HOME/.kube/proxyauth_config.yaml
    #[arg(short, long, value_name = "FILE")]
    proxy_auth_config: Option<PathBuf>,

    /// Context to use/override from kubeconfig
    #[arg(short, long, value_name = "CONTEXT")]
    context: Option<String>,

    /// Verbosity level
    /// By default, logging is set to 'info'
    /// Level are as follows:
    /// -v : debug
    /// -vv : trace
    /// -vvv : all logs including very verbose logs
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: Option<u8>,

    /// ProxyAuthK8S server URL
    /// If not provided, uses the default URL http://localhost:8080
    #[arg(
        short,
        long,
        value_name = "URL",
        default_value = "http://localhost:8080"
    )]
    server_url: String,

    /// Output format
    /// Specify the output format (e.g., json, yaml, table)
    /// Default is `table`
    #[arg(short, long, value_name = "FORMAT", default_value = "table")]
    format: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
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
        /// Set the current context to the specified cluster
        /// If no cluster name is provided, outputs the current context
        cluster_name: Option<String>,
        /// List all available contexts
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        list: bool,
    },
    /// Configuration management
    Config {
        /// Set the ProxyAuthK8S server URL in the configuration
        #[arg(short, long, value_name = "URL")]
        server_url: Option<String>,

        /// Set the default namespace in the configuration
        #[arg(short, long, value_name = "NAMESPACE")]
        namespace: Option<String>,

        /// Clear the configuration file, resetting all settings to defaults
        #[arg(long, action = clap::ArgAction::SetTrue)]
        clear: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    // You can see how many times a particular flag or argument occurred
    // Note, only flags can have multiple occurrences
    let ctx = CliCtx::from(cli.clone());

    init_tracing(
        ctx.to_tracing_verbose_level(),
        "kubectl_proxyauth".to_string(),
    );
    debug!("CLI : {:#?}", cli);
    debug!("CTX : {:#?}", ctx);

    // Match and execute the appropriate command
    match &cli.command {
        Some(Commands::Get { cluster_name }) => {
            //ctx.handle_get_clusters(cluster_name.clone());
            info!("Getting cluster info for: {:?}", cluster_name);
        }
        Some(Commands::Login {
            cluster_name,
            token,
        }) => {
            //ctx.handle_login(cluster_name.clone());
            info!(
                "Logging in to cluster: {:?} with token: {:?}",
                cluster_name, token
            );
        }
        Some(Commands::Logout { cluster_name }) => {
            //ctx.handle_logout(cluster_name.clone());
            info!("Logging out from cluster: {:?}", cluster_name);
        }
        Some(Commands::Cache { clear }) => {
            //ctx.handle_cache(*clear);
            info!("Handling cache clear: {}", clear);
        }
        Some(Commands::GetToken { cluster_name }) => {
            //ctx.handle_get_token(cluster_name.clone());
            // Detect if env var KUBERNETES_EXEC_INFO is set, change context accordingly
            info!("Getting token for cluster: {:?}", cluster_name);
        }
        Some(Commands::Context { cluster_name, list }) => {
            //ctx.handle_context(cluster_name.clone(), *list);
            info!(
                "Handling context for cluster: {:?}, list: {}",
                cluster_name, list
            );
        }
        Some(Commands::Config {
            server_url,
            namespace,
            clear,
        }) => {
            //ctx.handle_config(server_url.clone(), namespace.clone(), *clear);
            info!(
                "Handling config with server_url: {:?}, namespace: {:?}, clear: {}",
                server_url, namespace, clear
            );
        }
        None => {
            // If no subcommand is provided, you can show help or a default action
            warn!("No command provided. Use --help for more information.");
        }
    }
}
