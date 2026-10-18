//! Client-to-server websocket frames (RFC 6455 §5.2), shared by the filters
//! that read a tunnelled session: the port-forward filter today, the exec
//! channel filter of RFC 0004.

/// Client-to-server websocket frame decoder (RFC 6455 §5.2): yields the
/// unmasked payload of data frames, skips control frames.
#[derive(Default)]
pub(super) struct WebsocketDecoder {
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
    pub(super) fn decode(
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
