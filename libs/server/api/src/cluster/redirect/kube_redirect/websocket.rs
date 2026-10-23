//! Websocket frames (RFC 6455 §5.2), shared by the filters that read a
//! tunnelled session: the port-forward filter and the exec channel filter of
//! RFC 0004.
//!
//! Only frame headers are kept: a payload is streamed through, never
//! buffered, so a session costs a few bytes whatever its frames weigh.

/// Largest frame payload accepted (RFC 0004, T11): the kubelet sends 32 KiB,
/// kubectl less; anything larger is refused rather than followed.
pub(super) const MAX_FRAME_BYTES: u64 = 1024 * 1024;

/// Websocket frame decoder: yields the payload of data frames, skips control
/// frames. Client frames must be masked (RFC 6455 §5.3), server frames must
/// not; the decoder unmasks the former.
pub(super) struct WebsocketDecoder {
    /// Client-to-server frames (masked) rather than server-to-client ones.
    client: bool,
    header: Vec<u8>,
    payload: Option<FramePayload>,
    /// A fragmented data message is in progress.
    in_message: bool,
    /// The next data bytes start a new message.
    message_start: bool,
}

impl Default for WebsocketDecoder {
    fn default() -> Self {
        Self::client()
    }
}

struct FramePayload {
    remaining: u64,
    mask: Option<[u8; 4]>,
    offset: usize,
    is_data: bool,
}

impl WebsocketDecoder {
    /// A decoder for the frames a client sends.
    pub(super) fn client() -> Self {
        Self {
            client: true,
            header: Vec::new(),
            payload: None,
            in_message: false,
            message_start: false,
        }
    }

    /// A decoder for the frames a server sends.
    pub(super) fn server() -> Self {
        Self {
            client: false,
            ..Self::client()
        }
    }

    /// Feed `input`; `on_data` gets every chunk of data payload.
    pub(super) fn decode(
        &mut self,
        input: &[u8],
        mut on_data: impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        self.decode_messages(input, |data, _| on_data(data))
    }

    /// Feed `input`; `on_data` gets every chunk of data payload, and whether
    /// it is the first chunk of a message (its first byte is then the
    /// message's first byte, a channel number in the k8s channel protocols).
    pub(super) fn decode_messages(
        &mut self,
        mut input: &[u8],
        mut on_data: impl FnMut(&[u8], bool) -> Result<(), String>,
    ) -> Result<(), String> {
        while !input.is_empty() {
            let Some(payload) = self.payload.as_mut() else {
                let used = self.read_header(input)?;
                input = input.get(used..).ok_or_else(overrun)?;
                continue;
            };

            let take = usize::try_from(payload.remaining)
                .unwrap_or(usize::MAX)
                .min(input.len());
            let (chunk, rest) = input.split_at_checked(take).ok_or_else(overrun)?;
            if payload.is_data {
                let data: Vec<u8> = match payload.mask {
                    Some(mask) => chunk
                        .iter()
                        .zip(mask.iter().cycle().skip(payload.offset))
                        .map(|(byte, key)| byte ^ key)
                        .collect(),
                    None => chunk.to_vec(),
                };
                let start = std::mem::take(&mut self.message_start);
                on_data(&data, start)?;
            }
            // Both terms are below 4: no overflow.
            payload.offset = payload.offset.wrapping_add(take % 4) % 4;
            // `take` is at most `remaining`.
            payload.remaining = payload
                .remaining
                .saturating_sub(u64::try_from(take).unwrap_or(u64::MAX));
            input = rest;
            if payload.remaining == 0 {
                self.payload = None;
            }
        }
        Ok(())
    }

