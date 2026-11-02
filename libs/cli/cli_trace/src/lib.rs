//! Logging and tracing setup for the `kubectl_proxyauth` CLI.
//!
//! Configures a `tracing` subscriber tuned for interactive CLI use (verbosity
//! levels, compact formatting) so command handlers can emit structured logs.

use std::io::IsTerminal;

use time::format_description;
use tracing::{level_filters::LevelFilter, subscriber};
use tracing_subscriber::{
    EnvFilter, Layer, Registry,
    fmt::{self, time::UtcTime},
    layer::SubscriberExt,
};

pub mod level;

#[expect(
    clippy::expect_used,
    reason = "the time format is a static literal; a parse failure is a programming error caught by any run"
)]
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
    // Colours only on a terminal, and never with NO_COLOR set (no-color.org):
    // a redirected stderr must not fill a log file with escape codes.
    let ansi = std::io::stderr().is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(ansi)
        .with_thread_names(false)
        .with_target(false);
    // A timestamp helps a `-v` trace; at the default level it is noise in
    // front of messages meant for a person.
    let terminal_out = if verbose_level < level::VerboseLevel::INFO {
        layer.with_timer(timer).with_filter(env_filter).boxed()
    } else {
        layer.without_time().with_filter(env_filter).boxed()
    };

    match subscriber::set_global_default(Registry::default().with(terminal_out)) {
        Ok(()) => tracing::trace!("Initialized tracing for {}", name),
        Err(e) => eprintln!("Failed to initialize tracing: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracing_initialises_once_and_a_second_call_does_not_panic() {
        // Both layouts are built (timestamps for -v, none at the default
        // level); only the first call installs the global subscriber, the
        // second reports it on stderr instead of panicking, as a CLI run
        // twice in one process (tests, an embedding) would.
        init_tracing(level::VerboseLevel::DEBUG, "test".to_string());
        init_tracing(level::VerboseLevel::INFO, "test".to_string());
        tracing::info!("still logging after a second init");
    }
}
