//! Port filtering for the paths that open a container, Service or node port.
//!
//! [`port_target`] classifies every upstream path once; then, for
//! `pods/{name}/portforward` sessions:
//!
//! Where the ports travel depends on the websocket subprotocol the upstream
//! accepts:
//! - `v4.channel.k8s.io`, `v4.base64.channel.k8s.io` or none: the ports are
//!   fixed by the `ports` query parameter, checked before connecting.
//! - `SPDY/3.1+portforward.k8s.io` (kubectl): SPDY tunnelled in websocket
//!   frames, one SPDY stream per forwarded port, the port in the `port` header
//!   of each `SYN_STREAM`. Those headers are zlib-compressed with one context
//!   per direction for the whole session, so every client frame goes through
//!   [`PortForwardFilter`] before it is forwarded.
//!
//! Anything the filter cannot understand closes the session: a port that cannot
//! be read is treated as a port that is not allowed.

use crd::security::PortPolicy;
use crd::security::path_matcher::percent_decode_once;
use flate2::{Decompress, FlushDecompress, Status};

use super::spdy_dictionary::SPDY3_HEADER_DICTIONARY;
use super::websocket::WebsocketDecoder;

/// Subprotocol prefix of SPDY tunnelled over websocket.
const SPDY_TUNNEL_PREFIX: &str = "spdy/";

/// Subprotocols for which the kubelet takes the ports from the query string.
const QUERY_PORT_PROTOCOLS: [&str; 2] = ["v4.channel.k8s.io", "v4.base64.channel.k8s.io"];

/// Upper bound on a `SYN_STREAM`/`HEADERS` frame. Port-forward header blocks
/// are a few dozen bytes; the bound keeps a client from growing the buffer.
const MAX_SPDY_CONTROL_FRAME: usize = 64 * 1024;

/// Upper bound on a decompressed header block.
const MAX_SPDY_HEADER_BLOCK: usize = 256 * 1024;

const SPDY_VERSION: u16 = 3;
const SPDY_SYN_STREAM: u16 = 1;
const SPDY_HEADERS: u16 = 8;

/// Which port, if any, a request to an upstream path opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortTarget {
    /// The path opens no port.
    None,
    /// `pods/{name}/portforward`: ports in the query or the SPDY headers.
    PortForward,
    /// `pods/[scheme:]{name}:{port}/proxy[/...]` (a container port) or
    /// `services/[scheme:]{name}:{port}/proxy[/...]` (a Service port).
    ProxyPort(u16),
    /// A `proxy` subresource whose port is named, absent or unreadable: the
    /// proxy cannot tell which port it reaches.
    ProxyNamedOrMissing,
    /// `nodes/{node}[:{port}]/proxy[/...]`: the kubelet API itself.
    NodeProxy,
}

/// Classify `path` (the upstream path, without query).
///
/// Segments are decoded once, the way the apiserver decodes them, so an
/// encoded `portforwar%64` or `web%3A5432` is still recognised, while a
/// doubly encoded `web%253A5432` reaches it as a name without a port.
pub fn port_target(path: &str) -> PortTarget {
    let decoded: Vec<String> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(percent_decode_once)
        .collect();
    let segments: Vec<&str> = decoded.iter().map(String::as_str).collect();
    match segments.as_slice() {
        [
            "api",
            _,
            "namespaces",
            _,
            "pods" | "services",
            name,
            "proxy",
            ..,
        ] => proxy_port(name),
        ["api", _, "nodes", _, "proxy", ..] => PortTarget::NodeProxy,
        [.., "pods", _, "portforward"] => PortTarget::PortForward,
        _ => PortTarget::None,
    }
}

/// The port of a `[scheme:]name[:port]` proxy segment, as the apiserver splits
/// it: two parts are `name:port`, three are `scheme:name:port`.
fn proxy_port(segment: &str) -> PortTarget {
    let port = match segment.split(':').collect::<Vec<_>>().as_slice() {
        [_, port] | [_, _, port] => *port,
        _ => return PortTarget::ProxyNamedOrMissing,
    };
    parse_port(port).map_or(PortTarget::ProxyNamedOrMissing, PortTarget::ProxyPort)
}