    /// Consume header bytes from `input`; returns how many were used.
    fn read_header(&mut self, input: &[u8]) -> Result<usize, String> {
        let mut needed = self.header.get(1).map_or(2, |&second| header_len(second));
        let mut used: usize = 0;
        while self.header.len() < needed {
            let Some(&byte) = input.get(used) else {
                break;
            };
            self.header.push(byte);
            used = used.saturating_add(1);
            if let &[first, second] = self.header.as_slice() {
                // Refuse as soon as the flags are known, not once the rest of
                // a header that may never come has arrived.
                if first & 0x70 != 0 {
                    return Err("websocket frame uses an extension".into());
                }
                let masked = second & 0x80 != 0;
                if self.client && !masked {
                    return Err("unmasked client websocket frame".into());
                }
                if !self.client && masked {
                    return Err("masked server websocket frame".into());
                }
                needed = header_len(second);
            }
        }
        if self.header.len() < needed {
            return Ok(used);
        }
        let &[first, second, ..] = self.header.as_slice() else {
            return Err(overrun());
        };
        let fin = first & 0x80 != 0;
        let opcode = first & 0x0f;

        let (length, mask_at) = match second & 0x7f {
            126 => (
                u64::from(u16::from_be_bytes(header_bytes(&self.header, 2)?)),
                4,
            ),
            127 => (u64::from_be_bytes(header_bytes(&self.header, 2)?), 10),
            short => (u64::from(short), 2),
        };
        if length > MAX_FRAME_BYTES {
            return Err(format!("websocket frame of {length} bytes is too large"));
        }
        let mask = (second & 0x80 != 0)
            .then(|| header_bytes::<4>(&self.header, mask_at))
            .transpose()?;

        let is_data = match opcode {
            // Continuation of a fragmented message.
            0x0 if self.in_message => true,
            // A binary message; a new one cannot start inside another.
            0x2 if !self.in_message => {
                self.message_start = true;
                true
            }
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

/// Total header length (with the mask key, when the frame has one) once the
/// second byte is known.
fn header_len(second: u8) -> usize {
    let extended = match second & 0x7f {
        126 => 2,
        127 => 8,
        _ => 0,
    };
    let mask = if second & 0x80 != 0 { 4 } else { 0 };
    2_usize.saturating_add(extended).saturating_add(mask)
}

/// The `N` header bytes at `at`; `read_header` only asks for bytes that
/// `header_len` counted in, so a miss is a decoder bug, refused.
fn header_bytes<const N: usize>(header: &[u8], at: usize) -> Result<[u8; N], String> {
    header
        .get(at..)
        .and_then(<[u8]>::first_chunk::<N>)
        .copied()
        .ok_or_else(overrun)
}

/// The refusal of an offset past the bytes at hand (a decoder bug).
fn overrun() -> String {
    "websocket decoder overrun".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A masked client frame of `payload` with the given opcode and FIN bit.
    fn client_frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mask = [1, 2, 3, 4];
        let mut frame = vec![
            if fin { 0x80 } else { 0 } | opcode,
            0x80 | u8::try_from(payload.len()).expect("short test payload"),
        ];
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        frame
    }

    fn server_frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![
            0x82,
            u8::try_from(payload.len()).expect("short test payload"),
        ];
        frame.extend_from_slice(payload);
        frame
    }

    fn messages(
        decoder: &mut WebsocketDecoder,
        input: &[u8],
    ) -> Result<Vec<(Vec<u8>, bool)>, String> {
        let mut seen = Vec::new();
        decoder.decode_messages(input, |data, start| {
            seen.push((data.to_vec(), start));
            Ok(())
        })?;
        Ok(seen)
    }

    #[test]
    fn a_fragmented_message_starts_once() {
        let mut decoder = WebsocketDecoder::client();
        let mut input = client_frame(false, 0x2, &[0, b'h']);
        input.extend(client_frame(true, 0x0, b"i"));
        let seen = messages(&mut decoder, &input).unwrap();
        assert_eq!(seen, vec![(vec![0, b'h'], true), (b"i".to_vec(), false)]);
    }

    #[test]
    fn server_frames_are_unmasked_and_client_frames_masked() {
        let mut server = WebsocketDecoder::server();
        assert_eq!(
            messages(&mut server, &server_frame(&[1, b'o', b'k'])).unwrap(),
            vec![(vec![1, b'o', b'k'], true)]
        );
        // A masked frame from the server, or an unmasked one from a client.
        assert!(
            messages(
                &mut WebsocketDecoder::server(),
                &client_frame(true, 0x2, b"x")
            )
            .is_err()
        );
        assert!(messages(&mut WebsocketDecoder::client(), &server_frame(b"x")).is_err());
    }

    /// T11: a frame larger than 1 MiB is refused from its header.
    #[test]
    fn an_oversized_frame_is_refused() {
        let mut header = vec![0x82, 0x80 | 127];
        header.extend_from_slice(&(MAX_FRAME_BYTES + 1).to_be_bytes());
        header.extend_from_slice(&[0, 0, 0, 0]);
        assert!(messages(&mut WebsocketDecoder::client(), &header).is_err());
    }
}
