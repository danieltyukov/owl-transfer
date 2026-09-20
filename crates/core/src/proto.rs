//! The wire format: `u32` big-endian payload length, then a payload whose
//! first byte is the frame type. Control frames carry JSON; block frames
//! carry raw file bytes with a fixed header.

use anyhow::{bail, Context, Result};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::config::DeviceKind;
use crate::index::Entry;

pub const PROTOCOL_VERSION: u32 = 1;
pub const BLOCK_SIZE: u32 = 256 * 1024;
/// Larger than any control frame the engine builds and than a block with
/// its header; anything bigger is a broken or hostile peer.
pub const MAX_FRAME: usize = 4 * 1024 * 1024;
/// Entries per `Index` frame; a full index is sent as several.
pub const INDEX_CHUNK: usize = 1000;

const TYPE_CONTROL: u8 = 0;
const TYPE_BLOCK: u8 = 1;
const BLOCK_HEADER: usize = 1 + 8 + 1;

pub const BLOCK_OK: u8 = 0;
pub const BLOCK_UNAVAILABLE: u8 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    Hello {
        id: String,
        name: String,
        kind: DeviceKind,
        version: u32,
    },
    PairRequest {
        id: String,
        name: String,
        kind: DeviceKind,
    },
    PairChallenge {
        /// 32 hex characters.
        nonce: String,
    },
    PairAccept,
    PairReject {
        reason: String,
    },
    Index {
        entries: Vec<Entry>,
    },
    IndexUpdate {
        entries: Vec<Entry>,
    },
    Request {
        req_id: u64,
        path: String,
        hash: String,
        offset: u64,
        len: u32,
    },
    Ping,
    Pong,
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Control(Control),
    /// `[1][req_id: u64 BE][status: u8][bytes...]`
    Block {
        req_id: u64,
        status: u8,
        data: Bytes,
    },
}

/// Encodes a frame with its length prefix.
pub fn encode(frame: &Frame) -> Bytes {
    match frame {
        Frame::Control(control) => {
            let json = serde_json::to_vec(control).expect("control frames serialise");
            let mut buf = BytesMut::with_capacity(4 + 1 + json.len());
            buf.put_u32((1 + json.len()) as u32);
            buf.put_u8(TYPE_CONTROL);
            buf.put_slice(&json);
            buf.freeze()
        }
        Frame::Block {
            req_id,
            status,
            data,
        } => {
            let mut buf = BytesMut::with_capacity(4 + BLOCK_HEADER + data.len());
            buf.put_u32((BLOCK_HEADER + data.len()) as u32);
            buf.put_u8(TYPE_BLOCK);
            buf.put_u64(*req_id);
            buf.put_u8(*status);
            buf.put_slice(data);
            buf.freeze()
        }
    }
}

/// Decodes a payload (without its length prefix).
pub fn decode(mut payload: Bytes) -> Result<Frame> {
    if payload.is_empty() {
        bail!("empty frame");
    }
    match payload.get_u8() {
        TYPE_CONTROL => {
            let control: Control =
                serde_json::from_slice(&payload).context("malformed control frame")?;
            Ok(Frame::Control(control))
        }
        TYPE_BLOCK => {
            if payload.len() < BLOCK_HEADER - 1 {
                bail!("block frame too short");
            }
            let req_id = payload.get_u64();
            let status = payload.get_u8();
            Ok(Frame::Block {
                req_id,
                status,
                data: payload,
            })
        }
        other => bail!("unknown frame type {other}"),
    }
}

pub struct FrameReader<R> {
    reader: R,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(reader: R) -> Self {
        FrameReader { reader }
    }

    /// The next frame, or `None` at a clean end of stream. An end of stream
    /// in the middle of a frame is an error.
    pub async fn next(&mut self) -> Result<Option<Frame>> {
        let mut len_buf = [0u8; 4];
        match self.reader.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e).context("reading frame length"),
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        if len == 0 || len > MAX_FRAME {
            bail!("frame of {len} bytes is outside the allowed 1..={MAX_FRAME}");
        }
        let mut payload = vec![0u8; len];
        self.reader
            .read_exact(&mut payload)
            .await
            .context("reading frame payload")?;
        decode(Bytes::from(payload)).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn roundtrip_control_and_block() {
        let (mut client, server) = tokio::io::duplex(1 << 20);
        let frames = vec![
            Frame::Control(Control::Hello {
                id: "abc".into(),
                name: "Desk".into(),
                kind: DeviceKind::Desktop,
                version: PROTOCOL_VERSION,
            }),
            Frame::Control(Control::Ping),
            Frame::Block {
                req_id: 7,
                status: BLOCK_OK,
                data: Bytes::from_static(b"hello block"),
            },
            Frame::Control(Control::Request {
                req_id: 8,
                path: "a/b".into(),
                hash: "h".into(),
                offset: 262_144,
                len: BLOCK_SIZE,
            }),
        ];
        for f in &frames {
            client.write_all(&encode(f)).await.unwrap();
        }
        drop(client);
        let mut reader = FrameReader::new(server);
        for f in &frames {
            assert_eq!(reader.next().await.unwrap().as_ref(), Some(f));
        }
        assert_eq!(reader.next().await.unwrap(), None);
    }

    #[tokio::test]
    async fn oversized_frame_is_an_error() {
        let (mut client, server) = tokio::io::duplex(64);
        let len = (MAX_FRAME as u32 + 1).to_be_bytes();
        tokio::spawn(async move {
            let _ = client.write_all(&len).await;
        });
        let mut reader = FrameReader::new(server);
        assert!(reader.next().await.is_err());
    }

    #[tokio::test]
    async fn truncated_frame_is_an_error() {
        let (mut client, server) = tokio::io::duplex(64);
        client.write_all(&10u32.to_be_bytes()).await.unwrap();
        client.write_all(&[0, 1, 2]).await.unwrap();
        drop(client);
        let mut reader = FrameReader::new(server);
        assert!(reader.next().await.is_err());
    }

    #[test]
    fn control_json_uses_snake_case_tags() {
        let json = serde_json::to_string(&Control::PairAccept).unwrap();
        assert_eq!(json, r#"{"type":"pair_accept"}"#);
        let json = serde_json::to_string(&Control::PairChallenge {
            nonce: "00".repeat(16),
        })
        .unwrap();
        assert!(json.starts_with(r#"{"type":"pair_challenge","nonce":"#));
    }
}