/// Whether `path` (the upstream path, without query) is a pod port-forward.
pub(super) fn is_port_forward_path(path: &str) -> bool {
    port_target(path) == PortTarget::PortForward
}

/// Check the ports a port-forward asks for in its query string, returning
/// them when they are all allowed.
///
/// The apiserver reads `ports`, the kubelet `port`; both are checked, each
/// value may be a comma-separated list, as the kubelet accepts.
pub(super) fn check_query_ports(query: &str, policy: &PortPolicy) -> Result<Vec<u16>, String> {
    let mut ports = Vec::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode_once(&key.replace('+', " "));
        if !key.eq_ignore_ascii_case("ports") && !key.eq_ignore_ascii_case("port") {
            continue;
        }
        let value = percent_decode_once(&value.replace('+', " "));
        for port in value.split(',') {
            ports.push(check_port(port, policy)?);
        }
    }
    Ok(ports)
}

/// A port written as ASCII digits that fits a non-zero `u16`; digits only,
/// since `u16::from_str` would also accept a leading `+`.
fn parse_port(value: &str) -> Option<u16> {
    Some(value)
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|port| *port > 0)
}

fn check_port(value: &str, policy: &PortPolicy) -> Result<u16, String> {
    let port = parse_port(value).ok_or_else(|| format!("port \"{value}\" is not a valid port"))?;
    if policy.allows(port) {
        Ok(port)
    } else {
        Err(format!("port {port} is not allowed on this cluster"))
    }
}

/// How a restricted port-forward must be relayed, given the subprotocol and
/// extensions the upstream accepted in its `101`.
pub(super) fn filter_for_accepted_protocol(
    protocol: Option<&str>,
    extensions: Option<&str>,
    policy: &PortPolicy,
) -> Result<Option<PortForwardFilter>, String> {
    // A negotiated extension such as permessage-deflate would hide the SPDY
    // frames from the filter.
    if extensions.is_some_and(|value| !value.trim().is_empty()) {
        return Err("websocket extensions are not supported on a restricted port-forward".into());
    }

    let protocol = protocol.map(str::trim).unwrap_or_default();
    if protocol.is_empty()
        || QUERY_PORT_PROTOCOLS
            .iter()
            .any(|known| protocol.eq_ignore_ascii_case(known))
    {
        // Ports were fixed by the query string, already checked.
        return Ok(None);
    }
    let is_spdy_tunnel = protocol
        .get(..SPDY_TUNNEL_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(SPDY_TUNNEL_PREFIX));
    if is_spdy_tunnel {
        return Ok(Some(PortForwardFilter::new(policy.clone())));
    }
    Err(format!(
        "unsupported port-forward protocol \"{protocol}\" on a restricted port-forward"
    ))
}

/// Inspects the client side of a SPDY-over-websocket port-forward.
pub(super) struct PortForwardFilter {
    websocket: WebsocketDecoder,
    spdy: SpdyInspector,
}

impl PortForwardFilter {
    fn new(policy: PortPolicy) -> Self {
        Self {
            websocket: WebsocketDecoder::default(),
            spdy: SpdyInspector::new(policy),
        }
    }

    /// Feed the next bytes the client sent. An error means the session must be
    /// closed without forwarding `chunk`.
    pub(super) fn inspect(&mut self, chunk: &[u8]) -> Result<(), String> {
        let spdy = &mut self.spdy;
        self.websocket.decode(chunk, |payload| spdy.feed(payload))
    }

    /// Ports opened since the last call, each reported once per session.
    pub(super) fn take_opened(&mut self) -> Vec<u16> {
        std::mem::take(&mut self.spdy.opened)
    }

    /// After an [`inspect`](Self::inspect) error: the port a stream named and
    /// the policy refused, or `None` when the traffic could not be read.
    pub(super) fn refused_port(&self) -> Option<u16> {
        self.spdy.refused
    }
}

/// SPDY/3 frame reader over the tunnelled byte stream (draft 3, §2.2).
struct SpdyInspector {
    policy: PortPolicy,
    /// Allowed ports not yet reported, and every port reported so far: kubectl
    /// opens a data and an error stream per port, both naming it.
    opened: Vec<u16>,
    reported: Vec<u16>,
    /// The port a stream named when the policy refused it, for the audit.
    refused: Option<u16>,
    header: Vec<u8>,
    state: SpdyState,
    decompress: Decompress,
}

