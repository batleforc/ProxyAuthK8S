use clap::Subcommand;

use tracing::warn;

use crate::{ctx::CliCtx, error::ProxyAuthK8sError};

pub mod clear;
pub mod get;
pub mod get_output;
pub mod set_def;

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigCommands {
    /// Make a server the default one (the server of commands without --server-url).
    UseServer {
        /// URL of a server you have logged in to
        url: String,
    },
    /// Set the namespace used by default for a server's clusters.
    SetNamespace {
        /// URL of a server you have logged in to
        url: String,
        /// Namespace to use when -n is not given
        // Own id: `namespace` is the global flag's.
        #[arg(id = "default_namespace", value_name = "NAMESPACE")]
        namespace: String,
    },
    /// Deprecated: use `config use-server` or `config set-namespace`.
    #[command(hide = true)]
    SetDef {
        /// Server whose default namespace is set (with -n)
        #[arg(short, long, value_name = "URL", requires = "namespace")]
        server_url: Option<String>,
        /// Default namespace for that server (with -s)
        #[arg(short, long, value_name = "NAMESPACE", requires = "server_url")]
        namespace: Option<String>,
        /// Make this server the default one
        #[arg(
            short,
            long,
            value_name = "URL",
            conflicts_with_all = ["server_url", "namespace"]
        )]
        default_server: Option<String>,
    },
    /// Forget one server (-s) or every server (-a).
    Clear {
        /// Forget every server
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        all: bool,
        /// Forget this server
        #[arg(short, long, value_name = "URL")]
        server_url: Option<String>,
    },
    /// Show the default server's configuration, or every server's (-l).
    Get {
        /// Filter by server URL
        #[arg(short, long, value_name = "URL")]
        server_url: Option<String>,
        /// Filter by namespace
        #[arg(short, long, value_name = "NAMESPACE")]
        namespace: Option<String>,
        /// Show every server
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        list: bool,
    },
}

impl CliCtx {
    /// Dispatch a `config` subcommand, mirroring how every other top-level
    /// command is invoked as `ctx.handle_*(...)`.
    pub fn handle_config(&mut self, command: &ConfigCommands) -> Result<(), ProxyAuthK8sError> {
        match command {
            ConfigCommands::UseServer { url } => self.handle_set_def(None, None, Some(url)),
            ConfigCommands::SetNamespace { url, namespace } => {
                self.handle_set_def(Some(url), Some(namespace), None)
            }
            ConfigCommands::SetDef {
                server_url,
                namespace,
                default_server,
            } => {
                warn!(
                    "`config set-def` is deprecated: use `config use-server <URL>` or `config set-namespace <URL> <NAMESPACE>`"
                );
                self.handle_set_def(
                    server_url.as_ref(),
                    namespace.as_ref(),
                    default_server.as_ref(),
                )
            }
            ConfigCommands::Clear { all, server_url } => {
                self.handle_clear_config(*all, server_url.as_ref())
            }
            ConfigCommands::Get {
                server_url,
                namespace,
                list,
            } => self.handle_get_config(server_url.as_ref(), namespace.as_ref(), *list),
        }
    }
}
