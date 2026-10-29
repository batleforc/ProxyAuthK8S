//! Tunnelling upgraded connections (exec, attach, port-forward, proxy, watch).
//!
//! The handshake is checked here, then [`connect`] opens the upstream
//! connection and builds the handshake hyper sends on it, and [`tunnel`]
//! answers the client's `101` and pipes the upgraded connections both ways.
//! Any other upstream answer is relayed as a refusal.

use super::context::RedirectContext;

use axum::{
    body::Body,
    http::{self, HeaderMap, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use common::State;
use crd::ProxyKubeApi;
use crd::security::PortPolicy;
use crd::security::path_matcher::percent_decode_once;
use http_body_util::{BodyExt as _, Empty, Limited};
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::TokioIo;
use tracing::{error, instrument, warn};

use super::port_forward::{self, PortForwardFilter};
use crate::cluster::redirect::audit::{AuditContext, PortDecision};
use crate::cluster::redirect::status_response::{bad_request, forbidden};

mod connect;
mod tunnel;

use connect::{build_upgrade_request, connect_upgrade_stream};
use tunnel::relayed_response;

/// Upper bound on a non-upgraded upstream response read on the upgrade path.
/// Such a response is a refused handshake (an apiserver `Status`, a 4xx from a
/// proxied pod), buffered so the upstream connection can be dropped right after.
const MAX_REFUSED_RESPONSE_BYTES: usize = 1024 * 1024;

/// Subresources the apiserver serves over an upgraded connection.
const STREAMING_SUBRESOURCES: [&str; 4] = ["exec", "attach", "portforward", "proxy"];

/// Whether the request is an upgrade handshake that must be tunnelled.
///
/// Both `Connection: upgrade` and an `Upgrade` protocol are required (RFC 9110
/// §7.8), on a method and path the apiserver actually upgrades: the streaming
/// subresources and watches. Anything else goes through the standard path, where
/// `reqwest` owns the request framing, so an `Upgrade` header on an arbitrary
/// request cannot open a raw socket to the cluster.
pub(super) fn is_upgrade_request(req: &Parts, upstream_path: &str) -> bool {
    let has_upgrade_header = req
        .headers
        .get(http::header::UPGRADE)
        .is_some_and(|value| !value.is_empty());
    let connection_has_upgrade_token = req
        .headers
        .get_all(http::header::CONNECTION)
        .iter()
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
        && matches!(req.method, http::Method::GET | http::Method::POST)
        && is_upgrade_target(upstream_path, req.uri.query().unwrap_or_default())
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
/// carry a request body. hyper sends the handshake without one, but a client
/// body would still be bytes after the handshake that the client believes
/// framed: refusing any declared body keeps the anti-smuggling guard of the
/// hand-serialized path this replaced, and its `400`.
fn upgrade_request_declares_body(headers: &HeaderMap) -> bool {
    if headers.contains_key(http::header::TRANSFER_ENCODING) {
        return true;
    }
    match headers.get(http::header::CONTENT_LENGTH).map(|value| {
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
    /// The HTTP handshake with the upstream failed (sending it, or reading
    /// the answer).
    Handshake(String),
    /// The client connection cannot be upgraded (not HTTP/1.1).
    NotUpgradable,
    /// The upstream accepted a port-forward protocol the proxy cannot filter.
    UnfilterablePortForward(String),
    /// The upstream accepted an exec protocol whose channels the proxy
    /// cannot read, while exec is restricted (RFC 0004, T5).
    UnfilterableExec(String),
}

impl UpgradeRefusal {
    /// Log the refusal, record its audit event and build the client answer.
    fn respond(self, audit: &AuditContext) -> Response {
        match self {
            Self::NotWebsocket => {
                audit.emit_refusal(400);
                bad_request(
                    "this proxy only tunnels websocket upgrades; SPDY is not supported, use a kubectl recent enough to use websockets for exec, attach and port-forward; if a websocket attempt failed just before, kubectl -v=6 shows the upstream reason",
                )
            }
            Self::PortOutsidePolicy(reason) => {
                warn!(%reason, "refusing a port-forward to a port outside the allowed list");
                audit.emit_port_decision(PortDecision::Denied, None);
                audit.emit_refusal(403);
                forbidden(&reason)
            }
            Self::DeclaresBody => {
                audit.emit_refusal(400);
                (
                    StatusCode::BAD_REQUEST,
                    "upgrade requests must not carry a body",
                )
                    .into_response()
            }
            Self::InvalidUpstreamUrl(err) => {
                error!(error = %err, "invalid upstream url for upgrade request");
                audit.emit_refusal(502);
                (StatusCode::BAD_GATEWAY, "bad gateway").into_response()
            }
            Self::ConnectFailed(err) => {
                error!(error = %err, "could not open the upstream upgrade stream");
                audit.emit_refusal(503);
                (StatusCode::SERVICE_UNAVAILABLE, "upstream unavailable").into_response()
            }
            Self::Handshake(err) => {
                error!(error = %err, "the upgrade handshake with the upstream failed");
                audit.emit_refusal(502);
                (StatusCode::BAD_GATEWAY, "bad gateway").into_response()
            }
            Self::NotUpgradable => {
                audit.emit_refusal(400);
                bad_request("this connection cannot be upgraded; use HTTP/1.1")
            }
            Self::UnfilterablePortForward(reason) => {
                warn!(%reason, "refusing a port-forward the proxy cannot filter");
                audit.emit_refusal(403);
                forbidden(&reason)
            }
            Self::UnfilterableExec(reason) => {
                warn!(%reason, "refusing an exec the proxy cannot filter");
                audit.emit_refusal(403);
                forbidden(&reason)
            }
        }
    }
}

/// Whether the client asks for a websocket upgrade.
fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("websocket"))
}

/// Refuse a handshake the proxy will not tunnel. On success, tells whether the
/// request is a port-forward restricted by `port_policy`, and the ports its
/// query asks for.
fn check_handshake(
    req: &Parts,
    port_policy: &PortPolicy,
) -> Result<(bool, Vec<u16>), UpgradeRefusal> {
    // The policies come first: kubectl falls back to SPDY on any failed
    // websocket upgrade, a 403 included, and that retry must get the same
    // refusal and its reason rather than "SPDY is not supported".
    // The query fixes the ports of the channel protocols, and a SPDY tunnel
    // must not ask for others either.
    let is_restricted_port_forward =
        port_policy.is_restricted() && port_forward::is_port_forward_path(req.uri.path());
    let query_ports = if is_restricted_port_forward {
        port_forward::check_query_ports(req.uri.query().unwrap_or_default(), port_policy)
            .map_err(UpgradeRefusal::PortOutsidePolicy)?
    } else {
        Vec::new()
    };

    // Only websocket is tunnelled (RFC 0007, decision 1): SPDY is refused up
    // front with a readable error, as it was before the move to hyper.
    if !is_websocket_upgrade(&req.headers) {
        return Err(UpgradeRefusal::NotWebsocket);
    }

    // Upgrade handshakes never carry a body; a declared body here is an attempt
    // to smuggle a second request onto the upstream connection.
    if upgrade_request_declares_body(&req.headers) {
        return Err(UpgradeRefusal::DeclaresBody);
    }
    Ok((is_restricted_port_forward, query_ports))
}

/// Connect to the upstream and send it the handshake; the upstream's answer,
/// still to be upgraded or relayed.
async fn open_upstream(
    proxy: &ProxyKubeApi,
    data: &State,
    upstream_url: &reqwest::Url,
    request: http::Request<Empty<Bytes>>,
) -> Result<http::Response<hyper::body::Incoming>, UpgradeRefusal> {
    let stream = connect_upgrade_stream(proxy, data, upstream_url)
        .await
        .map_err(UpgradeRefusal::ConnectFailed)?;
    // One connection per session, no pool: an exec or a port-forward holds
    // it for hours.
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|err| UpgradeRefusal::Handshake(err.to_string()))?;
    tokio::spawn(async move {
        if let Err(err) = connection.with_upgrades().await {
            warn!(%err, "upstream upgrade connection ended with an error");
        }
    });
    sender
        .send_request(request)
        .await
        .map_err(|err| UpgradeRefusal::Handshake(err.to_string()))
}

/// The `message` of a Kubernetes `Status` body, the reason an apiserver gives
/// for refusing an upgrade; `None` for any other body.
fn status_message(body: &[u8]) -> Option<String> {
    let status: serde_json::Value = serde_json::from_slice(body).ok()?;
    (status.get("kind")?.as_str()? == "Status")
        .then(|| status.get("message")?.as_str().map(str::to_string))
        .flatten()
        .filter(|message| !message.is_empty())
}

/// Relay an upstream answer that is not a `101`, read in full (bounded) so
/// the upstream connection is dropped with it, and audit it with the reason
/// the upstream gave (RFC 0008 decision 13): kubectl's SPDY fallback hides it
/// from the user.
async fn refused_upgrade_response(
    response: http::Response<hyper::body::Incoming>,
    audit: &AuditContext,
) -> Response {
    let (head, body) = response.into_parts();
    let body = match Limited::new(body, MAX_REFUSED_RESPONSE_BYTES)
        .collect()
        .await
    {
        Ok(body) => body.to_bytes(),
        Err(err) => {
            error!(error = %err, status = %head.status, "could not read the refused upgrade response");
            tracing::Span::current().record("http.response.status_code", 502);
            audit.emit_refusal(502);
            return (
                StatusCode::BAD_GATEWAY,
                [(header::CONNECTION, "close")],
                "bad gateway",
            )
                .into_response();
        }
    };
    record_status(audit, head.status, status_message(&body).as_deref());
    let mut response = relayed_response(head.status, &head.headers, Body::from(body));
    // The client is not upgraded either: close its connection, as before.
    response.headers_mut().insert(
        header::CONNECTION,
        header::HeaderValue::from_static("close"),
    );
    response
}

/// A restricted port-forward is only tunnelled over a protocol whose ports are
/// known: fixed by the (checked) query, or read by the filter.
fn port_forward_filter(
    headers: &HeaderMap,
    port_policy: &PortPolicy,
) -> Result<Option<PortForwardFilter>, UpgradeRefusal> {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    port_forward::filter_for_accepted_protocol(
        header("sec-websocket-protocol"),
        header("sec-websocket-extensions"),
        port_policy,
    )
    .map_err(UpgradeRefusal::UnfilterablePortForward)
}

/// The protocol the connection switched to, only for a completed
/// `101 Switching Protocols` that names one.
fn switched_protocol(response: &http::Response<hyper::body::Incoming>) -> Option<String> {
    (response.status() == StatusCode::SWITCHING_PROTOCOLS)
        .then(|| response.headers().get(header::UPGRADE)?.to_str().ok())
        .flatten()
        .map(str::to_string)
}

/// Record the status the client is answered with, on the span and the audit
/// log, with the upstream's reason for a refused upgrade.
fn record_status(audit: &AuditContext, status: StatusCode, upstream_reason: Option<&str>) {
    tracing::Span::current().record("http.response.status_code", status.as_u16());
    audit.emit_upstream_refusal(status.as_u16(), upstream_reason);
}

// `proxy` is `Empty` so the whole `ProxyKubeApi` is not formatted into the span
// on every proxied request (an `#[instrument]` field is rendered at span creation
// whatever the subscriber's level); it is recorded only under DEBUG.
#[instrument(
    skip_all,
    fields(http.method = %ctx.method, peer_addr = ?ctx.peer_ip, proxy = tracing::field::Empty, url_to_call)
)]
/// `exec_streams` is `Some((stdin, tty))` for an exec or attach the policy
/// restricts: its channels are filtered on the tunnel (RFC 0004).
pub(super) async fn upgrade_redirect(
    ctx: RedirectContext,
    port_policy: PortPolicy,
    exec_streams: Option<(bool, bool)>,
) -> Response {
    let url_to_call = ctx.url_to_call();
    tracing::Span::current().record("url_to_call", ctx.url_without_query().as_str());
    if tracing::enabled!(tracing::Level::DEBUG) {
        tracing::Span::current().record("proxy", tracing::field::debug(&ctx.proxy));
    }
    let RedirectContext {
        mut req,
        data,
        method,
        peer_ip,
        proxy,
        user,
        audit,
        ..
    } = ctx;

    let outcome = async {
        let (is_restricted_port_forward, query_ports) = check_handshake(&req, &port_policy)?;
        let upstream_url = reqwest::Url::parse(&url_to_call)
            .map_err(|err| UpgradeRefusal::InvalidUpstreamUrl(err.to_string()))?;
        let request =
            build_upgrade_request(&req.headers, &method, &upstream_url, peer_ip, user.as_ref())
                .map_err(UpgradeRefusal::InvalidUpstreamUrl)?;
        let client_upgrade = req
            .extensions
            .remove::<OnUpgrade>()
            .ok_or(UpgradeRefusal::NotUpgradable)?;
        let mut response = open_upstream(&proxy, &data, &upstream_url, request).await?;

        // Only a completed `101 Switching Protocols` turns the connection into
        // a tunnel. On any other answer the upstream keeps parsing HTTP on that
        // connection, so piping the client's remaining bytes would let them
        // through as a second request that skipped authorization, the
        // allow-list and identity stamping. Answer with the refusal and drop
        // both connections.
        let Some(upgrade_protocol) = switched_protocol(&response) else {
            return Ok(refused_upgrade_response(response, &audit).await);
        };

        let filter = if is_restricted_port_forward {
            port_forward_filter(response.headers(), &port_policy)?.map(tunnel::SessionFilter::Port)
        } else if let Some((stdin, tty)) = exec_streams {
            let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok());
            super::exec::check_exec_protocol(
                header("sec-websocket-protocol"),
                header("sec-websocket-extensions"),
            )
            .map_err(UpgradeRefusal::UnfilterableExec)?;
            Some(tunnel::SessionFilter::Exec(
                super::exec::ExecChannelFilter::new(stdin, tty),
                super::exec::ExecChannelCounter::new(),
            ))
        } else {
            None
        };
        for port in query_ports {
            audit.emit_port_decision(PortDecision::Allowed, Some(port));
        }

        record_status(&audit, response.status(), None);
        let upstream_upgrade = hyper::upgrade::on(&mut response);
        Ok(tunnel::tunnel(
            client_upgrade,
            upstream_upgrade,
            response.headers(),
            &upgrade_protocol,
            audit.clone(),
            filter,
        ))
    };

    outcome
        .await
        .unwrap_or_else(|refusal: UpgradeRefusal| refusal.respond(&audit))
}

