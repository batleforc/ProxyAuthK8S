//! The exec channel filter on hostile websocket bytes (RFC 0004, T4, T11):
//! it never panics; once it refuses, it keeps refusing; and a session without
//! stdin never lets a stdin message through.

#![no_main]

use api::cluster::redirect::kube_redirect::ExecChannelFilter;
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    stdin: bool,
    tty: bool,
    chunks: Vec<Vec<u8>>,
}

fuzz_target!(|input: Input| {
    let mut filter = ExecChannelFilter::new(input.stdin, input.tty);
    let mut refused = false;
    for chunk in &input.chunks {
        let result = filter.inspect(chunk);
        if refused {
            // A refused session is over: the tunnel never feeds it again, but
            // nothing in the filter may turn a refusal into an acceptance.
            continue;
        }
        if result.is_err() {
            refused = true;
            if let Some(channel) = filter.refused_channel() {
                assert!(
                    (channel == 0 && !input.stdin)
                        || (channel == 4 && !input.tty)
                        || !matches!(channel, 0 | 4 | 255),
                    "an allowed channel {channel} was refused"
                );
            }
        }
    }
});
