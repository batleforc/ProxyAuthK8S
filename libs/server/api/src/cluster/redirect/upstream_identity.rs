//! The identity the proxy asserts to the apiserver in `FrontProxy` and
//! `Issuer` (RFC 0014 §4.2), and what each request presents upstream.
//!
//! Pure and without I/O: [`UpstreamIdentity::from_user`] is the fuzz target
//! of `fuzz_upstream_identity`.

use std::collections::BTreeMap;

use crd::upstream_auth::{UpstreamAuth, extra_key_refusal};
use http::HeaderValue;
use secrecy::{ExposeSecret as _, SecretString};

use crate::model::user::User;

/// At most this many groups are forwarded (RFC 0014 §4.2): header sizes are
/// capped by the apiserver and the Ingress in between.
pub const MAX_GROUPS: usize = 64;
/// At most this many bytes of name, groups and `extra` together.
pub const MAX_IDENTITY_BYTES: usize = 8 * 1024;

/// The identity the apiserver sees: prefixed name and groups, and the mapped
/// `extra`. Only built by [`Self::from_user`], so every value is safe to put
/// in a header as is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpstreamIdentity {
    pub username: String,
    pub groups: Vec<String>,
    pub extra: BTreeMap<String, Vec<String>>,
}

/// Why an identity cannot be forwarded: refused in `403` before any call
/// upstream, never cleaned (cleaning would change the identity in silence).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityRefusal {
    /// A name or group starting with `system:`.
    SystemPrefix,
    /// No username after the mapping.
    EmptyUsername,
    /// A control character, a byte a header cannot carry, an edge space, an
    /// empty group or `extra` value, or an `extra` key the apiserver would
    /// reinterpret or reserves.
    Untransmittable,
    /// More than [`MAX_GROUPS`] groups or [`MAX_IDENTITY_BYTES`] bytes.
    TooLarge,
}

impl IdentityRefusal {
    /// The `reason` of the audit line and the metric label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SystemPrefix => "system_prefix",
            Self::EmptyUsername => "empty_username",
            Self::Untransmittable => "untransmittable",
            Self::TooLarge => "too_large",
        }
    }

    /// The `Status` message, naming who can act and never the offending value.
    #[must_use]
    pub fn message(self, ns: &str, cluster: &str) -> String {
        let owner = format!("ask the owner of ProxyKubeApi {ns}/{cluster}");
        match self {
            Self::SystemPrefix => format!(
                "the identity mapped for this cluster has a reserved system: name or group; {owner}"
            ),
            Self::EmptyUsername => format!(
                "the identity mapped for this cluster has no username for the configured claim; {owner} to check claim_mappings"
            ),
            Self::Untransmittable => {
                format!(
                    "the identity mapped for this cluster has a value it cannot forward; {owner}"
                )
            }
            Self::TooLarge => format!(
                "the identity mapped for this cluster is too large to forward (more than {MAX_GROUPS} groups or 8 KiB); {owner} to narrow the groups mapping"
            ),
        }
    }
}

/// Whether `value` can go in a header exactly as is: a valid header value
/// (no control byte; UTF-8 is carried as is), no edge whitespace (the apiserver trims it, which would
/// make `p:bob ` the `p:bob` of someone else).
fn transmittable(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(char::is_whitespace)
        && !value.ends_with(char::is_whitespace)
        && !value.chars().any(char::is_control)
        && HeaderValue::from_str(value).is_ok()
}

fn reserved(value: &str) -> bool {
    value
        .get(..7)
        .is_some_and(|start| start.eq_ignore_ascii_case("system:"))
}

impl UpstreamIdentity {
    /// The prefixed identity of `user` under `upstream`, or why it cannot be
    /// forwarded.
    ///
    /// # Errors
    ///
    /// An [`IdentityRefusal`], the first rule broken, in the order of §4.2.
    pub fn from_user(user: &User, upstream: &UpstreamAuth) -> Result<Self, IdentityRefusal> {
        if user.username.is_empty() {
            return Err(IdentityRefusal::EmptyUsername);
        }
        let user_prefix = upstream.user_prefix.as_deref().unwrap_or_default();
        let group_prefix = upstream.group_prefix.as_deref().unwrap_or_default();
        let username = format!("{user_prefix}{}", user.username);
        let groups: Vec<String> = user
            .groups
            .iter()
            .map(|group| format!("{group_prefix}{group}"))
            .collect();

        if reserved(&username) || groups.iter().any(|group| reserved(group)) {
            return Err(IdentityRefusal::SystemPrefix);
        }
        if groups.len() > MAX_GROUPS {
            return Err(IdentityRefusal::TooLarge);
        }
        // An empty group stays refused once prefixed: checked on the raw value.
        if user.groups.iter().any(String::is_empty)
            || !transmittable(&username)
            || !groups.iter().all(|group| transmittable(group))
        {
            return Err(IdentityRefusal::Untransmittable);
        }
        for (key, values) in &user.extra {
            if extra_key_refusal(key).is_some() || !values.iter().all(|value| transmittable(value))
            {
                return Err(IdentityRefusal::Untransmittable);
            }
        }
        let size = std::iter::once(username.len())
            .chain(groups.iter().map(String::len))
            .chain(user.extra.iter().flat_map(|(key, values)| {
                std::iter::once(key.len()).chain(values.iter().map(String::len))
            }))
            .fold(0_usize, usize::saturating_add);
        if size > MAX_IDENTITY_BYTES {
            return Err(IdentityRefusal::TooLarge);
        }
        Ok(Self {
            username,
            groups,
            extra: user.extra.clone(),
        })
    }
}

