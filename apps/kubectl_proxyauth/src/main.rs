use clap::Parser;
use cli::{ctx::CliCtx, Cli};
use cli_trace::init_tracing;
use tracing::debug;

#[tokio::main]
async fn main() {
    let mut cli = Cli::parse();

    let ctx = CliCtx::from(cli.clone());

    init_tracing(
        ctx.to_tracing_verbose_level(),
        "kubectl_proxyauth".to_string(),
    );
    debug!("CLI : {:#?}", cli);
    debug!("CTX : {:#?}", ctx);

    cli.run_cli(ctx).await;
}
