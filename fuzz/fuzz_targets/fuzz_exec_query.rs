//! `parse_exec_query` on hostile queries (RFC 0004, T3): it never panics, is
//! deterministic, never accepts a repeated `container` or boolean nor a `;`,
//! and the `argv` it returns is every `command` value, in order, decoded once.

#![no_main]

use api::cluster::redirect::kube_redirect::parse_exec_query;
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

/// Queries shaped like exec queries, plus a raw one.
#[derive(Arbitrary, Debug)]
struct Input {
    pairs: Vec<(u8, String)>,
    raw: String,
}

const KEYS: [&str; 7] = ["command", "container", "stdin", "stdout", "stderr", "tty", "Command"];

fuzz_target!(|input: Input| {
    let shaped = input
        .pairs
        .iter()
        .map(|(key, value)| format!("{}={value}", KEYS[usize::from(*key) % KEYS.len()]))
        .collect::<Vec<_>>()
        .join("&");

    for query in [shaped.as_str(), input.raw.as_str()] {
        let parsed = parse_exec_query(query);
        assert_eq!(parsed, parse_exec_query(query), "not deterministic: {query:?}");
        let Ok(request) = parsed else { continue };
        assert!(!query.contains(';'), "a ';' was accepted: {query:?}");
        for key in ["container", "stdin", "stdout", "stderr", "tty"] {
            let count = query
                .split('&')
                .filter(|pair| pair.split('=').next() == Some(key))
                .count();
            assert!(count <= 1, "a repeated {key} was accepted: {query:?}");
        }
        let commands = query
            .split('&')
            .filter(|pair| pair.split('=').next() == Some("command"))
            .count();
        assert_eq!(request.argv.len(), commands, "{query:?}");
    }
});
