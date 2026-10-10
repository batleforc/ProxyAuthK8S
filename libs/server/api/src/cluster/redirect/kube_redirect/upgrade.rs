//! Tunnelling upgraded connections (exec, attach, port-forward, proxy, watch).
//!
//! The handshake is checked here, then [`connect`] opens the raw upstream
//! socket and writes the hand-serialized request, [`response`] reads the
//! upstream answer (relaying a refusal), and [`tunnel`] pipes the upgraded
//! connection both ways.

use super::context::RedirectContext;

use actix_web::{HttpRequest, HttpResponse, http, web};
use common::State;
use crd::ProxyKubeApi;
use crd::security::PortPolicy;
use crd::security::path_matcher::percent_decode_once;
use tokio::io::AsyncWriteExt;
use tracing::{error, instrument, warn};

use super::port_forward::{self, PortForwardFilter};
use crate::cluster::redirect::audit::AuditContext;
use crate::cluster::redirect::status_response::{bad_request, forbidden};

mod connect;
mod response;
mod tunnel;

use connect::{BoxedAsyncIo, connect_upgrade_stream, serialize_upgrade_request};
use response::{UpstreamHead, read_upgrade_response_headers, refused_upgrade_response};

/// Subresources the apiserver serves over an upgraded connection.
const STREAMING_SUBRESOURCES: [&str; 4] = ["exec", "attach", "portforward", "proxy"];

/// Whether the request is an upgrade handshake that must be tunnelled.
///
/// Both `Connection: upgrade` and an `Upgrade` protocol are required (RFC 9110
/// §7.8), on a method and path the apiserver actually upgrades: the streaming
/// subresources and watches. Anything else goes through the standard path, where
/// `reqwest` owns the request framing, so an `Upgrade` header on an arbitrary
/// request cannot open a raw socket to the cluster.
pub(super) fn is_upgrade_request(req: &HttpRequest, upstream_path: &str) -> bool {
    let has_upgrade_header = req
        .headers()
        .get(http::header::UPGRADE)
        .is_some_and(|value| !value.is_empty());
    let connection_has_upgrade_token = req
        .headers()
        .get_all(http::header::CONNECTION)
        .filter_map(|v| v.to_str().ok())
        .any(|v| {
            v.split(',').any(|token| {
                token
                    .trim()
                    .eq_ignore_ascii_case(http::header::UPGRADE.as_str())
            })
        });

    has_upgrade_header
        && connection_has_upgrade_token
        && matches!(*req.method(), http::Method::GET | http::Method::POST)
        && is_upgrade_target(upstream_path, req.query_string())
}

/// Whether the apiserver can upgrade a request to `upstream_path`.
fn is_upgrade_target(upstream_path: &str, query: &str) -> bool {
    // Decoded as the apiserver decodes them, so `exe%63` is still `exec`.
    let is_streaming_path = upstream_path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(percent_decode_once)
        .any(|segment| segment == "watch" || STREAMING_SUBRESOURCES.contains(&segment.as_str()));
    let is_watch_query = query
        .split('&')
        .any(|param| param == "watch=true" || param == "watch=1");

    is_streaming_path || is_watch_query
}

/// Whether the client declares a request body on the upgrade path.
///
/// Upgrade handshakes (websocket / SPDY exec, attach, port-forward, watch) never
/// carry a request body. The upgrade path hand-serializes the request onto a raw
/// upstream socket, so a client-supplied `Content-Length`/`Transfer-Encoding`
/// plus a body would let an attacker control request framing and smuggle a
/// second request past the proxy's authorization and identity stamping. Refusing
/// any declared body closes that vector.
fn upgrade_request_declares_body(req: &HttpRequest) -> bool {
    if req.headers().contains_key(http::header::TRANSFER_ENCODING) {
        return true;
    }
    match req
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
        }) {
        // No Content-Length header at all.
        None => false,
        // Present and parses to zero.
        Some(Some(0)) => false,
        // Present with a non-zero or unparseable value: treat as a body.
        Some(_) => true,
    }
}

