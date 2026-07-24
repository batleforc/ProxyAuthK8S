//! Header handling shared by the standard and upgrade proxy paths.

use std::net::IpAddr;

use crate::model::user::User;

/// Headers managed by the transport itself; forwarding them corrupts the
/// upstream request when the body is re-framed.
const HOP_BY_HOP: [&str; 5] = [
    "connection",
    "upgrade",
    "transfer-encoding",
    "content-length",
    "host",
];

/// Headers the proxy is the sole authority for.
///
/// Every client header is otherwise forwarded verbatim, so a caller could
/// simply send `x-forwarded-user: admin` and impersonate anyone downstream.
/// These are always dropped from the incoming request and re-created here.
const PROXY_OWNED: [&str; 3] = ["x-forwarded-for", "x-forwarded-user", "x-forwarded-groups"];

pub fn is_hop_by_hop(name: &str) -> bool {
    HOP_BY_HOP
        .iter()
        .any(|header| name.eq_ignore_ascii_case(header))
}

pub fn is_proxy_owned_header(name: &str) -> bool {
    PROXY_OWNED
        .iter()
        .any(|header| name.eq_ignore_ascii_case(header))
}

/// Build the outgoing `x-forwarded-for` value.
///
/// The header is a chain: appending preserves the hops recorded by whatever
/// sits in front of us, overwriting would erase the original client address.
pub fn forwarded_for_value(existing: Option<&str>, peer_ip: Option<IpAddr>) -> Option<String> {
    let existing = existing.map(str::trim).filter(|value| !value.is_empty());
    match (existing, peer_ip) {
        (Some(existing), Some(peer)) => Some(format!("{}, {}", existing, peer)),
        (Some(existing), None) => Some(existing.to_string()),
        (None, Some(peer)) => Some(peer.to_string()),
        (None, None) => None,
    }
}

/// Strip anything that could terminate a header line.
///
/// The upgrade path serializes headers by hand, so a claim carrying a CR or LF
/// would let an OIDC provider (or a compromised one) inject arbitrary headers
/// into the upstream request.
fn sanitize_header_value(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, '\r' | '\n' | '\0'))
        .collect()
}

/// Identity headers describing the authenticated caller, if there is one.
pub fn identity_headers(user: Option<&User>) -> Vec<(&'static str, String)> {
    match user {
        Some(user) => vec![
            ("x-forwarded-user", sanitize_header_value(&user.username)),
            (
                "x-forwarded-groups",
                sanitize_header_value(&user.groups.join(",")),
            ),
        ],
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> User {
        User {
            username: "alice".to_string(),
            email: "alice@example.com".to_string(),
            groups: vec!["dev".to_string(), "platform".to_string()],
        }
    }

    #[test]
    fn hop_by_hop_detection_is_case_insensitive() {
        assert!(is_hop_by_hop("Connection"));
        assert!(is_hop_by_hop("TRANSFER-ENCODING"));
        assert!(!is_hop_by_hop("authorization"));
    }

    #[test]
    fn proxy_owned_headers_are_recognised() {
        assert!(is_proxy_owned_header("X-Forwarded-For"));
        assert!(is_proxy_owned_header("x-forwarded-user"));
        assert!(is_proxy_owned_header("X-Forwarded-Groups"));
        assert!(!is_proxy_owned_header("x-forwarded-proto"));
    }

    #[test]
    fn forwarded_for_appends_to_the_existing_chain() {
        assert_eq!(
            forwarded_for_value(Some("203.0.113.7"), Some("198.51.100.2".parse().unwrap())),
            Some("203.0.113.7, 198.51.100.2".to_string())
        );
        assert_eq!(
            forwarded_for_value(
                Some("203.0.113.7, 198.51.100.1"),
                Some("198.51.100.2".parse().unwrap())
            ),
            Some("203.0.113.7, 198.51.100.1, 198.51.100.2".to_string())
        );
    }

    #[test]
    fn forwarded_for_handles_missing_pieces() {
        assert_eq!(
            forwarded_for_value(None, Some("198.51.100.2".parse().unwrap())),
            Some("198.51.100.2".to_string())
        );
        assert_eq!(
            forwarded_for_value(Some("203.0.113.7"), None),
            Some("203.0.113.7".to_string())
        );
        assert_eq!(forwarded_for_value(None, None), None);
        assert_eq!(
            forwarded_for_value(Some("   "), Some("198.51.100.2".parse().unwrap())),
            Some("198.51.100.2".to_string())
        );
    }

    #[test]
    fn identity_headers_describe_the_caller() {
        assert_eq!(
            identity_headers(Some(&user())),
            vec![
                ("x-forwarded-user", "alice".to_string()),
                ("x-forwarded-groups", "dev,platform".to_string()),
            ]
        );
        assert!(identity_headers(None).is_empty());
    }

    #[test]
    fn identity_headers_cannot_inject_extra_headers() {
        let user = User {
            username: "alice\r\nx-admin: true".to_string(),
            email: String::new(),
            groups: vec!["dev\nx-admin: true".to_string()],
        };
        let headers = identity_headers(Some(&user));
        assert_eq!(headers[0].1, "alicex-admin: true");
        assert_eq!(headers[1].1, "devx-admin: true");
        assert!(headers.iter().all(|(_, v)| !v.contains(['\r', '\n'])));
    }
}
