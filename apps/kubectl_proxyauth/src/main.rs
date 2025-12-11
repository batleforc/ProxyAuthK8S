use clap::Parser;
use cli::{ctx::CliCtx, Cli, Commands};
use cli_trace::init_tracing;
use tracing::{debug, info, warn};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let mut ctx = CliCtx::from(cli.clone());

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
            //ctx.handle_config(server_url.clone(), namespace.clone(), *clear);
            debug!("Handling config command: {:?}", command);
            if let Some(command) = command {
                command.handle_config_commands(&mut ctx);
            } else {
                warn!("No config subcommand provided. Use --help for more information.");
            }
        }
        None => {
            // If no subcommand is provided, you can show help or a default action
            warn!("No command provided. Use --help for more information.");
        }
    }
}