/// Why an upgrade is answered without a tunnel. [`UpgradeRefusal::respond`]
/// owns the log line, audit status and client response of each case.
#[derive(Debug)]
enum UpgradeRefusal {
    /// Only websocket is tunnelled (see [`check_handshake`]).
    NotWebsocket,
    /// The port-forward query asks for a port outside the policy.
    PortOutsidePolicy(String),
    /// The client declared a request body.
    DeclaresBody,
    /// The upstream URL built for the request does not parse.
    InvalidUpstreamUrl(String),
    /// TCP connect / TLS handshake to the upstream failed.
    ConnectFailed(String),
    /// Writing the handshake upstream failed.
    WriteFailed(std::io::Error),
    /// Flushing the handshake upstream failed.
    FlushFailed(std::io::Error),
    /// The upstream response head could not be read.
    BadResponseHead(String),
    /// The upstream accepted a port-forward protocol the proxy cannot filter.
    UnfilterablePortForward(String),
}

impl UpgradeRefusal {
    /// Log the refusal, record its audit event and build the client answer.
    fn respond(self, audit: &AuditContext) -> HttpResponse {
        match self {
            Self::NotWebsocket => {
                audit.emit(400);
                bad_request(
                    "this proxy only tunnels websocket upgrades; SPDY is not supported, use a kubectl recent enough to use websockets for exec, attach and port-forward",
                )
            }
            Self::PortOutsidePolicy(reason) => {
                warn!(%reason, "refusing a port-forward to a port outside the allowed list");
                audit.emit(403);
                forbidden(&reason)
            }
            Self::DeclaresBody => {
                audit.emit(400);
                HttpResponse::BadRequest().body("upgrade requests must not carry a body")
            }
            Self::InvalidUpstreamUrl(err) => {
                error!(error = %err, "invalid upstream url for upgrade request");
                audit.emit(502);
                HttpResponse::BadGateway().body("bad gateway")
            }
            Self::ConnectFailed(err) => {
                error!(error = %err, "could not open the upstream upgrade stream");
                audit.emit(503);
                HttpResponse::ServiceUnavailable().body("upstream unavailable")
            }
            Self::WriteFailed(err) => {
                error!(error = %err, "could not write the upgrade request upstream");
                audit.emit(503);
                HttpResponse::ServiceUnavailable().body("upstream unavailable")
            }
            Self::FlushFailed(err) => {
                error!(error = %err, "could not flush the upgrade request upstream");
                audit.emit(503);
                HttpResponse::ServiceUnavailable().body("upstream unavailable")
            }
            Self::BadResponseHead(err) => {
                error!(error = %err, "could not read the upstream upgrade response headers");
                audit.emit(502);
                HttpResponse::BadGateway().body("bad gateway")
            }
            Self::UnfilterablePortForward(reason) => {
                warn!(%reason, "refusing a port-forward the proxy cannot filter");
                audit.emit(403);
                forbidden(&reason)
            }
        }
    }
}

/// Whether the client asks for a websocket upgrade.
fn is_websocket_upgrade(req: &HttpRequest) -> bool {
    req.headers()
        .get(http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("websocket"))
}

/// Refuse a handshake the proxy will not tunnel. On success, tells whether the
/// request is a port-forward restricted by `port_policy`.
fn check_handshake(req: &HttpRequest, port_policy: &PortPolicy) -> Result<bool, UpgradeRefusal> {
    // Only websocket is tunnelled: actix-http hands the bytes that follow the
    // handshake to the handler for `Upgrade: websocket` alone, so a SPDY
    // session would die right after its `101`. Refusing it up front gives the
    // client a readable error instead.
    if !is_websocket_upgrade(req) {
        return Err(UpgradeRefusal::NotWebsocket);
    }

    // The query fixes the ports of the channel protocols, and a SPDY tunnel
    // must not ask for others either.
    let is_restricted_port_forward =
        port_policy.is_restricted() && port_forward::is_port_forward_path(req.path());
    if is_restricted_port_forward {
        port_forward::check_query_ports(req.query_string(), port_policy)
            .map_err(UpgradeRefusal::PortOutsidePolicy)?;
    }

    // Upgrade handshakes never carry a body; a declared body here is an attempt
    // to smuggle a second request onto the raw upstream socket.
    if upgrade_request_declares_body(req) {
        return Err(UpgradeRefusal::DeclaresBody);
    }
    Ok(is_restricted_port_forward)
}

