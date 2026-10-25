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

/// The highest protocol this code speaks. Agent and gateway each announce
/// `PROTOCOL_MIN..=PROTOCOL_VERSION` and keep the highest both speak
/// ([`negotiate`]); without one, the tunnel closes with
/// `Close { reason: "protocol" }` (RFC 0013 §4.2).
pub const PROTOCOL_VERSION: u32 = 1;
/// The lowest protocol this code still accepts: an old one stays at least a
/// minor release after the next is out (RFC 0013, decision 12).
pub const PROTOCOL_MIN: u32 = 1;

/// The longest version an agent may announce (RFC 0013 §7.3).
pub const MAX_VERSION_BYTES: usize = 64;

/// The protocol both ranges speak, the highest one; `None` without one. A
/// range announced as `(0, 0)` is an agent from before RFC 0013, which spoke
/// protocol 1 only (decision 2).
#[must_use]
pub fn negotiate(agent: (u32, u32), gateway: (u32, u32)) -> Option<u32> {
    let agent = if agent == (0, 0) { (1, 1) } else { agent };
    let (low, high) = (agent.0.max(gateway.0), agent.1.min(gateway.1));
    (agent.0 <= agent.1 && low <= high).then_some(high)
}

/// `version` if it is a SemVer of at most [`MAX_VERSION_BYTES`]: what an
/// agent announces reaches Redis and the `ProxyKubeApi` status (RFC 0013, T1).
#[must_use]
pub fn valid_version(version: &str) -> Option<&str> {
    (version.len() <= MAX_VERSION_BYTES && semver::Version::parse(version).is_ok())
        .then_some(version)
}

/// Whether `fingerprint` is a SHA-256 in lowercase hex, as the agent computes it.
#[must_use]
pub fn valid_fingerprint(fingerprint: &str) -> bool {
    fingerprint.len() == 64
        && fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every pair of ranges over `0..=4`, against the definition: the
    /// highest protocol inside both.
    #[test]
    fn negotiation_keeps_the_highest_protocol_of_both_ranges() {
        for agent in (0..=4).flat_map(|min| (0..=4).map(move |max| (min, max))) {
            for gateway in (1..=4).flat_map(|min| (min..=4).map(move |max| (min, max))) {
                let effective = if agent == (0, 0) { (1, 1) } else { agent };
                let expected = (1..=4).rev().find(|protocol| {
                    (effective.0..=effective.1).contains(protocol)
                        && (gateway.0..=gateway.1).contains(protocol)
                });
                assert_eq!(negotiate(agent, gateway), expected, "{agent:?} {gateway:?}");
            }
        }
        assert_eq!(negotiate((0, 0), (PROTOCOL_MIN, PROTOCOL_VERSION)), Some(1));
        assert_eq!(negotiate((2, 3), (1, 1)), None);
        assert_eq!(negotiate((3, 2), (1, 4)), None);
    }

    #[test]
    fn only_a_short_semver_is_a_version() {
        for good in ["0.4.0", "0.5.0-rc.1", "1.2.3+build.7"] {
            assert_eq!(valid_version(good), Some(good));
        }
        let long = format!("0.4.0-{}", "a".repeat(MAX_VERSION_BYTES));
        for bad in [
            "",
            "test",
            "0.4",
            "0.4.0\n{\"forged\":1}",
            "0.4.0\0",
            long.as_str(),
        ] {
            assert_eq!(valid_version(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_fingerprint_is_64_lowercase_hex() {
        assert!(valid_fingerprint(&"ab".repeat(32)));
        assert!(!valid_fingerprint(&"AB".repeat(32)));
        assert!(!valid_fingerprint(&"ab".repeat(31)));
        assert!(!valid_fingerprint(&format!("{}\n", "a".repeat(63))));
    }
}