/// The identity a trusted mode asserts upstream, and in `Issuer` the JWT
/// the proxy signed for it (RFC 0014 §5.2).
#[derive(Debug)]
pub struct Asserted {
    pub identity: UpstreamIdentity,
    pub token: Option<SecretString>,
}

/// What a request presents to the apiserver (RFC 0014 §6.3).
#[derive(Clone, Copy, Debug)]
pub enum Upstream<'a> {
    /// The caller's `Authorization` is relayed; `x-forwarded-*` describe the
    /// caller, when there is one.
    Passthrough(Option<&'a User>),
    /// `FrontProxy`: every `Authorization` is dropped, `X-Remote-*` assert
    /// the identity, on the connection that presents the proxy's certificate.
    Headers(&'a UpstreamIdentity),
    /// `Issuer`: every `Authorization` is replaced by the proxy's JWT for
    /// the identity; no `X-Remote-*`, no `x-forwarded-*`.
    Bearer(&'a UpstreamIdentity, &'a SecretString),
}

impl<'a> Upstream<'a> {
    #[must_use]
    pub fn new(user: Option<&'a User>, asserted: Option<&'a Asserted>) -> Self {
        match asserted {
            None => Self::Passthrough(user),
            Some(Asserted {
                identity,
                token: None,
            }) => Self::Headers(identity),
            Some(Asserted {
                identity,
                token: Some(token),
            }) => Self::Bearer(identity, token),
        }
    }

    /// Whether the client's `Authorization` must not reach the apiserver.
    #[must_use]
    pub fn drops_authorization(self) -> bool {
        !matches!(self, Self::Passthrough(_))
    }

    /// The headers the proxy sets upstream, each one appended (a repeated
    /// `X-Remote-Group` is one group per value). In `Headers`, the values
    /// were checked by [`UpstreamIdentity::from_user`].
    #[must_use]
    pub fn headers(self) -> Vec<(String, String)> {
        match self {
            Self::Passthrough(None) => Vec::new(),
            Self::Passthrough(Some(user)) => vec![
                ("x-forwarded-user".to_string(), sanitize(&user.username)),
                (
                    "x-forwarded-groups".to_string(),
                    sanitize(&user.groups.join(",")),
                ),
            ],
            Self::Headers(identity) => {
                let mut headers = vec![("X-Remote-User".to_string(), identity.username.clone())];
                headers.extend(
                    identity
                        .groups
                        .iter()
                        .map(|group| ("X-Remote-Group".to_string(), group.clone())),
                );
                for (key, values) in &identity.extra {
                    let name = format!("X-Remote-Extra-{}", percent_encode_key(key));
                    headers.extend(values.iter().map(|value| (name.clone(), value.clone())));
                }
                headers
            }
            Self::Bearer(_, token) => vec![(
                "authorization".to_string(),
                format!("Bearer {}", token.expose_secret()),
            )],
        }
    }
}

/// Strip anything that could terminate a header line, for the `x-forwarded-*`
/// of `Passthrough`, as before RFC 0014.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, '\r' | '\n' | '\0'))
        .collect()
}

