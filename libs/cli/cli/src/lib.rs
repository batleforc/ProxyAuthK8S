//! Core logic for the `kubectl_proxyauth` CLI.
//!
//! Implements the command handlers (login, logout, config, context, get) and
//! the persisted configuration model that the thin `kubectl_proxyauth` binary
//! dispatches to.

pub mod cli_config;
pub mod command;
pub mod config_cmd;
pub mod context;
pub mod ctx;
pub mod error;
pub mod get;
pub mod helper;
pub mod login;
pub mod logout;
pub mod output;

// Re-exported at the crate root: `cli::Cli` is the binary's entry point and the
// path every caller already uses.
pub use command::{CacheCommands, Cli, Commands};