/// `actix_web::test::TestRequest`'s shape, producing the request head the
/// upgrade path reads.
#[cfg(test)]
pub(super) mod test_request {
    use axum::http::{Method, Request, request::Parts};

    pub(crate) struct TestRequest(axum::http::request::Builder);

    impl TestRequest {
        pub(crate) fn get() -> Self {
            Self(Request::builder().method(Method::GET).uri("/"))
        }

        pub(crate) fn uri(self, uri: &str) -> Self {
            Self(self.0.uri(uri))
        }

        pub(crate) fn insert_header(self, (name, value): (&str, &str)) -> Self {
            Self(self.0.header(name, value))
        }

        #[expect(
            clippy::wrong_self_convention,
            reason = "actix's name, so the ported tests read as they did"
        )]
        pub(crate) fn to_http_request(self) -> Parts {
            self.0.body(()).expect("valid test request").into_parts().0
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_status_message_is_the_upstream_reason() {
        let body = br#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"pod web does not have a host assigned","code":400}"#;
        assert_eq!(
            super::status_message(body).as_deref(),
            Some("pod web does not have a host assigned")
        );
        assert_eq!(super::status_message(b"<html>nope</html>"), None);
        assert_eq!(
            super::status_message(br#"{"kind":"Pod","message":"x"}"#),
            None
        );
        assert_eq!(
            super::status_message(br#"{"kind":"Status","message":""}"#),
            None
        );
    }

    use super::test_request::TestRequest;
    use super::*;

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
        let declares =
            |req: TestRequest| upgrade_request_declares_body(&req.to_http_request().headers);
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
                .headers
        ));
    }

    #[test]
    fn port_forward_ports_are_checked_against_the_policy() {
        let allowed = websocket(&format!("{PORT_FORWARD}?ports=8080")).to_http_request();
        assert_eq!(
            check_handshake(&allowed, &only_8080()).unwrap(),
            (true, vec![8080])
        );

        let refused = websocket(&format!("{PORT_FORWARD}?ports=22")).to_http_request();
        assert!(matches!(
            check_handshake(&refused, &only_8080()),
            Err(UpgradeRefusal::PortOutsidePolicy(_))
        ));

        // Unrestricted policy: never a restricted port-forward.
        assert_eq!(
            check_handshake(&refused, &PortPolicy::Any).unwrap(),
            (false, vec![])
        );
    }

    /// R13: kubectl retries a refused websocket upgrade in SPDY; the retry
    /// must get the port refusal, not "SPDY is not supported".
    #[test]
    fn the_port_policy_is_checked_before_the_websocket_check() {
        let spdy = TestRequest::get()
            .uri(&format!("{PORT_FORWARD}?ports=22"))
            .insert_header(("upgrade", "SPDY/3.1"))
            .to_http_request();
        assert!(matches!(
            check_handshake(&spdy, &only_8080()),
            Err(UpgradeRefusal::PortOutsidePolicy(_))
        ));
        let conforming = TestRequest::get()
            .uri(&format!("{PORT_FORWARD}?ports=8080"))
            .insert_header(("upgrade", "SPDY/3.1"))
            .to_http_request();
        assert!(matches!(
            check_handshake(&conforming, &only_8080()),
            Err(UpgradeRefusal::NotWebsocket)
        ));
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
        let cases = [
            (UpgradeRefusal::NotWebsocket, 400),
            (UpgradeRefusal::PortOutsidePolicy("p".into()), 403),
            (UpgradeRefusal::DeclaresBody, 400),
            (UpgradeRefusal::InvalidUpstreamUrl("u".into()), 502),
            (UpgradeRefusal::ConnectFailed("c".into()), 503),
            (UpgradeRefusal::Handshake("h".into()), 502),
            (UpgradeRefusal::NotUpgradable, 400),
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
    //! implementation shape: the request-smuggling guard, and the handshake
    //! builder that strips client-supplied identity and stamps the proxy's own.
    //! The handshake hyper will send is rendered as text, so the assertions
    //! read as they did on the hand-serialized request it replaced.

    use super::test_request::TestRequest;
    use super::{build_upgrade_request, upgrade_request_declares_body};
    use crate::model::user::User;
    use axum::http::{self, HeaderMap};

    fn request(headers: &[(&str, &str)]) -> HeaderMap {
        let mut req = TestRequest::get().uri("/api/v1/pods");
        for (name, value) in headers {
            req = req.insert_header((*name, *value));
        }
        req.to_http_request().headers
    }

    fn user() -> User {
        User {
            username: "alice".to_string(),
            email: "alice@example.com".to_string(),
            groups: vec!["dev".to_string(), "platform".to_string()],
            ..User::default()
        }
    }

    fn serialize(headers: &[(&str, &str)], user: Option<&User>) -> String {
        let req = request(headers);
        let url = reqwest::Url::parse("https://cluster.example.com:6443/api/v1/pods?watch=true")
            .expect("test url should parse");
        let built = build_upgrade_request(
            &req,
            &http::Method::GET,
            &url,
            Some("10.1.2.3".parse().expect("ip should parse")),
            user,
        )
        .expect("the handshake builds");
        let mut wire = format!("{} {} HTTP/1.1\r\n", built.method(), built.uri());
        for (name, value) in built.headers() {
            wire.push_str(&format!("{name}: {}\r\n", value.to_str().unwrap()));
        }
        wire
    }

    /// The smuggling guard. An upgrade handshake never carries a body — a
    /// client-controlled framing header plus a body would let a second request
    /// ride past authorization.
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

    /// The framing headers must never reach the upstream, or the guard above
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
        assert!(wire.contains("host: cluster.example.com:6443"), "{wire}");
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
}
