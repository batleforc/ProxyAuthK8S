//! Maps a parsed [`Cli`] onto the matching [`CliCtx`] handler and turns the
//! outcome into a process exit code.

use tracing::{debug, warn};

use crate::{
    CacheCommands, Cli, Commands, cli_config::browser::BrowserFlag, ctx::CliCtx,
    error::ProxyAuthK8sError,
};

impl Cli {
    pub async fn run_cli(&mut self, mut ctx: CliCtx) -> std::process::ExitCode {
        // Each handler returns `Result<(), ProxyAuthK8sError>` and logs its own
        // error detail; here we only translate the outcome into the process exit
        // code so a failed command exits non-zero (previously every command but
        // `get-token` exited 0 regardless of failure).
        let result: Result<(), ProxyAuthK8sError> = match &self.command {
            Some(Commands::Get { cluster_name }) => {
                debug!("Getting cluster info for: {:?}", cluster_name);
                ctx.handle_get_clusters(cluster_name.clone()).await
            }
            Some(Commands::Login {
                cluster_name,
                token,
                certificate_authority,
                browser,
                browser_args,
            }) => {
                ctx.browser_flag = BrowserFlag::from_args(browser.as_deref(), browser_args);
                // Never log the token value; only whether one was supplied.
                debug!(
                    "Logging in to cluster: {:?} (token provided: {})",
                    cluster_name,
                    token.is_some()
                );
                ctx.handle_login(
                    cluster_name.clone(),
                    token.clone(),
                    certificate_authority.as_deref(),
                )
                .await
            }
            Some(Commands::Logout { cluster_name }) => {
                debug!("Logging out from cluster: {:?}", cluster_name);
                ctx.handle_logout(cluster_name.clone())
            }
            Some(Commands::Cache { command }) => match command {
                CacheCommands::Clear => {
                    debug!("Clearing all cached tokens");
                    ctx.handle_cache_clear()
                }
            },
            Some(Commands::GetToken { cluster_name }) => {
                debug!("Getting token for cluster: {:?}", cluster_name);
                // Propagate a non-zero exit code so `kubectl` sees the exec-credential
                // plugin failed instead of treating a 0 exit as "no credential".
                return if ctx.handle_get_token(cluster_name.clone()).await.is_err() {
                    std::process::ExitCode::FAILURE
                } else {
                    std::process::ExitCode::SUCCESS
                };
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
                ctx.handle_context(context_name.clone(), *list, *set)
            }
            Some(Commands::Config { command }) => {
                debug!("Handling config command: {:?}", command);
                if let Some(command) = command {
                    ctx.handle_config(command)
                } else {
                    warn!("No config subcommand provided. Use --help for more information.");
                    // No subcommand is a usage hint, not a failed operation.
                    Ok(())
                }
            }
            None => {
                // If no subcommand is provided, show a hint (not a failure).
                warn!("No command provided. Use --help for more information.");
                Ok(())
            }
        };
        match result {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(_) => std::process::ExitCode::FAILURE,
        }
    }
}
