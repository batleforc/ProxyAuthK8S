use clap::Subcommand;
use tracing::error;

pub mod clear;
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
        #[arg(long, value_name = "URL")]
        server_url: Option<String>,
        /// Filter by namespace
        #[arg(short, long, value_name = "NAMESPACE")]
        namespace: Option<String>,
        /// list all configurations
        #[arg(short, long, action = clap::ArgAction::SetTrue)]
        list: bool,
    },
}

impl ConfigCommands {
    pub fn handle_config_commands(&self, ctx: &mut crate::ctx::CliCtx) {
        match self {
            ConfigCommands::SetDef { .. } => {
                self.handle_set_def(ctx);
            }
            ConfigCommands::Clear { .. } => {
                self.handle_clear(ctx);
            }
            _ => {
                error!("Config command not implemented yet.");
            }
        }
    }
}