enum SpdyState {
    Header,
    /// Bytes of a frame that are not inspected.
    Skip(usize),
    /// A header-carrying control frame being buffered.
    Control {
        kind: u16,
        length: usize,
        body: Vec<u8>,
    },
}

impl SpdyInspector {
    fn new(policy: PortPolicy) -> Self {
        Self {
            policy,
            opened: Vec::new(),
            reported: Vec::new(),
            refused: None,
            header: Vec::with_capacity(8),
            state: SpdyState::Header,
            decompress: Decompress::new(true),
        }
    }

    fn feed(&mut self, mut input: &[u8]) -> Result<(), String> {
        while !input.is_empty() {
            match &mut self.state {
                SpdyState::Header => {
                    let take = 8_usize.saturating_sub(self.header.len()).min(input.len());
                    let (chunk, rest) = split(input, take)?;
                    self.header.extend_from_slice(chunk);
                    input = rest;
                    if self.header.len() == 8 {
                        self.state = self.start_frame()?;
                        self.header.clear();
                    }
                }
                SpdyState::Skip(remaining) => {
                    let take = (*remaining).min(input.len());
                    *remaining = remaining.saturating_sub(take);
                    input = split(input, take)?.1;
                    if *remaining == 0 {
                        self.state = SpdyState::Header;
                    }
                }
                SpdyState::Control { kind, length, body } => {
                    let take = length.saturating_sub(body.len()).min(input.len());
                    let (chunk, rest) = split(input, take)?;
                    body.extend_from_slice(chunk);
                    input = rest;
                    if body.len() == *length {
                        let (kind, body) = (*kind, std::mem::take(body));
                        self.state = SpdyState::Header;
                        self.check_control_frame(kind, &body)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn start_frame(&self) -> Result<SpdyState, String> {
        let &[h0, h1, h2, h3, _, h5, h6, h7] = self.header.as_slice() else {
            return Err("truncated SPDY frame header".into());
        };
        let length = usize::from(h5) << 16 | usize::from(h6) << 8 | usize::from(h7);
        let is_control = h0 & 0x80 != 0;
        if !is_control {
            return Ok(skip(length));
        }

        let version = u16::from_be_bytes([h0, h1]) & 0x7fff;
        if version != SPDY_VERSION {
            return Err(format!("unsupported SPDY version {version}"));
        }
        let kind = u16::from_be_bytes([h2, h3]);
        if kind != SPDY_SYN_STREAM && kind != SPDY_HEADERS {
            return Ok(skip(length));
        }
        if length > MAX_SPDY_CONTROL_FRAME {
            return Err("SPDY header frame exceeded the allowed size".into());
        }
        if length == 0 {
            return Err("empty SPDY header frame".into());
        }
        Ok(SpdyState::Control {
            kind,
            length,
            body: Vec::with_capacity(length),
        })
    }

    fn check_control_frame(&mut self, kind: u16, body: &[u8]) -> Result<(), String> {
        // SYN_STREAM: stream id, associated stream id, priority and slot.
        // HEADERS: stream id. The name/value block follows.
        let block_at = if kind == SPDY_SYN_STREAM { 10 } else { 4 };
        let block = body
            .get(block_at..)
            .ok_or_else(|| "truncated SPDY header frame".to_string())?;
        // Every block is inflated, even when it carries no port, to keep the
        // shared zlib context in step with the upstream's.
        let headers = self.inflate(block)?;
        for value in port_header_values(&headers)? {
            let port = check_port(value, &self.policy).inspect_err(|_| {
                self.refused = parse_port(value);
            })?;
            if !self.reported.contains(&port) {
                self.reported.push(port);
                self.opened.push(port);
            }
        }
        Ok(())
    }

    fn inflate(&mut self, input: &[u8]) -> Result<Vec<u8>, String> {
        let mut output = Vec::with_capacity(input.len().saturating_mul(4).max(256));
        let mut consumed = 0;
        loop {
            if output.len() == output.capacity() {
                if output.len() >= MAX_SPDY_HEADER_BLOCK {
                    return Err("SPDY header block exceeded the allowed size".into());
                }
                output.reserve(4096);
            }
            let (in_before, out_before) = (self.decompress.total_in(), output.len());
            let result = self.decompress.decompress_vec(
                input
                    .get(consumed..)
                    .ok_or_else(|| "SPDY header block overrun".to_string())?,
                &mut output,
                FlushDecompress::Sync,
            );
            consumed = usize::try_from(self.decompress.total_in().saturating_sub(in_before))
                .ok()
                .and_then(|read| consumed.checked_add(read))
                .ok_or_else(|| "SPDY header block too large".to_string())?;

            match result {
                Ok(Status::StreamEnd) => {
                    return Err("SPDY header compression stream ended".into());
                }
                Ok(_) => {
                    let output_full = output.len() == output.capacity();
                    if consumed == input.len() && !output_full {
                        return Ok(output);
                    }
                    let progressed =
                        self.decompress.total_in() != in_before || output.len() != out_before;
                    if !progressed && !output_full {
                        return Err("truncated SPDY header block".into());
                    }
                }
                Err(err) if err.needs_dictionary().is_some() => {
                    self.decompress
                        .set_dictionary(&SPDY3_HEADER_DICTIONARY)
                        .map_err(|e| format!("SPDY header dictionary rejected: {e}"))?;
                }
                Err(err) => return Err(format!("invalid SPDY header block: {err}")),
            }
        }
    }
}

/// `input` cut at `take`; the callers bound `take` by `input.len()`, so a
/// miss is an inspector bug, refused.
fn split(input: &[u8], take: usize) -> Result<(&[u8], &[u8]), String> {
    input
        .split_at_checked(take)
        .ok_or_else(|| "SPDY inspector overrun".to_string())
}

fn skip(length: usize) -> SpdyState {
    if length == 0 {
        SpdyState::Header
    } else {
        SpdyState::Skip(length)
    }
}

/// Values of every `port` header in a SPDY/3 name/value block (§2.6.10).
/// A header may carry several values separated by NUL bytes.
fn port_header_values(block: &[u8]) -> Result<Vec<&str>, String> {
    let mut rest = block;
    let count = u32::from_be_bytes(*take_chunk::<4>(&mut rest)?);

    let mut ports = Vec::new();
    for _ in 0..count {
        let name = take_length_prefixed(&mut rest)?;
        let value = take_length_prefixed(&mut rest)?;
        if name.eq_ignore_ascii_case(b"port") {
            let value = std::str::from_utf8(value)
                .map_err(|_| "SPDY port header is not valid UTF-8".to_string())?;
            ports.extend(value.split('\0'));
        }
    }
    Ok(ports)
}

fn take_chunk<'a, const N: usize>(rest: &mut &'a [u8]) -> Result<&'a [u8; N], String> {
    let (chunk, tail) = rest
        .split_first_chunk::<N>()
        .ok_or_else(|| "truncated SPDY header block".to_string())?;
    *rest = tail;
    Ok(chunk)
}

/// A 32-bit length followed by that many bytes.
fn take_length_prefixed<'a>(rest: &mut &'a [u8]) -> Result<&'a [u8], String> {
    let length = u32::from_be_bytes(*take_chunk::<4>(rest)?);
    let length = usize::try_from(length).map_err(|_| "truncated SPDY header block".to_string())?;
    if rest.len() < length {
        return Err("truncated SPDY header block".into());
    }
    let (chunk, tail) = rest.split_at(length);
    *rest = tail;
    Ok(chunk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};

    fn only(ports: &[u16]) -> PortPolicy {
        PortPolicy::Only(ports.iter().map(|port| *port..=*port).collect())
    }

    /// Header compressor matching spdystream's: zlib with the SPDY dictionary,
    /// sync-flushed after each block.
    struct HeaderCompressor(Compress);

    impl HeaderCompressor {
        fn new() -> Self {
            let mut compress = Compress::new(Compression::best(), true);
            compress
                .set_dictionary(&SPDY3_HEADER_DICTIONARY)
                .expect("dictionary");
            Self(compress)
        }

        fn compress(&mut self, headers: &[(&str, &str)]) -> Vec<u8> {
            let mut block = Vec::new();
            block.extend_from_slice(&u32::try_from(headers.len()).unwrap().to_be_bytes());
            for (name, value) in headers {
                block.extend_from_slice(&u32::try_from(name.len()).unwrap().to_be_bytes());
                block.extend_from_slice(name.as_bytes());
                block.extend_from_slice(&u32::try_from(value.len()).unwrap().to_be_bytes());
                block.extend_from_slice(value.as_bytes());
            }
            let mut out = Vec::with_capacity(block.len().saturating_add(64));
            self.0
                .compress_vec(&block, &mut out, FlushCompress::Sync)
                .expect("compress");
            out
        }
    }

    fn control_frame(kind: u16, body: &[u8]) -> Vec<u8> {
        let mut frame = vec![0x80, 0x03];
        frame.extend_from_slice(&kind.to_be_bytes());
        let length = u32::try_from(body.len()).unwrap();
        frame.push(0);
        frame.extend_from_slice(&length.to_be_bytes()[1..]);
        frame.extend_from_slice(body);
        frame
    }

    fn syn_stream(compressor: &mut HeaderCompressor, stream_id: u32, port: &str) -> Vec<u8> {
        let mut body = stream_id.to_be_bytes().to_vec();
        body.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        body.extend(compressor.compress(&[
            ("port", port),
            ("streamtype", "data"),
            ("requestid", "0"),
        ]));
        control_frame(SPDY_SYN_STREAM, &body)
    }

    fn data_frame(stream_id: u32, data: &[u8]) -> Vec<u8> {
        let mut frame = stream_id.to_be_bytes().to_vec();
        frame.push(0);
        frame.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes()[1..]);
        frame.extend_from_slice(data);
        frame
    }

