//! Port filtering for `pods/{name}/portforward` sessions.
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

/// Whether `path` (the upstream path, without query) is a pod port-forward.
///
/// Segments are decoded the way the apiserver decodes them, so an encoded
/// `portforwar%64` is still recognised.
pub(super) fn is_port_forward_path(path: &str) -> bool {
    let segments: Vec<String> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(percent_decode_once)
        .collect();
    matches!(
        segments.as_slice(),
        [.., pods, _, portforward] if pods == "pods" && portforward == "portforward"
    )
}

/// Check the ports a port-forward asks for in its query string.
///
/// The apiserver reads `ports`, the kubelet `port`; both are checked, each
/// value may be a comma-separated list, as the kubelet accepts.
pub(super) fn check_query_ports(query: &str, policy: &PortPolicy) -> Result<(), String> {
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode_once(&key.replace('+', " "));
        if !key.eq_ignore_ascii_case("ports") && !key.eq_ignore_ascii_case("port") {
            continue;
        }
        let value = percent_decode_once(&value.replace('+', " "));
        for port in value.split(',') {
            check_port(port, policy)?;
        }
    }
    Ok(())
}

fn check_port(value: &str, policy: &PortPolicy) -> Result<(), String> {
    let port = Some(value)
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| v.parse::<u16>().ok())
        .ok_or_else(|| format!("port \"{value}\" is not a valid port"))?;
    if policy.allows(port) {
        Ok(())
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
}

/// Client-to-server websocket frame decoder (RFC 6455 §5.2): yields the
/// unmasked payload of data frames, skips control frames.
#[derive(Default)]
struct WebsocketDecoder {
    header: Vec<u8>,
    payload: Option<FramePayload>,
    /// A fragmented data message is in progress.
    in_message: bool,
}

struct FramePayload {
    remaining: u64,
    mask: [u8; 4],
    offset: usize,
    is_data: bool,
}

impl WebsocketDecoder {
    fn decode(
        &mut self,
        mut input: &[u8],
        mut on_data: impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        while !input.is_empty() {
            let Some(payload) = self.payload.as_mut() else {
                let used = self.read_header(input)?;
                input = &input[used..];
                continue;
            };

            let take = usize::try_from(payload.remaining)
                .unwrap_or(usize::MAX)
                .min(input.len());
            if payload.is_data {
                let unmasked: Vec<u8> = input[..take]
                    .iter()
                    .enumerate()
                    .map(|(i, byte)| byte ^ payload.mask[(payload.offset + i) % 4])
                    .collect();
                on_data(&unmasked)?;
            }
            payload.offset = (payload.offset + take) % 4;
            payload.remaining -= take as u64;
            input = &input[take..];
            if payload.remaining == 0 {
                self.payload = None;
            }
        }
        Ok(())
    }

    /// Consume header bytes from `input`; returns how many were used.
    fn read_header(&mut self, input: &[u8]) -> Result<usize, String> {
        let mut needed = 2;
        if self.header.len() >= 2 {
            needed = header_len(self.header[1]);
        }
        let mut used = 0;
        while self.header.len() < needed && used < input.len() {
            self.header.push(input[used]);
            used += 1;
            if self.header.len() == 2 {
                // Refuse as soon as the flags are known, not once the rest of
                // a header that may never come has arrived.
                if self.header[0] & 0x70 != 0 {
                    return Err("websocket frame uses an extension".into());
                }
                if self.header[1] & 0x80 == 0 {
                    return Err("unmasked client websocket frame".into());
                }
                needed = header_len(self.header[1]);
            }
        }
        if self.header.len() < needed {
            return Ok(used);
        }
        let first = self.header[0];
        let second = self.header[1];
        let fin = first & 0x80 != 0;
        let opcode = first & 0x0f;

        let (length, mask_at) = match second & 0x7f {
            126 => (
                u64::from(u16::from_be_bytes([self.header[2], self.header[3]])),
                4,
            ),
            127 => (
                u64::from_be_bytes(self.header[2..10].try_into().expect("8 header bytes")),
                10,
            ),
            short => (u64::from(short), 2),
        };
        let mask: [u8; 4] = self.header[mask_at..mask_at + 4]
            .try_into()
            .expect("4 mask bytes");

        let is_data = match opcode {
            // Continuation of a fragmented message.
            0x0 if self.in_message => true,
            // A binary message; a new one cannot start inside another.
            0x2 if !self.in_message => true,
            0x8..=0xa if fin && length <= 125 => false,
            _ => return Err(format!("unexpected websocket frame (opcode {opcode:#x})")),
        };
        if is_data {
            self.in_message = !fin;
        }

        self.header.clear();
        if length > 0 {
            self.payload = Some(FramePayload {
                remaining: length,
                mask,
                offset: 0,
                is_data,
            });
        }
        Ok(used)
    }
}

/// Total header length (with the mask key) once the second byte is known.
fn header_len(second: u8) -> usize {
    let extended = match second & 0x7f {
        126 => 2,
        127 => 8,
        _ => 0,
    };
    2 + extended + 4
}

/// SPDY/3 frame reader over the tunnelled byte stream (draft 3, §2.2).
struct SpdyInspector {
    policy: PortPolicy,
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
            header: Vec::with_capacity(8),
            state: SpdyState::Header,
            decompress: Decompress::new(true),
        }
    }

    fn feed(&mut self, mut input: &[u8]) -> Result<(), String> {
        while !input.is_empty() {
            match &mut self.state {
                SpdyState::Header => {
                    let take = (8 - self.header.len()).min(input.len());
                    self.header.extend_from_slice(&input[..take]);
                    input = &input[take..];
                    if self.header.len() == 8 {
                        self.state = self.start_frame()?;
                        self.header.clear();
                    }
                }
                SpdyState::Skip(remaining) => {
                    let take = (*remaining).min(input.len());
                    *remaining -= take;
                    input = &input[take..];
                    if *remaining == 0 {
                        self.state = SpdyState::Header;
                    }
                }
                SpdyState::Control { kind, length, body } => {
                    let take = (*length - body.len()).min(input.len());
                    body.extend_from_slice(&input[..take]);
                    input = &input[take..];
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
        let h = &self.header;
        let length = usize::from(h[5]) << 16 | usize::from(h[6]) << 8 | usize::from(h[7]);
        let is_control = h[0] & 0x80 != 0;
        if !is_control {
            return Ok(skip(length));
        }

        let version = u16::from_be_bytes([h[0], h[1]]) & 0x7fff;
        if version != SPDY_VERSION {
            return Err(format!("unsupported SPDY version {version}"));
        }
        let kind = u16::from_be_bytes([h[2], h[3]]);
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
            check_port(value, &self.policy)?;
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
                &input[consumed..],
                &mut output,
                FlushDecompress::Sync,
            );
            consumed += usize::try_from(self.decompress.total_in() - in_before)
                .map_err(|_| "SPDY header block too large".to_string())?;

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
            let mut out = Vec::with_capacity(block.len() + 64);
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
    fn query_ports_are_checked_in_both_spellings() {
        let policy = only(&[8080, 9090]);
        assert!(check_query_ports("ports=8080&ports=9090", &policy).is_ok());
        assert!(check_query_ports("ports=8080,9090", &policy).is_ok());
        assert!(check_query_ports("timeout=5s", &policy).is_ok());
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
        assert!(
            PortForwardFilter::new(policy)
                .inspect(&binary(&garbage))
                .is_err()
        );
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
