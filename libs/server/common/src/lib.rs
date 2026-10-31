//! State, configuration and Redis/OIDC plumbing shared by the server crates.
//!
//! - [`config`]: every environment variable the server reads, parsed once.
//! - [`state`]: the shared [`State`] (construction) and its Redis access methods.
//! - [`kv`]: the store behind them, Redis or (under `test-util`) `kv_memory`.
//! - [`server_config`]: the HTTP(S) listener and its rustls configuration.
//! - [`error`]: boot-time error types.

pub mod agent_gateway;
pub mod config;
pub mod error;
pub mod kv;
#[cfg(feature = "test-util")]
pub mod kv_memory;
pub mod oidc_cache;
pub mod oidc_conf;
pub mod oidc_config_cache;
pub mod oidc_error;
pub mod redis_pool;
pub mod server_config;
pub mod state;
#[cfg(feature = "test-util")]
pub mod test_skip;
pub mod token_audience;
pub mod token_exchange;
pub mod traits;
pub mod upstream_cache;
pub mod upstream_trust;

pub use error::{StateInitError, TlsConfigError};
pub use server_config::ServerConfig;
pub use state::State;
