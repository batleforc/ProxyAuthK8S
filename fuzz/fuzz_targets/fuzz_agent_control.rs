//! The agent tunnel's control messages (RFC 0012 §6.2) on hostile bytes:
//! never a panic, nothing over the bound is decoded, and what decodes
//! encodes back to a frame that decodes to the same message.

#![no_main]

use agent_proto::control::{MAX_CONTROL_BYTES, decode, encode};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(kind) = decode(data) else { return };
    assert!(data.len() <= MAX_CONTROL_BYTES, "decoded past the bound");
    let frame = encode(kind.clone()).expect("a decoded message re-encodes");
    let (prefix, body) = frame.split_at(4);
    let len = u32::from_be_bytes(prefix.try_into().expect("a 4-byte prefix"));
    assert_eq!(usize::try_from(len).ok(), Some(body.len()), "length prefix");
    assert_eq!(decode(body).expect("its own frame decodes"), kind);
});
