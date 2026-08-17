//! Logging and tracing setup for the `kubectl_proxyauth` CLI.
//!
//! Configures a `tracing` subscriber tuned for interactive CLI use (verbosity
//! levels, compact formatting) so command handlers can emit structured logs.

use time::format_description;
use tracing::{level_filters::LevelFilter, subscriber};
use tracing_subscriber::{
    EnvFilter, Layer, Registry,
    fmt::{self, time::UtcTime},
    layer::SubscriberExt,
};

pub mod level;

pub fn init_tracing(verbose_level: level::VerboseLevel, name: String) {
    // Pinned to version 1 (what the now-deprecated `parse()` used) to keep the
    // exact same parsing behavior rather than adopt v3's different syntax.
    let time_format = format_description::parse_borrowed::<1>("[hour]:[minute]:[second]")
        .expect("format string should be valid!");
    let timer = UtcTime::new(time_format);
    // A malformed RUST_LOG in the environment must not crash the CLI (it runs as
    // a kubectl exec-credential plugin); `from_env_lossy` drops bad directives
    // and keeps the default level instead of panicking.
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::from(verbose_level).into())
        .from_env_lossy();
    let terminal_out = fmt::layer()
        .with_writer(std::io::stderr)
        .with_thread_names(false)
        .with_timer(timer)
        .with_target(false)
        .with_filter(env_filter)
        .boxed();

    match subscriber::set_global_default(Registry::default().with(terminal_out)) {
        Ok(()) => tracing::trace!("Initialized tracing for {}", name),
        Err(e) => eprintln!("Failed to initialize tracing: {e}"),
    }
}
