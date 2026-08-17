use clap::Subcommand;

use crate::{ctx::CliCtx, error::ProxyAuthK8sError};

pub mod clear;
pub mod get;
pub mod get_output;
pub mod set_def;

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigCommands {
    /// Set configuration options
    /// Default server flag are mutually exclusive with the two other flags
    /// The two other flags need to be provided together
    SetDef {
        /// Set the default server URL
        #[arg(short, long, value_name = "URL")]
        server_url: Option<String>,
        /// Set the default namespace for the selected server
        #[arg(short, long, value_name = "NAMESPACE")]
        namespace: Option<String>,
        /// Set the default server namespace
        #[arg(short, long, value_name = "SERVER_NAME")]
        default_server: Option<String>,
    },
    /// Clear configuration options
    Clear {
        /// Clear all configurations
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        all: bool,
        /// Clear configuration for a specific server URL
        #[arg(short, long, value_name = "URL")]
        server_url: Option<String>,
    },
    /// Get configuration
    /// If no flags are provided, shows the current configuration
    Get {
        /// Filter by server URL
        #[arg(short, long, value_name = "URL")]
        server_url: Option<String>,
        /// Filter by namespace
        #[arg(short, long, value_name = "NAMESPACE")]
        namespace: Option<String>,
        /// list all configurations
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        list: bool,
    },
}

impl CliCtx {
    /// Dispatch a `config` subcommand, mirroring how every other top-level
    /// command is invoked as `ctx.handle_*(...)`.
    pub fn handle_config(&mut self, command: &ConfigCommands) -> Result<(), ProxyAuthK8sError> {
        match command {
            ConfigCommands::SetDef {
                server_url,
                namespace,
                default_server,
            } => self.handle_set_def(
                server_url.as_ref(),
                namespace.as_ref(),
                default_server.as_ref(),
            ),
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
