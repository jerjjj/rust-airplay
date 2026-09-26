#![allow(missing_docs)]
// SPDX-License-Identifier: GPL-3.0-or-later
//! AirPlay mirror TCP framing and NAL conversion.

use crate::wire::ParseError;

pub const MIRROR_HEADER_LEN: usize = 128;
pub const DEFAULT_MAX_PAYLOAD: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorPacket {
    pub payload_len: usize,
    pub packet_type: [u8; 2],
    pub option: [u8; 2],
    pub timestamp: u64,
    pub payload: Vec<u8>,
}

/// Incremental decoder for the 128-byte mirror TCP framing.
///
/// A payload that exceeds the configured maximum is a fatal framing error: the
/// next frame boundary is unknown, so later [`MirrorDecoder::push`] calls keep
/// reporting the same error until [`MirrorDecoder::reset`] discards the stream.
#[derive(Debug)]
pub struct MirrorDecoder {
    buffer: Vec<u8>,
    start: usize,
    maximum_payload: usize,
    poisoned: Option<ParseError>,
}

impl Default for MirrorDecoder {
    fn default() -> Self {
        Self::new()
    }
}
impl MirrorDecoder {
    pub fn new() -> Self {
        Self::with_maximum_payload(DEFAULT_MAX_PAYLOAD)
    }
    pub fn with_maximum_payload(maximum_payload: usize) -> Self {
        Self {
            buffer: Vec::new(),
            start: 0,
            maximum_payload,
            poisoned: None,
        }
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<MirrorPacket>, ParseError> {
        if let Some(error) = &self.poisoned {
            return Err(error.clone());
        }
        self.buffer.extend_from_slice(bytes);
        let mut packets = Vec::new();
        loop {
            let available = self.buffer.len() - self.start;
            if available < MIRROR_HEADER_LEN {
                break;
            }
            let (payload_len, packet_type, option, timestamp) = {
                let header = &self.buffer[self.start..self.start + MIRROR_HEADER_LEN];
                (
                    u32::from_le_bytes(
                        header[..4]
                            .try_into()
                            .map_err(|_| ParseError::Invalid("payload length"))?,
                    ) as usize,
                    [header[4], header[5]],
                    [header[6], header[7]],
                    u64::from_le_bytes(
                        header[8..16]
                            .try_into()
                            .map_err(|_| ParseError::Invalid("timestamp"))?,
                    ),
                )
            };
            if payload_len > self.maximum_payload {
                let error = ParseError::TooLarge {
                    declared: payload_len,
                    maximum: self.maximum_payload,
                };
                self.buffer.clear();
                self.start = 0;
                self.poisoned = Some(error.clone());
                return Err(error);
            }
            let total = MIRROR_HEADER_LEN
                .checked_add(payload_len)
                .ok_or(ParseError::Invalid("packet size"))?;
            if available < total {
                break;
            }
            let payload_start = self.start + MIRROR_HEADER_LEN;
            packets.push(MirrorPacket {
                payload_len,
                packet_type,
                option,
                timestamp,
                payload: self.buffer[payload_start..payload_start + payload_len].to_vec(),
            });
            self.start += total;
        }
        if self.start > 0 {
            self.buffer.drain(..self.start);
            self.start = 0;
        }
        Ok(packets)
    }
    /// Bytes of a frame that is still incomplete.
    pub fn buffered_len(&self) -> usize {
        self.buffer.len() - self.start
    }
    /// Discards buffered bytes and clears a fatal framing error.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.start = 0;
        self.poisoned = None;
    }
}

pub fn nal_length_prefixed_to_annex_b(input: &[u8]) -> Result<Vec<u8>, ParseError> {
    let mut output = Vec::with_capacity(input.len().saturating_add(16));
    let mut offset = 0;
    while offset < input.len() {
        if input.len() - offset < 4 {
            return Err(ParseError::Incomplete);
        }
        let length = u32::from_be_bytes(
            input[offset..offset + 4]
                .try_into()
                .map_err(|_| ParseError::Invalid("NAL length"))?,
        ) as usize;
        offset += 4;
        if length == 0 {
            return Err(ParseError::Invalid("empty NAL unit"));
        }
        let end = offset
            .checked_add(length)
            .ok_or(ParseError::Invalid("NAL size"))?;
        if end > input.len() {
            return Err(ParseError::Incomplete);
        }
        output.extend_from_slice(&[0, 0, 0, 1]);
        output.extend_from_slice(&input[offset..end]);
        offset = end;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; MIRROR_HEADER_LEN];
        bytes[..4].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes[4..6].copy_from_slice(&[0, 0x10]);
        bytes[8..16].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }
    #[test]
    fn accepts_split_tcp_packets() {
        let bytes = packet(b"video");
        let mut decoder = MirrorDecoder::new();
        assert!(decoder.push(&bytes[..31]).unwrap().is_empty());
        let packets = decoder.push(&bytes[31..]).unwrap();
        assert_eq!(packets[0].payload, b"video");
        assert_eq!(packets[0].timestamp, 0x1122_3344_5566_7788);
    }
    #[test]
    fn accepts_coalesced_packets_in_one_push() {
        let mut bytes = packet(b"first");
        bytes.extend_from_slice(&packet(b"second"));
        let mut decoder = MirrorDecoder::new();
        let packets = decoder.push(&bytes).unwrap();
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].payload, b"first");
        assert_eq!(packets[1].payload, b"second");
        assert_eq!(decoder.buffered_len(), 0);
    }
    #[test]
    fn keeps_partial_header_and_payload_buffered() {
        let bytes = packet(b"frame");
        let mut decoder = MirrorDecoder::new();
        assert!(decoder.push(&bytes[..10]).unwrap().is_empty());
        assert_eq!(decoder.buffered_len(), 10);
        let packets = decoder.push(&bytes[10..]).unwrap();
        assert_eq!(packets[0].payload, b"frame");
        assert_eq!(decoder.buffered_len(), 0);
    }
    #[test]
    fn oversized_payload_poisons_until_reset() {
        let mut decoder = MirrorDecoder::with_maximum_payload(4);
        let error = ParseError::TooLarge {
            declared: 5,
            maximum: 4,
        };
        assert_eq!(decoder.push(&packet(b"12345")), Err(error.clone()));
        assert_eq!(decoder.buffered_len(), 0);
        assert_eq!(decoder.push(&packet(b"ok")), Err(error));
        decoder.reset();
        let packets = decoder.push(&packet(b"ok")).unwrap();
        assert_eq!(packets[0].payload, b"ok");
    }
    #[test]
    fn converts_nals_and_rejects_truncation() {
        assert_eq!(
            nal_length_prefixed_to_annex_b(&[0, 0, 0, 2, 0x67, 0x42]).unwrap(),
            vec![0, 0, 0, 1, 0x67, 0x42]
        );
        assert_eq!(
            nal_length_prefixed_to_annex_b(&[0, 0, 0, 3, 1]),
            Err(ParseError::Incomplete)
        );
    }
}
