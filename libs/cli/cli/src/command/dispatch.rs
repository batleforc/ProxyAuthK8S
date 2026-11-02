//! Maps a parsed [`Cli`] onto the matching [`CliCtx`] handler and turns the
//! outcome into a process exit code.

use tracing::{debug, warn};

use crate::{
    CacheCommands, Cli, Commands, cli_config::browser::BrowserFlag, ctx::CliCtx,
    error::ProxyAuthK8sError,
};

impl Cli {
    pub async fn run_cli(&mut self, mut ctx: CliCtx) -> std::process::ExitCode {
        // Each handler returns `Result<(), ProxyAuthK8sError>` and leaves the
        // error to this function: it is printed once, with its code and hint,
        // and turned into a non-zero exit code.
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
            Some(Commands::Status) => ctx.handle_status(),
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
                // A non-zero exit lets `kubectl` see the exec-credential plugin
                // failed instead of treating a 0 exit as "no credential".
                ctx.handle_get_token(cluster_name.clone()).await
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
                if *list {
                    warn!("`context --list` is deprecated: use `get-contexts`");
                }
                if *set {
                    warn!("`context --set` is deprecated: use `use-context <NAME>`");
                }
                ctx.handle_context(context_name.clone(), *list, *set)
            }
            Some(Commands::UseContext { context_name }) => {
                ctx.handle_context(Some(context_name.clone()), false, true)
            }
            Some(Commands::GetContexts) => ctx.handle_context(None, true, false),
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
            Err(e) => {
                eprintln!("{}", render_error(&e));
                std::process::ExitCode::FAILURE
            }
        }
    }
}

/// The text printed on stderr for a failed command: the error, then its hint.
fn render_error(e: &ProxyAuthK8sError) -> String {
    match e.hint() {
        Some(hint) => format!("error: {e}\n  hint: {hint}"),
        None => format!("error: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::process::ExitCode;

    /// Run `args` as typed, on a fresh context with nothing logged in.
    async fn run(dir: &std::path::Path, args: &[&str]) -> ExitCode {
        let mut cli =
            Cli::try_parse_from(std::iter::once("kubectl-proxyauth").chain(args.iter().copied()))
                .unwrap();
        cli.run_cli(CliCtx::for_test_in(dir)).await
    }

    #[tokio::test]
    async fn every_command_is_routed_and_its_outcome_is_the_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        for args in [
            &["cache", "clear"][..],
            &["context"],
            &["ctx", "--list"],
            &["get-contexts"],
            &["config", "get"],
            &["config", "get", "--list"],
        ] {
            assert_eq!(run(dir.path(), args).await, ExitCode::SUCCESS, "{args:?}");
        }
        // Nothing logged in, or nothing to act on: a failure, not a 0.
        for args in [
            &["get"][..],
            &["status"],
            &["login"],
            &["login", "prod", "--browser", "none"],
            &["logout"],
            &["get-token", "prod"],
            &["use-context", "nope"],
            &["ctx", "--set"],
            &["config", "use-server", "https://nope.example"],
            &["config", "set-def", "-d", "https://nope.example"],
        ] {
            assert_eq!(run(dir.path(), args).await, ExitCode::FAILURE, "{args:?}");
        }
    }

    #[tokio::test]
    async fn no_command_is_a_hint_not_a_failure() {
        // clap prints the help first; these shapes only exist when the CLI is
        // built by hand, and must not fail.
        let dir = tempfile::tempdir().unwrap();
        for command in [None, Some(Commands::Config { command: None })] {
            let mut cli = Cli::try_parse_from(["kubectl-proxyauth", "status"]).unwrap();
            cli.command = command;
            assert_eq!(
                cli.run_cli(CliCtx::for_test_in(dir.path())).await,
                ExitCode::SUCCESS
            );
        }
    }

    #[test]
    fn a_failure_is_printed_once_with_its_code_and_hint() {
        assert_eq!(
            render_error(&ProxyAuthK8sError::NotLoggedIn),
            "error: ERR000021: Not logged in to any ProxyAuthK8S server\n  hint: log in first: kubectl proxyauth login --server-url https://<server>"
        );
        assert_eq!(
            render_error(&ProxyAuthK8sError::InvalidUsage("bad".to_string())),
            "error: ERR000017: bad"
        );
    }
}
