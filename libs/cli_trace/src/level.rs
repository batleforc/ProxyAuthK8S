use serde_repr::{Deserialize_repr, Serialize_repr};
use std::fmt::{Debug, Display};
use tracing::Level;

#[derive(Serialize_repr, Deserialize_repr, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum VerboseLevel {
    ERROR = 4,
    WARN = 3,
    INFO = 2,
    DEBUG = 1,
    TRACE = 0,
}

impl From<VerboseLevel> for Level {
    fn from(level: VerboseLevel) -> Self {
        match level {
            VerboseLevel::ERROR => Level::ERROR,
            VerboseLevel::WARN => Level::WARN,
            VerboseLevel::INFO => Level::INFO,
            VerboseLevel::DEBUG => Level::DEBUG,
            VerboseLevel::TRACE => Level::TRACE,
        }
    }
}

impl From<VerboseLevel> for tracing_subscriber::filter::LevelFilter {
    fn from(level: VerboseLevel) -> Self {
        match level {
            VerboseLevel::ERROR => tracing_subscriber::filter::LevelFilter::ERROR,
            VerboseLevel::WARN => tracing_subscriber::filter::LevelFilter::WARN,
            VerboseLevel::INFO => tracing_subscriber::filter::LevelFilter::INFO,
            VerboseLevel::DEBUG => tracing_subscriber::filter::LevelFilter::DEBUG,
            VerboseLevel::TRACE => tracing_subscriber::filter::LevelFilter::TRACE,
        }
    }
}

impl Debug for VerboseLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ERROR => write!(f, "ERROR"),
            Self::WARN => write!(f, "WARN"),
            Self::INFO => write!(f, "INFO"),
            Self::DEBUG => write!(f, "DEBUG"),
            Self::TRACE => write!(f, "TRACE"),
        }
    }
}

impl Display for VerboseLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ERROR => write!(f, "ERROR"),
            Self::WARN => write!(f, "WARN"),
            Self::INFO => write!(f, "INFO"),
            Self::DEBUG => write!(f, "DEBUG"),
            Self::TRACE => write!(f, "TRACE"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verbose_level() {
        let level = VerboseLevel::ERROR;
        let serialized = serde_json::to_string(&level).unwrap();
        let deserialized: VerboseLevel = serde_json::from_str(&serialized).unwrap();

        assert_eq!(level, deserialized);
    }

    #[test]
    fn test_verbose_level_conversion() {
        let level = VerboseLevel::ERROR;
        let tracing_level: Level = level.into();
        let level_filter: tracing_subscriber::filter::LevelFilter = level.into();

        assert_eq!(tracing_level, Level::ERROR);
        assert_eq!(level_filter, tracing_subscriber::filter::LevelFilter::ERROR);

        let level = VerboseLevel::WARN;
        let tracing_level: Level = level.into();
        let level_filter: tracing_subscriber::filter::LevelFilter = level.into();

        assert_eq!(tracing_level, Level::WARN);
        assert_eq!(level_filter, tracing_subscriber::filter::LevelFilter::WARN);

        let level = VerboseLevel::INFO;
        let tracing_level: Level = level.into();
        let level_filter: tracing_subscriber::filter::LevelFilter = level.into();

        assert_eq!(tracing_level, Level::INFO);
        assert_eq!(level_filter, tracing_subscriber::filter::LevelFilter::INFO);

        let level = VerboseLevel::DEBUG;
        let tracing_level: Level = level.into();
        let level_filter: tracing_subscriber::filter::LevelFilter = level.into();

        assert_eq!(tracing_level, Level::DEBUG);
        assert_eq!(level_filter, tracing_subscriber::filter::LevelFilter::DEBUG);

        let level = VerboseLevel::TRACE;
        let tracing_level: Level = level.into();
        let level_filter: tracing_subscriber::filter::LevelFilter = level.into();

        assert_eq!(tracing_level, Level::TRACE);
        assert_eq!(level_filter, tracing_subscriber::filter::LevelFilter::TRACE);
    }

    #[test]
    fn test_verbose_level_debug() {
        let level = VerboseLevel::ERROR;
        assert_eq!(format!("{:?}", level), "ERROR");

        let level = VerboseLevel::WARN;
        assert_eq!(format!("{:?}", level), "WARN");

        let level = VerboseLevel::INFO;
        assert_eq!(format!("{:?}", level), "INFO");

        let level = VerboseLevel::DEBUG;
        assert_eq!(format!("{:?}", level), "DEBUG");

        let level = VerboseLevel::TRACE;
        assert_eq!(format!("{:?}", level), "TRACE");
    }

    #[test]
    fn test_verbose_level_display() {
        let level = VerboseLevel::ERROR;
        assert_eq!(format!("{}", level), "ERROR");

        let level = VerboseLevel::WARN;
        assert_eq!(format!("{}", level), "WARN");

        let level = VerboseLevel::INFO;
        assert_eq!(format!("{}", level), "INFO");

        let level = VerboseLevel::DEBUG;
        assert_eq!(format!("{}", level), "DEBUG");

        let level = VerboseLevel::TRACE;
        assert_eq!(format!("{}", level), "TRACE");
    }
}