/// Connect to the upstream and send it the serialized handshake.
async fn open_upstream(
    proxy: &ProxyKubeApi,
    data: &web::Data<State>,
    upstream_url: &reqwest::Url,
    request_bytes: &[u8],
) -> Result<BoxedAsyncIo, UpgradeRefusal> {
    let mut upstream = connect_upgrade_stream(proxy, data, upstream_url)
        .await
        .map_err(UpgradeRefusal::ConnectFailed)?;
    upstream
        .write_all(request_bytes)
        .await
        .map_err(UpgradeRefusal::WriteFailed)?;
    upstream
        .flush()
        .await
        .map_err(UpgradeRefusal::FlushFailed)?;
    Ok(upstream)
}

/// A restricted port-forward is only tunnelled over a protocol whose ports are
/// known: fixed by the (checked) query, or read by the filter.
fn port_forward_filter(
    head: &UpstreamHead,
    port_policy: &PortPolicy,
) -> Result<Option<PortForwardFilter>, UpgradeRefusal> {
    port_forward::filter_for_accepted_protocol(
        head.header("sec-websocket-protocol"),
        head.header("sec-websocket-extensions"),
        port_policy,
    )
    .map_err(UpgradeRefusal::UnfilterablePortForward)
}

/// Record the status the client is answered with, on the span and the audit log.
fn record_status(audit: &AuditContext, status: http::StatusCode) {
    tracing::Span::current().record("http.response.status_code", status.as_u16());
    audit.emit(status.as_u16());
}

