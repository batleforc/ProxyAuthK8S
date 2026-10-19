//! `request_info` on hostile requests (RFC 0006 §6.3): deterministic, never a
//! panic, a method other than GET/HEAD is never a read, and a decoded
//! `exec`/`attach`/`portforward` subresource is always `create`.

#![no_main]

use arbitrary::Arbitrary;
use crd::security::{Verb, request_info};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    method: u8,
    namespace: String,
    resource: String,
    name: String,
    subresource: String,
    query: String,
    raw_path: String,
    raw_method: String,
}

fuzz_target!(|input: Input| {
    const METHODS: [&str; 8] = [
        "GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "get",
    ];
    let method = METHODS[usize::from(input.method) % METHODS.len()];
    let shaped = format!(
        "/api/v1/namespaces/{}/{}/{}/{}",
        input.namespace, input.resource, input.name, input.subresource
    );

    for (method, path) in [
        (method, shaped.as_str()),
        (input.raw_method.as_str(), input.raw_path.as_str()),
        (method, input.raw_path.as_str()),
    ] {
        let info = request_info(method, path, &input.query);
        assert_eq!(
            info,
            request_info(method, path, &input.query),
            "not deterministic"
        );
        let Ok(info) = info else { continue };

        if method != "GET" && method != "HEAD" {
            assert!(
                !Verb::READ.contains(&info.verb),
                "{method} {path:?} classified as a read"
            );
        }
        if matches!(info.subresource.as_str(), "exec" | "attach" | "portforward") {
            assert_eq!(info.verb, Verb::Create, "{method} {path:?}");
        }
    }
});
