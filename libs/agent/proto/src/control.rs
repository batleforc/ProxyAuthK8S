//! Control channel messages (RFC 0012 §6.2): protobuf through `prost`, derived
//! by hand (no `protoc`), each frame a big-endian `u32` length then the body.
//! A frame longer than [`MAX_CONTROL_BYTES`] is refused before it is read.

use futures_util::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use prost::Message as _;

/// Bound on one control message (RFC 0012 §7.3).
pub const MAX_CONTROL_BYTES: usize = 64 * 1024;

/// First message of a tunnel, from the proxy replica.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Hello {
    #[prost(string, tag = "1")]
    pub replica_id: String,
    /// Live proxy replicas: the agent dials until it holds one tunnel to each.
    #[prost(uint32, tag = "2")]
    pub replicas_expected: u32,
    #[prost(uint32, tag = "3")]
    pub protocol: u32,
}

/// The agent's answer to `Hello`. Informational: nothing here is trusted.
#[derive(Clone, PartialEq, prost::Message)]
pub struct AgentInfo {
    #[prost(string, tag = "1")]
    pub version: String,
    /// SHA-256 of the remote cluster's `kube-root-ca.crt`, for a warning only
    /// (T4: the apiserver CA comes from `spec.cert`).
    #[prost(string, tag = "2")]
    pub ca_fingerprint: String,
    /// The range of protocols the agent speaks (RFC 0013); both `0` from an
    /// agent older than these fields.
    #[prost(uint32, tag = "3")]
    pub protocol_min: u32,
    #[prost(uint32, tag = "4")]
    pub protocol_max: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct RenewRequest {
    /// PEM certificate signing request; only its public key is used.
    #[prost(bytes = "vec", tag = "1")]
    pub csr: Vec<u8>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct RenewResponse {
    /// PEM certificate chain issued by the agent CA.
    #[prost(bytes = "vec", tag = "1")]
    pub cert: Vec<u8>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Ping {
    #[prost(uint64, tag = "1")]
    pub nonce: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Pong {
    #[prost(uint64, tag = "1")]
    pub nonce: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Close {
    #[prost(string, tag = "1")]
    pub reason: String,
    /// The sender's protocol range (RFC 0013): both `0` from a 0.4.0 peer.
    #[prost(uint32, tag = "2")]
    pub protocol_min: u32,
    #[prost(uint32, tag = "3")]
    pub protocol_max: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Control {
    #[prost(oneof = "Kind", tags = "1, 2, 3, 4, 5, 6, 7")]
    pub kind: Option<Kind>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub enum Kind {
    #[prost(message, tag = "1")]
    Hello(Hello),
    #[prost(message, tag = "2")]
    AgentInfo(AgentInfo),
    #[prost(message, tag = "3")]
    RenewRequest(RenewRequest),
    #[prost(message, tag = "4")]
    RenewResponse(RenewResponse),
    #[prost(message, tag = "5")]
    Ping(Ping),
    #[prost(message, tag = "6")]
    Pong(Pong),
    #[prost(message, tag = "7")]
    Close(Close),
}

#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("control channel: {0}")]
    Io(#[from] std::io::Error),
    #[error("control message of {0} bytes is over the {MAX_CONTROL_BYTES}-byte bound")]
    TooLarge(usize),
    #[error("undecodable control message: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("control message without a kind")]
    Empty,
}

/// One frame: the length prefix and the body.
pub fn encode(kind: Kind) -> Result<Vec<u8>, ControlError> {
    let body = Control { kind: Some(kind) }.encode_to_vec();
    let len = u32::try_from(body.len())
        .ok()
        .filter(|_| body.len() <= MAX_CONTROL_BYTES)
        .ok_or(ControlError::TooLarge(body.len()))?;
    let mut frame = Vec::with_capacity(body.len().saturating_add(4));
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Decode a body read after its length prefix.
pub fn decode(body: &[u8]) -> Result<Kind, ControlError> {
    if body.len() > MAX_CONTROL_BYTES {
        return Err(ControlError::TooLarge(body.len()));
    }
    Control::decode(body)?.kind.ok_or(ControlError::Empty)
}

pub async fn write<W: AsyncWrite + Unpin>(writer: &mut W, kind: Kind) -> Result<(), ControlError> {
    writer.write_all(&encode(kind)?).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Kind, ControlError> {
    let mut prefix = [0u8; 4];
    reader.read_exact(&mut prefix).await?;
    let len = usize::try_from(u32::from_be_bytes(prefix)).unwrap_or(usize::MAX);
    if len > MAX_CONTROL_BYTES {
        return Err(ControlError::TooLarge(len));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    decode(&body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::io::Cursor;

    #[tokio::test]
    async fn every_kind_survives_a_round_trip() {
        let kinds = vec![
            Kind::Hello(Hello {
                replica_id: "proxy-0".into(),
                replicas_expected: 2,
                protocol: crate::PROTOCOL_VERSION,
            }),
            Kind::AgentInfo(AgentInfo {
                version: "0.4.0".into(),
                ca_fingerprint: "ab".repeat(32),
                protocol_min: crate::PROTOCOL_MIN,
                protocol_max: crate::PROTOCOL_VERSION,
            }),
            Kind::RenewRequest(RenewRequest {
                csr: b"-----BEGIN CERTIFICATE REQUEST-----".to_vec(),
            }),
            Kind::RenewResponse(RenewResponse {
                cert: b"-----BEGIN CERTIFICATE-----".to_vec(),
            }),
            Kind::Ping(Ping { nonce: 7 }),
            Kind::Pong(Pong { nonce: 7 }),
            Kind::Close(Close {
                reason: "protocol".into(),
                protocol_min: 1,
                protocol_max: 2,
            }),
        ];
        let mut wire = Vec::new();
        for kind in &kinds {
            write(&mut wire, kind.clone()).await.unwrap();
        }
        let mut reader = Cursor::new(wire);
        for kind in kinds {
            assert_eq!(read(&mut reader).await.unwrap(), kind);
        }
    }

    #[tokio::test]
    async fn an_oversized_length_is_refused_before_the_body_is_read() {
        // Only the prefix is there: a reader that tried to read the body would
        // fail on EOF instead.
        let prefix = u32::try_from(MAX_CONTROL_BYTES + 1).unwrap().to_be_bytes();
        let err = read(&mut Cursor::new(prefix.to_vec())).await.unwrap_err();
        assert!(matches!(err, ControlError::TooLarge(_)), "{err}");
    }

    #[test]
    fn an_oversized_message_is_never_encoded() {
        let csr = vec![0u8; MAX_CONTROL_BYTES];
        let err = encode(Kind::RenewRequest(RenewRequest { csr })).unwrap_err();
        assert!(matches!(err, ControlError::TooLarge(_)), "{err}");
    }

    /// An agent 0.4.0 sends tags 1 and 2 only: they read as `0`, protocol 1.
    #[test]
    fn an_agent_info_without_its_protocol_range_reads_as_zero() {
        #[derive(Clone, PartialEq, prost::Message)]
        struct AgentInfoV1 {
            #[prost(string, tag = "1")]
            version: String,
            #[prost(string, tag = "2")]
            ca_fingerprint: String,
        }
        let old = AgentInfoV1 {
            version: "0.4.0".into(),
            ca_fingerprint: String::new(),
        }
        .encode_to_vec();
        let info = AgentInfo::decode(old.as_slice()).unwrap();
        assert_eq!((info.protocol_min, info.protocol_max), (0, 0));
        assert_eq!(
            crate::negotiate((info.protocol_min, info.protocol_max), (1, 1)),
            Some(1)
        );
    }

    #[test]
    fn a_message_without_a_kind_is_refused() {
        let body = Control { kind: None }.encode_to_vec();
        assert!(matches!(decode(&body), Err(ControlError::Empty)));
    }
}