// `proxy` is `Empty` so the whole `ProxyKubeApi` is not formatted into the span
// on every proxied request (an `#[instrument]` field is rendered at span creation
// whatever the subscriber's level); it is recorded only under DEBUG.
#[instrument(
    skip_all,
    fields(http.method = %ctx.method, peer_addr = ?ctx.peer_addr, proxy = tracing::field::Empty, url_to_call)
)]
pub(super) async fn upgrade_redirect(
    ctx: RedirectContext,
    port_policy: PortPolicy,
) -> HttpResponse {
    let url_to_call = ctx.url_to_call();
    tracing::Span::current().record("url_to_call", ctx.url_without_query().as_str());
    if tracing::enabled!(tracing::Level::DEBUG) {
        tracing::Span::current().record("proxy", tracing::field::debug(&ctx.proxy));
    }
    let RedirectContext {
        req,
        data,
        payload,
        method,
        peer_addr,
        proxy,
        user,
        audit,
        ..
    } = ctx;

    let outcome = async {
        let is_restricted_port_forward = check_handshake(&req, &port_policy)?;
        let upstream_url = reqwest::Url::parse(&url_to_call)
            .map_err(|err| UpgradeRefusal::InvalidUpstreamUrl(err.to_string()))?;
        let request_bytes =
            serialize_upgrade_request(&req, &method, &upstream_url, peer_addr, user.as_ref());
        let mut upstream = open_upstream(&proxy, &data, &upstream_url, &request_bytes).await?;
        let head = read_upgrade_response_headers(&mut upstream)
            .await
            .map_err(UpgradeRefusal::BadResponseHead)?;

        // Only a completed `101 Switching Protocols` turns the connection into
        // a tunnel. On any other answer the upstream keeps parsing HTTP on that
        // socket, so piping the client's remaining bytes would let them through
        // as a second request that skipped authorization, the allow-list and
        // identity stamping. Answer with the refusal and drop both connections.
        let Some(upgrade_protocol) = head.switched_protocol() else {
            record_status(&audit, head.status);
            return Ok(refused_upgrade_response(&mut upstream, head).await);
        };

        let filter = if is_restricted_port_forward {
            port_forward_filter(&head, &port_policy)?
        } else {
            None
        };

        record_status(&audit, head.status);
        Ok(tunnel::tunnel(upstream, head, upgrade_protocol, payload, filter).await)
    };

    outcome
        .await
        .unwrap_or_else(|refusal: UpgradeRefusal| refusal.respond(&audit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test::TestRequest;

    const PORT_FORWARD: &str = "/clusters/ns/c/api/v1/namespaces/d/pods/p/portforward";

    fn websocket(uri: &str) -> TestRequest {
        TestRequest::get()
            .uri(uri)
            .insert_header(("upgrade", "websocket"))
            .insert_header(("connection", "Upgrade"))
    }

    fn only_8080() -> PortPolicy {
        PortPolicy::Only(std::iter::once(8080..=8080).collect())
    }

    #[test]
    fn watches_and_streaming_subresources_are_upgrade_targets() {
        assert!(is_upgrade_target("/api/v1/namespaces/d/pods/p/exe%63", ""));
        assert!(is_upgrade_target("/api/v1/watch/pods", ""));
        assert!(is_upgrade_target("/api/v1/pods", "watch=1"));
        assert!(!is_upgrade_target("/api/v1/pods", "watch=false"));
        assert!(!is_upgrade_target("/api/v1/namespaces/d/pods/p/log", ""));
    }

    #[test]
    fn only_a_zero_content_length_is_bodyless() {
        let declares = |req: TestRequest| upgrade_request_declares_body(&req.to_http_request());
        assert!(!declares(TestRequest::get()));
        assert!(!declares(
            TestRequest::get().insert_header(("content-length", "0"))
        ));
        assert!(declares(
            TestRequest::get().insert_header(("content-length", "4"))
        ));
        assert!(declares(
            TestRequest::get().insert_header(("content-length", "x"))
        ));
        assert!(declares(
            TestRequest::get().insert_header(("transfer-encoding", "chunked"))
        ));
    }

    #[test]
    fn a_spdy_handshake_is_refused() {
        let req = TestRequest::get()
            .insert_header(("upgrade", "SPDY/3.1"))
            .to_http_request();
        assert!(matches!(
            check_handshake(&req, &PortPolicy::Any),
            Err(UpgradeRefusal::NotWebsocket)
        ));
        assert!(is_websocket_upgrade(
            &TestRequest::get()
                .insert_header(("upgrade", " WebSocket "))
                .to_http_request()
        ));
    }

    #[test]
    fn port_forward_ports_are_checked_against_the_policy() {
        let allowed = websocket(&format!("{PORT_FORWARD}?ports=8080")).to_http_request();
        assert!(check_handshake(&allowed, &only_8080()).unwrap());

        let refused = websocket(&format!("{PORT_FORWARD}?ports=22")).to_http_request();
        assert!(matches!(
            check_handshake(&refused, &only_8080()),
            Err(UpgradeRefusal::PortOutsidePolicy(_))
        ));

        // Unrestricted policy: never a restricted port-forward.
        assert!(!check_handshake(&refused, &PortPolicy::Any).unwrap());
    }

    #[test]
    fn a_declared_body_is_refused_after_the_port_check() {
        let req = websocket("/api/v1/namespaces/d/pods/p/exec")
            .insert_header(("content-length", "10"))
            .to_http_request();
        assert!(matches!(
            check_handshake(&req, &PortPolicy::Any),
            Err(UpgradeRefusal::DeclaresBody)
        ));
    }

    #[test]
    fn each_refusal_keeps_its_status() {
        let audit = AuditContext::new("ns", "c", "GET", "/api");
        let io = || std::io::Error::other("io");
        let cases = [
            (UpgradeRefusal::NotWebsocket, 400),
            (UpgradeRefusal::PortOutsidePolicy("p".into()), 403),
            (UpgradeRefusal::DeclaresBody, 400),
            (UpgradeRefusal::InvalidUpstreamUrl("u".into()), 502),
            (UpgradeRefusal::ConnectFailed("c".into()), 503),
            (UpgradeRefusal::WriteFailed(io()), 503),
            (UpgradeRefusal::FlushFailed(io()), 503),
            (UpgradeRefusal::BadResponseHead("h".into()), 502),
            (UpgradeRefusal::UnfilterablePortForward("f".into()), 403),
        ];
        for (refusal, status) in cases {
            let label = format!("{refusal:?}");
            assert_eq!(refusal.respond(&audit).status().as_u16(), status, "{label}");
        }
    }
}

#[cfg(test)]
mod security_contract_tests {
    //! Characterization tests for the upgrade path's security contract.
    //!
    //! Written deliberately BEFORE the planned merge of the response-header
    //! logic into a shared `copy_upstream_response_headers`. Tests added after a
    //! refactor can only pin whatever the refactor produced; these pin what the
    //! code is required to do, so they can still fail if the merge changes it.
    //!
    //! Scope is the two functions that encode a security contract rather than an
    //! implementation shape: the request-smuggling guard, and the serializer that
    //! strips client-supplied identity and stamps the proxy's own.

    use super::{serialize_upgrade_request, upgrade_request_declares_body};
    use crate::model::user::User;
    use actix_web::dev::PeerAddr;
    use actix_web::test::TestRequest;
    use actix_web::{HttpRequest, http};

    fn request(headers: &[(&str, &str)]) -> HttpRequest {
        let mut req = TestRequest::get().uri("/api/v1/pods");
        for (name, value) in headers {
            req = req.insert_header((*name, *value));
        }
        req.to_http_request()
    }

    fn user() -> User {
        User {
            username: "alice".to_string(),
            email: "alice@example.com".to_string(),
            groups: vec!["dev".to_string(), "platform".to_string()],
        }
    }

    fn serialize(headers: &[(&str, &str)], user: Option<&User>) -> String {
        let req = request(headers);
        let url = reqwest::Url::parse("https://cluster.example.com:6443/api/v1/pods?watch=true")
            .expect("test url should parse");
        String::from_utf8(serialize_upgrade_request(
            &req,
            &http::Method::GET,
            &url,
            Some(PeerAddr(
                "10.1.2.3:5555".parse().expect("addr should parse"),
            )),
            user,
        ))
        .expect("serialized request should be utf-8")
    }

    /// The smuggling guard. An upgrade handshake never carries a body, and this
    /// path hand-serializes onto a raw socket — a client-controlled framing
    /// header plus a body would let a second request ride past authorization.
    #[test]
    fn a_declared_body_is_refused_on_the_upgrade_path() {
        assert!(upgrade_request_declares_body(&request(&[(
            "content-length",
            "5"
        )])));
        assert!(upgrade_request_declares_body(&request(&[(
            "transfer-encoding",
            "chunked"
        )])));
        // Unparseable length: fail closed rather than guess.
        assert!(upgrade_request_declares_body(&request(&[(
            "content-length",
            "not-a-number"
        )])));
        // Transfer-Encoding wins even alongside a zero length — the pair is the
        // classic desync primitive.
        assert!(upgrade_request_declares_body(&request(&[
            ("content-length", "0"),
            ("transfer-encoding", "chunked"),
        ])));
    }

    #[test]
    fn a_bodyless_handshake_is_allowed() {
        assert!(!upgrade_request_declares_body(&request(&[])));
        assert!(!upgrade_request_declares_body(&request(&[(
            "content-length",
            "0"
        )])));
    }

    /// The framing headers must never reach the raw socket, or the guard above
    /// could be bypassed by anything that sets them later.
    #[test]
    fn framing_headers_are_stripped_from_the_serialized_request() {
        let wire = serialize(
            &[
                ("content-length", "0"),
                ("transfer-encoding", "chunked"),
                ("upgrade", "websocket"),
            ],
            Some(&user()),
        );
        let lower = wire.to_ascii_lowercase();
        assert!(!lower.contains("content-length"), "{wire}");
        assert!(!lower.contains("transfer-encoding"), "{wire}");
    }

    /// `connection`/`upgrade` are what make this an upgrade — stripping them
    /// would silently turn the handshake into an ordinary request.
    #[test]
    fn the_upgrade_headers_themselves_are_preserved() {
        let wire = serialize(
            &[("upgrade", "SPDY/3.1"), ("connection", "Upgrade")],
            Some(&user()),
        );
        // Header names are case-insensitive; the serializer rebuilds these two
        // with their canonical casing.
        let lower = wire.to_ascii_lowercase();
        assert!(lower.contains("upgrade: spdy/3.1"), "{wire}");
        assert!(lower.contains("connection: upgrade"), "{wire}");
    }

    /// The whole point of the proxy's identity stamping: a client must not be
    /// able to present itself as someone else by setting the headers the proxy
    /// owns.
    #[test]
    fn client_supplied_identity_headers_are_replaced_not_forwarded() {
        let wire = serialize(
            &[
                ("x-forwarded-user", "root"),
                ("x-forwarded-groups", "system:masters"),
                ("upgrade", "websocket"),
            ],
            Some(&user()),
        );

        assert!(
            !wire.contains("root"),
            "a spoofed x-forwarded-user reached the upstream: {wire}"
        );
        assert!(
            !wire.contains("system:masters"),
            "spoofed groups reached the upstream: {wire}"
        );
        assert!(wire.contains("x-forwarded-user: alice"), "{wire}");
        assert!(wire.contains("x-forwarded-groups: dev,platform"), "{wire}");
    }

    /// Impersonation headers are the upstream's own auth mechanism; a client
    /// that could set them would be talking to the apiserver as anyone.
    ///
    /// `Authorization` is deliberately NOT in that set: the caller's bearer
    /// token IS the cluster credential in this design, so it is forwarded. This
    /// test records that on purpose — the standard path applies the identical
    /// policy at `upstream.rs:68`, and the two must not drift apart.
    #[test]
    fn upstream_auth_headers_from_the_client_are_dropped() {
        let wire = serialize(
            &[
                ("impersonate-user", "system:admin"),
                ("impersonate-group", "system:masters"),
                ("x-remote-user", "root"),
                ("authorization", "Bearer the-callers-own-token"),
                ("upgrade", "websocket"),
            ],
            Some(&user()),
        );
        let lower = wire.to_ascii_lowercase();
        assert!(!lower.contains("impersonate-user"), "{wire}");
        assert!(!lower.contains("impersonate-group"), "{wire}");
        assert!(!lower.contains("x-remote-user"), "{wire}");
        assert!(
            lower.contains("authorization: bearer the-callers-own-token"),
            "the caller's own token is the upstream credential and must survive: {wire}"
        );
    }

    /// The Host must describe the upstream we actually opened a socket to, not
    /// whatever the client asked for.
    #[test]
    fn the_host_is_rewritten_to_the_upstream_authority() {
        let wire = serialize(
            &[("host", "evil.example.com"), ("upgrade", "websocket")],
            Some(&user()),
        );
        assert!(wire.contains("Host: cluster.example.com:6443"), "{wire}");
        assert!(!wire.contains("evil.example.com"), "{wire}");
    }

    /// The request line has to carry the query, or a `watch=true` upgrade
    /// silently becomes a non-watch request.
    #[test]
    fn the_request_line_keeps_method_path_and_query() {
        let wire = serialize(&[("upgrade", "websocket")], Some(&user()));
        assert!(
            wire.starts_with("GET /api/v1/pods?watch=true HTTP/1.1\r\n"),
            "{wire}"
        );
    }

    /// No resolved user means no identity headers at all — an absent header is
    /// safe, an empty one asserts an identity nobody holds.
    #[test]
    fn no_user_means_no_identity_headers() {
        let wire = serialize(&[("upgrade", "websocket")], None);
        let lower = wire.to_ascii_lowercase();
        assert!(!lower.contains("x-forwarded-user"), "{wire}");
        assert!(!lower.contains("x-forwarded-groups"), "{wire}");
    }

    #[test]
    fn the_forwarded_for_chain_is_appended_not_replaced() {
        let wire = serialize(
            &[("x-forwarded-for", "203.0.113.9"), ("upgrade", "websocket")],
            Some(&user()),
        );
        assert!(
            wire.contains("x-forwarded-for: 203.0.113.9, 10.1.2.3"),
            "the original client hop must survive: {wire}"
        );
    }

    /// Headers end with a blank line; without it the upstream waits forever.
    #[test]
    fn the_header_block_is_terminated() {
        let wire = serialize(&[("upgrade", "websocket")], Some(&user()));
        assert!(wire.ends_with("\r\n\r\n"), "{wire:?}");
    }
}
