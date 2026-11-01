//! `UpstreamIdentity::from_user` on hostile claims (RFC 0014, T2, T4, T22):
//! it never panics, and an identity it accepts never starts with `system:`,
//! never carries an edge space, and goes in a header exactly as is.

#![no_main]

use std::collections::BTreeMap;

use api::cluster::redirect::upstream_identity::{Upstream, UpstreamIdentity};
use api::model::user::User;
use arbitrary::Arbitrary;
use crd::upstream_auth::{UpstreamAuth, UpstreamAuthMode};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    username: String,
    groups: Vec<String>,
    extra: Vec<(String, Vec<String>)>,
    user_prefix: String,
    group_prefix: String,
}

fn safe(value: &str) -> bool {
    !value.to_ascii_lowercase().starts_with("system:")
        && !value.starts_with(char::is_whitespace)
        && !value.ends_with(char::is_whitespace)
        && http::HeaderValue::from_str(value).is_ok()
}

fuzz_target!(|input: Input| {
    let user = User {
        username: input.username,
        groups: input.groups,
        extra: input.extra.into_iter().collect::<BTreeMap<_, _>>(),
        ..User::default()
    };
    let upstream = UpstreamAuth {
        mode: UpstreamAuthMode::FrontProxy,
        user_prefix: Some(input.user_prefix),
        group_prefix: Some(input.group_prefix),
        issuer_key: None,
        token_exchange: None,
    };
    let Ok(identity) = UpstreamIdentity::from_user(&user, &upstream) else {
        return;
    };
    assert!(safe(&identity.username), "{identity:?}");
    assert!(identity.groups.iter().all(|group| safe(group)), "{identity:?}");
    for (name, value) in Upstream::Headers(&identity).headers() {
        assert!(http::HeaderName::from_bytes(name.as_bytes()).is_ok(), "{name:?}");
        assert!(http::HeaderValue::from_str(&value).is_ok(), "{value:?}");
    }
});
