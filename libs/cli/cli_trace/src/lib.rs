use time::format_description;
use tracing::{level_filters::LevelFilter, subscriber};
use tracing_subscriber::{
    fmt::{self, time::UtcTime},
    layer::SubscriberExt,
    EnvFilter, Layer, Registry,
};

pub mod level;

pub fn init_tracing(verbose_level: level::VerboseLevel, name: String) {
    let time_format = format_description::parse("[hour]:[minute]:[second]")
        .expect("format string should be valid!");
    let timer = UtcTime::new(time_format);
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::from(verbose_level).into())
        .from_env()
        .unwrap();
    let terminal_out = fmt::layer()
        .with_thread_names(false)
        .with_timer(timer)
        .with_target(false)
        .with_filter(env_filter)
        .boxed();

    match subscriber::set_global_default(Registry::default().with(terminal_out)) {
        Ok(_) => tracing::trace!("Initialized tracing for {}", name),
        Err(e) => eprintln!("Failed to initialize tracing: {}", e),
    }
}