    /// A masked client websocket frame.
    fn ws_frame(first: u8, payload: &[u8]) -> Vec<u8> {
        let mask = [0x12, 0x34, 0x56, 0x78];
        let mut frame = vec![first];
        match payload.len() {
            len @ 0..=125 => frame.push(0x80 | u8::try_from(len).unwrap()),
            len @ 126..=0xffff => {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&u16::try_from(len).unwrap().to_be_bytes());
            }
            len => {
                frame.push(0x80 | 127);
                frame.extend_from_slice(&(len as u64).to_be_bytes());
            }
        }
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        frame
    }

    fn binary(payload: &[u8]) -> Vec<u8> {
        ws_frame(0x82, payload)
    }

    #[test]
    fn port_forward_paths_are_recognised() {
        assert!(is_port_forward_path(
            "/api/v1/namespaces/dev/pods/web/portforward"
        ));
        assert!(is_port_forward_path(
            "/api/v1/namespaces/dev/pods/web/portforwar%64/"
        ));
        assert!(!is_port_forward_path(
            "/api/v1/namespaces/dev/pods/web/exec"
        ));
        assert!(!is_port_forward_path(
            "/api/v1/namespaces/dev/services/web/portforward"
        ));
    }

    #[test]
    fn port_target_classifies_every_port_bearing_path() {
        use PortTarget::{NodeProxy, None as NoPort, PortForward, ProxyNamedOrMissing, ProxyPort};
        const NS: &str = "/api/v1/namespaces/dev";
        let cases = [
            (format!("{NS}/pods/web/portforward"), PortForward),
            (format!("{NS}/pods/web:8080/proxy"), ProxyPort(8080)),
            (
                format!("{NS}/pods/web:8080/proxy/metrics/x"),
                ProxyPort(8080),
            ),
            (format!("{NS}/pods/https:web:8443/proxy/"), ProxyPort(8443)),
            (format!("{NS}/services/web:80/proxy"), ProxyPort(80)),
            (format!("{NS}/services/http:web:80/proxy/a"), ProxyPort(80)),
            // Decoded once, like the apiserver.
            (format!("{NS}/pods/web%3A5432/proxy"), ProxyPort(5432)),
            (format!("{NS}/pods/web%3a5432/prox%79"), ProxyPort(5432)),
            // Doubly encoded: the apiserver sees a name without a port.
            (format!("{NS}/pods/web%253A5432/proxy"), ProxyNamedOrMissing),
            // Named, missing, empty, too many parts, out of range, not ASCII.
            (format!("{NS}/services/web:http/proxy"), ProxyNamedOrMissing),
            (format!("{NS}/pods/web/proxy"), ProxyNamedOrMissing),
            (format!("{NS}/pods/web:/proxy"), ProxyNamedOrMissing),
            (format!("{NS}/pods/a:b:c:80/proxy"), ProxyNamedOrMissing),
            (format!("{NS}/pods/web:0/proxy"), ProxyNamedOrMissing),
            (format!("{NS}/pods/web:65536/proxy"), ProxyNamedOrMissing),
            (format!("{NS}/pods/web:+80/proxy"), ProxyNamedOrMissing),
            (format!("{NS}/pods/web:８０/proxy"), ProxyNamedOrMissing),
            (
                "/api/v1/nodes/n1/proxy/portForward/dev/web".to_string(),
                NodeProxy,
            ),
            (
                "/api/v1/nodes/n1:10250/proxy/exec/dev/web/c".to_string(),
                NodeProxy,
            ),
            ("/api/v1/nodes/n1/prox%79".to_string(), NodeProxy),
            // Not port-bearing.
            (format!("{NS}/pods/web"), NoPort),
            (format!("{NS}/pods/web/log"), NoPort),
            (format!("{NS}/pods/web/exec"), NoPort),
            (format!("{NS}/configmaps/web:80/proxy"), NoPort),
            ("/api/v1/nodes/n1".to_string(), NoPort),
            (
                "/apis/apps/v1/namespaces/dev/deployments/web".to_string(),
                NoPort,
            ),
        ];
        for (path, expected) in cases {
            assert_eq!(port_target(&path), expected, "{path}");
        }
    }

    #[test]
    fn query_ports_are_checked_in_both_spellings() {
        let policy = only(&[8080, 9090]);
        assert_eq!(
            check_query_ports("ports=8080&ports=9090", &policy),
            Ok(vec![8080, 9090])
        );
        assert!(check_query_ports("ports=8080,9090", &policy).is_ok());
        assert_eq!(check_query_ports("timeout=5s", &policy), Ok(vec![]));
        assert!(check_query_ports("ports=8080&ports=22", &policy).is_err());
        assert!(check_query_ports("port=22", &policy).is_err());
        assert!(check_query_ports("port%73=22", &policy).is_err());
        assert!(check_query_ports("ports=8080%2C22", &policy).is_err());
        assert!(check_query_ports("ports=%2B22", &policy).is_err());
        assert!(check_query_ports("ports=", &policy).is_err());
    }

    #[test]
    fn the_accepted_protocol_selects_the_filter() {
        let policy = only(&[8080]);
        assert!(matches!(
            filter_for_accepted_protocol(Some("SPDY/3.1+portforward.k8s.io"), None, &policy),
            Ok(Some(_))
        ));
        assert!(matches!(
            filter_for_accepted_protocol(Some("v4.channel.k8s.io"), None, &policy),
            Ok(None)
        ));
        assert!(matches!(
            filter_for_accepted_protocol(None, None, &policy),
            Ok(None)
        ));
        assert!(filter_for_accepted_protocol(Some("v5.channel.k8s.io"), None, &policy).is_err());
        assert!(
            filter_for_accepted_protocol(
                Some("SPDY/3.1+portforward.k8s.io"),
                Some("permessage-deflate"),
                &policy
            )
            .is_err()
        );
    }

    #[test]
    fn allowed_ports_pass_and_others_close_the_session() {
        let mut compressor = HeaderCompressor::new();
        let mut filter = PortForwardFilter::new(only(&[8080]));

        let allowed = syn_stream(&mut compressor, 1, "8080");
        assert!(filter.inspect(&binary(&allowed)).is_ok());
        assert!(
            filter
                .inspect(&binary(&data_frame(1, b"GET / HTTP/1.1\r\n")))
                .is_ok()
        );

        // Same compression context: the second block depends on the first.
        let denied = syn_stream(&mut compressor, 3, "22");
        let err = filter.inspect(&binary(&denied)).unwrap_err();
        assert!(err.contains("22"), "{err}");
        assert_eq!(filter.refused_port(), Some(22));
    }

    /// Each opened port is reported once, though kubectl names it on both its
    /// data and error streams.
    #[test]
    fn opened_ports_are_reported_once() {
        let mut compressor = HeaderCompressor::new();
        let mut filter = PortForwardFilter::new(only(&[8080, 9090]));
        for (stream_id, port) in [(1, "8080"), (3, "8080"), (5, "9090")] {
            filter
                .inspect(&binary(&syn_stream(&mut compressor, stream_id, port)))
                .unwrap();
        }
        assert_eq!(filter.take_opened(), vec![8080, 9090]);
        assert!(filter.take_opened().is_empty());
    }

    #[test]
    fn frames_split_at_every_byte_are_still_inspected() {
        let mut compressor = HeaderCompressor::new();
        let mut stream = binary(&syn_stream(&mut compressor, 1, "8080"));
        // A SPDY frame split over two websocket frames, then a denied port.
        let denied = syn_stream(&mut compressor, 3, "22");
        let (head, tail) = denied.split_at(5);
        stream.extend(ws_frame(0x02, head));
        stream.extend(ws_frame(0x80, tail));

        let mut filter = PortForwardFilter::new(only(&[8080]));
        let result = stream.chunks(1).try_for_each(|byte| filter.inspect(byte));
        assert!(result.unwrap_err().contains("22"));
    }

    #[test]
    fn several_port_values_must_all_be_allowed() {
        let mut compressor = HeaderCompressor::new();
        let mut body = 1u32.to_be_bytes().to_vec();
        body.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        body.extend(compressor.compress(&[("Port", "8080\u{0}22")]));

        let mut filter = PortForwardFilter::new(only(&[8080]));
        assert!(
            filter
                .inspect(&binary(&control_frame(SPDY_SYN_STREAM, &body)))
                .is_err()
        );
    }

    #[test]
    fn headers_frames_keep_the_compression_context_in_step() {
        let mut compressor = HeaderCompressor::new();
        let allowed = syn_stream(&mut compressor, 1, "8080");
        let mut headers = 1u32.to_be_bytes().to_vec();
        headers.extend(compressor.compress(&[("x-anything", "value")]));

        let mut filter = PortForwardFilter::new(only(&[8080]));
        assert!(filter.inspect(&binary(&allowed)).is_ok());
        assert!(
            filter
                .inspect(&binary(&control_frame(SPDY_HEADERS, &headers)))
                .is_ok()
        );
        let denied = syn_stream(&mut compressor, 3, "22");
        assert!(filter.inspect(&binary(&denied)).is_err());
    }

    #[test]
    fn unreadable_traffic_closes_the_session() {
        let policy = only(&[8080]);
        // Compressed (RSV1) frame.
        assert!(
            PortForwardFilter::new(policy.clone())
                .inspect(&ws_frame(0xc2, b"x"))
                .is_err()
        );
        // Text frame.
        assert!(
            PortForwardFilter::new(policy.clone())
                .inspect(&ws_frame(0x81, b"x"))
                .is_err()
        );
        // Unmasked frame.
        assert!(
            PortForwardFilter::new(policy.clone())
                .inspect(&[0x82, 0x01, 0x00])
                .is_err()
        );
        // Another SPDY version.
        let frame = [0x80, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x0a];
        assert!(
            PortForwardFilter::new(policy.clone())
                .inspect(&binary(&frame))
                .is_err()
        );
        // A header block that is not zlib.
        let garbage = control_frame(SPDY_SYN_STREAM, &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 2, 3]);
        let mut filter = PortForwardFilter::new(policy);
        assert!(filter.inspect(&binary(&garbage)).is_err());
        // Unreadable, not refused: the audit has no port to report.
        assert_eq!(filter.refused_port(), None);
    }

    #[test]
    fn control_frames_and_other_spdy_frames_pass() {
        let mut filter = PortForwardFilter::new(only(&[8080]));
        // Websocket ping.
        assert!(filter.inspect(&ws_frame(0x89, b"hi")).is_ok());
        // SPDY PING (type 6) and SETTINGS (type 4).
        assert!(
            filter
                .inspect(&binary(&control_frame(6, &[0, 0, 0, 1])))
                .is_ok()
        );
        assert!(
            filter
                .inspect(&binary(&control_frame(4, &[0, 0, 0, 0])))
                .is_ok()
        );
    }
}
