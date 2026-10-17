//! Black-box integration suite against the k3d bench (RFC 0009).
//!
//! The action under test always goes through the Ingress and the proxy, as a
//! real client would; the admin kubeconfig is only for setting a test up and
//! for indirect checks (logs, metrics, object state), never for the action.
//! Run with `task it:up` then `task it:test`.

pub mod bench;
pub mod cli;
pub mod observe;
pub mod personas;

pub use bench::{Bench, Namespace};
pub use cli::{Output, Session};
pub use personas::Persona;
