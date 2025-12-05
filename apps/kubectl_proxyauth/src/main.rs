use std::path::PathBuf;

use clap::{Parser, Subcommand};
use cli_trace::init_tracing;
use tracing::info;

use crate::ctx::CliCtx;

pub mod ctx;
pub mod error;

#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Namespace to search within
    /// If not provided, uses the default namespace
    #[arg(short, long, value_name = "NAMESPACE", default_value = "default")]
    namespace: String,

    /// Path to the kubeconfig file
    /// If not provided, uses the default kubeconfig location
    #[arg(short, long, value_name = "FILE")]
    kubeconfig: Option<PathBuf>,

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
    /// Handle Kubectl contexts
    Context {
        /// Set the current context to the specified cluster
        /// If no cluster name is provided, outputs the current context
        cluster_name: Option<String>,
        /// List all available contexts
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        list: bool,
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
    info!("CLI : {:#?}", cli);
    info!("CTX : {:#?}", ctx);
}