/// An `extra` key as the apiserver reads it after `X-Remote-Extra-`: every
/// byte outside the unreserved set percent-encoded, `%` and `/` included.
fn percent_encode_key(key: &str) -> String {
    let mut encoded = String::with_capacity(key.len());
    for byte in key.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str, groups: &[&str]) -> User {
        User {
            username: name.to_string(),
            groups: groups.iter().map(ToString::to_string).collect(),
            ..User::default()
        }
    }

    fn upstream() -> UpstreamAuth {
        UpstreamAuth {
            mode: crd::upstream_auth::UpstreamAuthMode::FrontProxy,
            user_prefix: Some("proxyauth:".to_string()),
            group_prefix: Some("proxyauth:".to_string()),
            issuer_key: None,
        }
    }

    #[test]
    fn the_identity_is_prefixed() {
        let identity = UpstreamIdentity::from_user(&user("alice", &["dev"]), &upstream()).unwrap();
        assert_eq!(identity.username, "proxyauth:alice");
        assert_eq!(identity.groups, ["proxyauth:dev"]);
    }

    /// T2: a `system:masters` from the IdP is harmless once prefixed.
    #[test]
    fn a_system_group_from_the_idp_is_prefixed_or_refused() {
        let identity =
            UpstreamIdentity::from_user(&user("alice", &["system:masters"]), &upstream()).unwrap();
        assert_eq!(identity.groups, ["proxyauth:system:masters"]);
        let mut unprefixed = upstream();
        unprefixed.group_prefix = Some(String::new());
        assert_eq!(
            UpstreamIdentity::from_user(&user("alice", &["System:Masters"]), &unprefixed),
            Err(IdentityRefusal::SystemPrefix)
        );
    }

    /// T4, T22: refused, never cleaned.
    #[test]
    fn a_value_a_header_cannot_carry_is_refused_not_cleaned() {
        for (name, groups) in [
            ("alice\r\nx-admin: true", vec!["dev"]),
            ("bob ", vec!["dev"]),
            ("alice", vec![""]),
            ("alice", vec!["dev\n"]),
        ] {
            assert_eq!(
                UpstreamIdentity::from_user(&user(name, &groups), &upstream()),
                Err(IdentityRefusal::Untransmittable),
                "{name:?} {groups:?}"
            );
        }
        // A non-ASCII name is forwarded intact: header values carry its
        // UTF-8 bytes and the apiserver reads them back as such.
        assert!(UpstreamIdentity::from_user(&user("élodie", &["dev"]), &upstream()).is_ok());
        let mut with_extra = user("alice", &[]);
        with_extra
            .extra
            .insert("Example.com/team".into(), vec!["blue".into()]);
        assert_eq!(
            UpstreamIdentity::from_user(&with_extra, &upstream()),
            Err(IdentityRefusal::Untransmittable)
        );
    }

    #[test]
    fn an_empty_name_or_too_large_an_identity_is_refused() {
        assert_eq!(
            UpstreamIdentity::from_user(&user("", &[]), &upstream()),
            Err(IdentityRefusal::EmptyUsername)
        );
        let many: Vec<String> = (0..=MAX_GROUPS).map(|i| format!("g{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(
            UpstreamIdentity::from_user(&user("alice", &many), &upstream()),
            Err(IdentityRefusal::TooLarge)
        );
        let long = "g".repeat(200);
        let long: Vec<&str> = std::iter::repeat_n(long.as_str(), 50).collect();
        assert_eq!(
            UpstreamIdentity::from_user(&user("alice", &long), &upstream()),
            Err(IdentityRefusal::TooLarge)
        );
    }

    /// The limits themselves are accepted, one past them refused (RFC 0015
    /// §6.2): 64 groups, and 8 KiB of name and groups together.
    #[test]
    fn an_identity_at_the_limits_is_accepted_one_past_them_refused() {
        let groups: Vec<String> = (0..MAX_GROUPS).map(|i| format!("g{i}")).collect();
        let groups: Vec<&str> = groups.iter().map(String::as_str).collect();
        assert!(UpstreamIdentity::from_user(&user("alice", &groups), &upstream()).is_ok());

        // "proxyauth:" + name: the name fills the rest of the 8 KiB.
        let fill = MAX_IDENTITY_BYTES - "proxyauth:".len();
        assert_eq!(MAX_IDENTITY_BYTES, 8_192);
        assert!(UpstreamIdentity::from_user(&user(&"a".repeat(fill), &[]), &upstream()).is_ok());
        assert_eq!(
            UpstreamIdentity::from_user(&user(&"a".repeat(fill + 1), &[]), &upstream()),
            Err(IdentityRefusal::TooLarge)
        );
    }

    /// The `reason` of the audit line and the metric label are a contract
    /// with the dashboards; the message names the cluster to act on.
    #[test]
    fn each_refusal_has_its_label_and_names_its_cluster() {
        let all = [
            (IdentityRefusal::SystemPrefix, "system_prefix"),
            (IdentityRefusal::EmptyUsername, "empty_username"),
            (IdentityRefusal::Untransmittable, "untransmittable"),
            (IdentityRefusal::TooLarge, "too_large"),
        ];
        for (refusal, label) in all {
            assert_eq!(refusal.as_str(), label);
            assert!(
                refusal
                    .message("team-a", "prod")
                    .contains("ProxyKubeApi team-a/prod"),
                "{label}"
            );
        }
    }

    /// Decision 23: one header per group, a comma kept inside its group.
    #[test]
    fn front_proxy_sends_one_group_header_per_group_and_encoded_extra_keys() {
        let mut caller = user("alice", &["a,b", "c"]);
        caller
            .extra
            .insert("example.com/team".into(), vec!["blue".into(), "red".into()]);
        let identity = UpstreamIdentity::from_user(&caller, &upstream()).unwrap();
        let headers = Upstream::Headers(&identity).headers();
        assert_eq!(
            headers,
            [
                ("X-Remote-User".to_string(), "proxyauth:alice".to_string()),
                ("X-Remote-Group".to_string(), "proxyauth:a,b".to_string()),
                ("X-Remote-Group".to_string(), "proxyauth:c".to_string()),
                (
                    "X-Remote-Extra-example.com%2Fteam".to_string(),
                    "blue".to_string()
                ),
                (
                    "X-Remote-Extra-example.com%2Fteam".to_string(),
                    "red".to_string()
                ),
            ]
        );
        assert!(Upstream::Headers(&identity).drops_authorization());
        assert!(!Upstream::Passthrough(Some(&caller)).drops_authorization());
    }

    #[test]
    fn passthrough_keeps_the_forwarded_headers_of_before() {
        let caller = user("alice\r\n", &["dev", "ops"]);
        assert_eq!(
            Upstream::Passthrough(Some(&caller)).headers(),
            [
                ("x-forwarded-user".to_string(), "alice".to_string()),
                ("x-forwarded-groups".to_string(), "dev,ops".to_string()),
            ]
        );
        assert!(Upstream::Passthrough(None).headers().is_empty());
    }

    /// Invariants over generated users (RFC 0015 §6.4): what the fixed cases
    /// above do not reach.
    mod properties {
        use super::*;
        use proptest::prelude::*;

        /// Mostly printable text, with the edge whitespace, control bytes,
        /// `system:` and non-ASCII the rules are about.
        fn text() -> impl Strategy<Value = String> {
            prop_oneof![
                "[a-zA-Z0-9:@._-]{0,12}",
                "\\PC{0,12}",
                "[ \t\r\n\0a-z]{0,6}",
                "(?i:system:)[a-z]{0,6}",
            ]
        }

        fn arbitrary_user() -> impl Strategy<Value = User> {
            (
                text(),
                prop::collection::vec(text(), 0..70),
                prop::collection::btree_map(text(), prop::collection::vec(text(), 0..3), 0..3),
            )
                .prop_map(|(username, groups, extra)| User {
                    username,
                    groups,
                    extra,
                    ..User::default()
                })
        }

        fn percent_decode(encoded: &str) -> String {
            let mut parts = encoded.split('%');
            let mut out = parts.next().unwrap_or_default().as_bytes().to_vec();
            for part in parts {
                let (hex, rest) = part.split_at(2);
                out.push(u8::from_str_radix(hex, 16).unwrap());
                out.extend_from_slice(rest.as_bytes());
            }
            String::from_utf8(out).unwrap()
        }

        proptest! {
            /// Accepted, an identity reads back whole from the `X-Remote-*`
            /// headers, every header is valid, and nothing says `system:`.
            #[test]
            fn an_accepted_identity_reads_back_whole_from_its_headers(user in arbitrary_user()) {
                let Ok(identity) = UpstreamIdentity::from_user(&user, &upstream()) else {
                    return Ok(());
                };
                let headers = Upstream::Headers(&identity).headers();
                for (name, value) in &headers {
                    prop_assert!(axum::http::HeaderName::from_bytes(name.as_bytes()).is_ok());
                    prop_assert!(HeaderValue::from_str(value).is_ok());
                }
                let values = |wanted: &str| -> Vec<String> {
                    headers
                        .iter()
                        .filter(|(name, _)| name == wanted)
                        .map(|(_, value)| value.clone())
                        .collect()
                };
                prop_assert_eq!(values("X-Remote-User"), vec![identity.username.clone()]);
                prop_assert_eq!(values("X-Remote-Group"), identity.groups.clone());
                for (key, expected) in &identity.extra {
                    let name = format!("X-Remote-Extra-{}", percent_encode_key(key));
                    prop_assert_eq!(percent_decode(&percent_encode_key(key)), key.clone());
                    prop_assert_eq!(&values(&name), expected);
                }
                prop_assert!(!reserved(&identity.username));
                prop_assert!(identity.groups.iter().all(|group| !reserved(group)));
            }

            /// `Passthrough` sanitizes instead of refusing: its forwarded
            /// headers never carry a line break or a NUL.
            #[test]
            fn passthrough_never_forwards_a_line_break(user in arbitrary_user()) {
                for (_, value) in Upstream::Passthrough(Some(&user)).headers() {
                    prop_assert!(!value.contains(['\r', '\n', '\0']));
                }
            }
        }
    }
}
