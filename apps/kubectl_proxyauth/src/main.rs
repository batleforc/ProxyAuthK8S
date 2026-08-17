use clap::Parser;
use cli::{Cli, ctx::CliCtx};
use cli_trace::init_tracing;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let mut cli = Cli::parse();

    let ctx = match CliCtx::try_from(cli.clone()) {
        Ok(ctx) => ctx,
        Err(e) => {
            // Clean error + non-zero exit rather than a panic backtrace, so the
            // `kubectl` exec-credential path fails gracefully on a bad config.
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    init_tracing(
        ctx.to_tracing_verbose_level(),
        "kubectl_proxyauth".to_string(),
    );
    // NOTE: never debug-print `cli` or `ctx` here: `cli` carries the `--token`
    // value and `ctx` embeds the full kubeconfig (client keys, bearer tokens),
    // which would end up in cleartext on stderr at `-v`.

    cli.run_cli(ctx).await
}
