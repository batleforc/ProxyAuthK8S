//! The agent tunnel's control messages (RFC 0012 §6.2) on hostile bytes:
//! never a panic, nothing over the bound is decoded, and what decodes
//! encodes back to a frame that decodes to the same message; what an agent
//! announces is checked without a panic either (RFC 0013 §7.3).

#![no_main]

use agent_proto::control::{Kind, MAX_CONTROL_BYTES, decode, encode};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(kind) = decode(data) else { return };
    if let Kind::AgentInfo(info) = &kind {
        if let Some(version) = agent_proto::valid_version(&info.version) {
            assert!(version.len() <= agent_proto::MAX_VERSION_BYTES);
            assert!(!version.contains(['\n', '\r', '\0']), "{version:?}");
        }
        let _ = agent_proto::valid_fingerprint(&info.ca_fingerprint);
        let _ = agent_proto::negotiate((info.protocol_min, info.protocol_max), (1, 1));
    }
    assert!(data.len() <= MAX_CONTROL_BYTES, "decoded past the bound");
    let frame = encode(kind.clone()).expect("a decoded message re-encodes");
    let (prefix, body) = frame.split_at(4);
    let len = u32::from_be_bytes(prefix.try_into().expect("a 4-byte prefix"));
    assert_eq!(usize::try_from(len).ok(), Some(body.len()), "length prefix");
    assert_eq!(decode(body).expect("its own frame decodes"), kind);
});
