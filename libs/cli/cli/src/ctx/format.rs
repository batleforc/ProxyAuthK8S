//! Output format selected by the global `--format` flag.

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// Rendering used by every command that prints a result.
#[derive(Serialize, Deserialize, Debug, Clone, Default, ValueEnum)]
pub enum ContextFormat {
    #[default]
    Table,
    Json,
    Yaml,
}
