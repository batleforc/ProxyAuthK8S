//! The agent gateway of RFC 0012 (decision 26): the tunnels agents hold, and
//! the `CONNECT` the proxy reaches them by.

pub mod ca;
pub mod connect;
pub mod enroll;
pub mod listener;
pub mod presence;
pub mod registry;
