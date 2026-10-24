//! Wire protocol of the agent tunnel (RFC 0012 §6.2).
//!
//! An agent dials a WebSocket to the proxy; [`ws_io::WsIo`] turns it into a
//! byte stream, which [`mux`] multiplexes with yamux. The agent opens the
//! first stream, the control channel ([`control`]); every stream the proxy
//! opens after that is a data stream the agent splices to its one upstream,
//! without reading it.

pub mod control;
pub mod mux;
pub mod ws_io;

/// Version a proxy replica announces in `Hello`; an agent that does not know
/// it closes the tunnel with `Close { reason: "protocol" }`.
pub const PROTOCOL_VERSION: u32 = 1;
