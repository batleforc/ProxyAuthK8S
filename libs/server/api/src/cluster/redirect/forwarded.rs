//! Header handling shared by the standard and upgrade proxy paths.

use std::net::IpAddr;
use std::sync::LazyLock;

use crate::model::user::User;

/// Number of trusted reverse proxies sitting in front of this service.
///
/// Throttle identity (bans, rate limits) is taken this many hops back in the
/// `X-Forwarded-For` chain. The default of `0` trusts nothing and uses the
/// direct socket peer, which preserves the previous behaviour for deployments
/// with no known proxy in front. Set it to the number of trusted hops (e.g. `1`
/// behind a single ingress/LB) so bans target the real client instead of the
/// shared ingress address.
static TRUSTED_PROXY_COUNT: LazyLock<usize> = LazyLock::new(|| {
    std::env::var("TRUSTED_PROXY_COUNT")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0)
});

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

/// Header-name prefixes that assert an identity to the upstream Kubernetes
/// apiserver: native impersonation (`Impersonate-User/Group/Uid/Extra-*`) and the
/// conventional `requestheader` authentication names (`X-Remote-User/Group/Uid/
/// Extra-*`). The proxy conveys the caller's identity itself (via the stamped
/// `x-forwarded-*` headers / its front-proxy client cert), so a client must never
/// be allowed to smuggle these through: otherwise `X-Remote-User: system:admin`
/// or `Impersonate-User: …` would be authenticated as another principal upstream.
/// Matched as prefixes so the open-ended `*-extra-<key>` families are covered.
const UPSTREAM_AUTH_PREFIXES: [&str; 2] = ["impersonate-", "x-remote-"];

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

/// Whether `name` is an upstream-authentication/impersonation header a client is
/// never allowed to set. See [`UPSTREAM_AUTH_PREFIXES`].
pub fn is_upstream_auth_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    UPSTREAM_AUTH_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
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

/// Resolve the throttling identity (client IP as a string) honouring
/// `TRUSTED_PROXY_COUNT` trusted `X-Forwarded-For` hops.
///
/// The chain is ordered from the closest hop (the socket peer) outward toward
/// the original client: `[peer, xff_rightmost, .., xff_leftmost]`. We then skip
/// `TRUSTED_PROXY_COUNT` trusted hops. With `0` this is exactly the socket peer
/// (previous behaviour); with `1` behind an ingress it is the address the
/// ingress recorded for the client. Falls back to `"unknown"` when neither a
/// peer nor a usable forwarded address is available.
pub fn throttle_client_ip(forwarded_for: Option<&str>, peer_ip: Option<IpAddr>) -> String {
    let mut chain: Vec<String> = Vec::new();
    if let Some(peer) = peer_ip {
        chain.push(peer.to_string());
    }
    if let Some(forwarded_for) = forwarded_for {
        for entry in forwarded_for.split(',').rev() {
            let entry = entry.trim();
            if !entry.is_empty() {
                chain.push(entry.to_string());
            }
        }
    }
    if chain.is_empty() {
        return "unknown".to_string();
    }
    let index = (*TRUSTED_PROXY_COUNT).min(chain.len() - 1);
    chain[index].clone()
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
    fn upstream_auth_headers_are_stripped() {
        // Kubernetes native impersonation.
        assert!(is_upstream_auth_header("Impersonate-User"));
        assert!(is_upstream_auth_header("impersonate-group"));
        assert!(is_upstream_auth_header("Impersonate-Uid"));
        assert!(is_upstream_auth_header("Impersonate-Extra-scopes"));
        // Conventional requestheader auth names.
        assert!(is_upstream_auth_header("X-Remote-User"));
        assert!(is_upstream_auth_header("x-remote-group"));
        assert!(is_upstream_auth_header("X-Remote-Extra-foo"));
        // Not identity headers.
        assert!(!is_upstream_auth_header("authorization"));
        assert!(!is_upstream_auth_header("x-forwarded-user"));
        assert!(!is_upstream_auth_header("content-type"));
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
    fn throttle_client_ip_defaults_to_the_socket_peer() {
        // With TRUSTED_PROXY_COUNT unset (0), the direct peer wins even if an
        // X-Forwarded-For is present, so a forged XFF cannot spoof the identity.
        let peer: IpAddr = "198.51.100.2".parse().unwrap();
        assert_eq!(
            throttle_client_ip(Some("203.0.113.7, 10.0.0.1"), Some(peer)),
            "198.51.100.2"
        );
        assert_eq!(throttle_client_ip(None, Some(peer)), "198.51.100.2");
    }

    #[test]
    fn throttle_client_ip_falls_back_to_forwarded_then_unknown() {
        // No peer: the rightmost forwarded hop is used (index 0 of the chain).
        assert_eq!(
            throttle_client_ip(Some("203.0.113.7, 10.0.0.1"), None),
            "10.0.0.1"
        );
        assert_eq!(throttle_client_ip(None, None), "unknown");
        assert_eq!(throttle_client_ip(Some("   "), None), "unknown");
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
