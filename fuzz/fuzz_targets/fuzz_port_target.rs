//! `port_target` on hostile paths: it never panics, is deterministic, and a
//! `ProxyPort(p)` it returns is the port the apiserver would read from the
//! `[scheme:]name:port` segment, decoded once.

#![no_main]

use api::cluster::redirect::kube_redirect::{PortTarget, port_target};
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

/// One round of `%XX` decoding, written independently of the proxy's own.
fn decode_once(value: &str) -> String {
    let bytes = value.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Paths shaped like the ones `port_target` cares about, so the mutator does
/// not spend its budget rediscovering `/api/v1/namespaces/`.
#[derive(Arbitrary, Debug)]
struct Input {
    kind: u8,
    namespace: String,
    name: String,
    tail: String,
    raw: String,
}

fuzz_target!(|input: Input| {
    let kind = ["pods", "services", "nodes", "configmaps"][usize::from(input.kind % 4)];
    let shaped = if kind == "nodes" {
        format!("/api/v1/nodes/{}/proxy/{}", input.name, input.tail)
    } else {
        format!(
            "/api/v1/namespaces/{}/{kind}/{}/proxy/{}",
            input.namespace, input.name, input.tail
        )
    };

    for path in [shaped.as_str(), input.raw.as_str()] {
        let target = port_target(path);
        assert_eq!(target, port_target(path), "not deterministic: {path:?}");

        if let PortTarget::ProxyPort(port) = target {
            let segments: Vec<String> = path
                .split('/')
                .filter(|s| !s.is_empty())
                .map(decode_once)
                .collect();
            assert!(port > 0, "{path:?}");
            assert!(
                segments.len() >= 7 && segments[6] == "proxy",
                "ProxyPort outside a proxy subresource: {path:?}"
            );
            let parts: Vec<&str> = segments[5].split(':').collect();
            assert!(matches!(parts.len(), 2 | 3), "{path:?}");
            let written = parts[parts.len() - 1];
            assert!(written.bytes().all(|b| b.is_ascii_digit()), "{path:?}");
            assert_eq!(written.parse::<u16>().ok(), Some(port), "{path:?}");
        }
    }
});
