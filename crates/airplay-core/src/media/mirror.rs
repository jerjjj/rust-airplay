// SPDX-License-Identifier: GPL-3.0-or-later
//! The mirror stream: its keys, and the packets that carry video.
//!
//! A mirroring client sets up a stream of its own with `SETUP`, and the two
//! things that make it different from audio are that it travels over TCP and that
//! its keystream is derived rather than carried. The derivation is written out in
//! upstream (`upstream/UxPlay@3dbf7cee:lib/mirror_buffer.c`, `mirror_buffer_init_aes`):
//!
//! ```text
//! key = SHA-512("AirPlayStreamKey" + decimal(streamConnectionID) + audio key)
//! iv  = SHA-512("AirPlayStreamIV"  + decimal(streamConnectionID) + audio key)
//! ```
//!
//! both truncated to sixteen bytes, with the audio key being the one the
//! session's `SETUP` carried. `streamConnectionID` is what the client sent in the
//! `type: 110` stream entry, which this port already keeps
//! (`server::SetupStream::stream_connection_id`).
//!
//! The same file records a consequence that is easy to miss: a re-key means a new
//! stream, so the AES-CTR keystream restarts at offset zero. Nothing here holds
//! state across streams, so that is a property of how a caller uses
//! [`StreamKeys`] rather than of the type.

use crate::crypto::sha512_concat;

/// Length of the key and IV a mirror stream uses
/// (`RAOP_AESKEY_LEN` in upstream).
pub const STREAM_KEY_LEN: usize = 16;

/// Prefix of the material a mirror stream's key is hashed from.
const KEY_PREFIX: &str = "AirPlayStreamKey";

/// Prefix of the material a mirror stream's IV is hashed from.
const IV_PREFIX: &str = "AirPlayStreamIV";

/// Length of the header every mirror packet starts with
/// (`upstream/UxPlay@3dbf7cee:lib/raop_rtp_mirror.c:298-302` reads exactly 128
/// bytes before it looks at anything else).
pub const HEADER_LEN: usize = 128;

/// Largest payload this port reads, matching upstream's own buffer
/// (`RAOP_PACKET_LEN` in `raop_rtp_mirror.c`).
pub const MAX_PAYLOAD: usize = 32_768;

/// What a mirror packet carries.
///
/// Upstream identifies the kind by `packet[4]` and `packet[5]`
/// (`raop_rtp_mirror.c:339-344`). The distinction matters because the video
/// packets are encrypted with the stream's derived keys and the rest are not:
/// decrypting a keep-alive produces nonsense, and not decrypting video produces a
/// black screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorKind {
    /// Encrypted video, whose payload holds one or more NAL units.
    Video {
        /// Whether the payload starts with an IDR frame, which a decoder needs to
        /// start.
        idr: bool,
    },
    /// Unencrypted codec configuration: a sequence parameter set and a picture
    /// parameter set, which a decoder needs before the first frame.
    CodecData,
    /// An unencrypted packet with no payload, sent once a second.
    KeepAlive,
    /// An unencrypted report about the stream, sent once a second.
    StreamingReport,
    /// A pair this port does not know, kept as it arrived.
    Unknown {
        /// `packet[4]`.
        first: u8,
        /// `packet[5]`.
        second: u8,
    },
}

impl MirrorKind {
    /// Whether the payload is encrypted with the stream's keys.
    pub fn is_encrypted(self) -> bool {
        matches!(self, Self::Video { .. })
    }

    /// Reads the kind from the two bytes upstream reads it from.
    fn from_header(first: u8, second: u8) -> Self {
        match (first, second) {
            (0x00, 0x00) => Self::Video { idr: false },
            (0x00, 0x10) => Self::Video { idr: true },
            (0x01, 0x00) => Self::CodecData,
            (0x02, 0x00) => Self::KeepAlive,
            (0x05, 0x00) => Self::StreamingReport,
            (first, second) => Self::Unknown { first, second },
        }
    }
}

/// The header of one mirror packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirrorHeader {
    /// Bytes the payload holds, from `packet[0..4]`.
    pub payload_size: u32,
    /// What the payload is.
    pub kind: MirrorKind,
    /// The two bytes upstream keeps as the payload's option, from `packet[6..8]`.
    pub option: u16,
    /// The client's timestamp, from `packet[8..16]`.
    ///
    /// Upstream notes that a streaming report has none
    /// (`raop_rtp_mirror.c:363`), so it is zero there and must not be read as a
    /// time.
    pub timestamp: u64,
}

/// Why a mirror packet could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorError {
    /// Shorter than the header every packet starts with.
    TooShort {
        /// Length it has.
        length: usize,
    },
    /// The header promises more payload than the datagram carries.
    Truncated {
        /// Bytes the header promised.
        promised: usize,
        /// Bytes that are there.
        present: usize,
    },
    /// A payload size beyond what a receiver reads.
    TooLarge {
        /// Bytes the header promised.
        promised: usize,
        /// Bytes a receiver accepts.
        maximum: usize,
    },
}

impl std::fmt::Display for MirrorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { length } => write!(
                formatter,
                "the mirror packet is {length} bytes, shorter than its header"
            ),
            Self::Truncated { promised, present } => write!(
                formatter,
                "the header promises {promised} bytes of payload but {present} are there"
            ),
            Self::TooLarge { promised, maximum } => write!(
                formatter,
                "the header promises {promised} bytes of payload, more than the {maximum} a receiver reads"
            ),
        }
    }
}

impl std::error::Error for MirrorError {}

/// Reads one mirror packet.
///
/// # Errors
///
/// Returns [`MirrorError`] for a packet shorter than its header, one whose header
/// promises more payload than is present, or one that promises more than
/// [`MAX_PAYLOAD`].
pub fn parse_packet(datagram: &[u8]) -> Result<(MirrorHeader, &[u8]), MirrorError> {
    let header = parse_header(datagram)?;
    let promised = header.payload_size as usize;
    let present = datagram.len() - HEADER_LEN;
    if promised > present {
        return Err(MirrorError::Truncated { promised, present });
    }
    Ok((header, &datagram[HEADER_LEN..HEADER_LEN + promised]))
}

/// Reads just the header, for a caller that reads the payload separately.
///
/// A TCP reader has to do that: it reads the header first, learns how much
/// follows and then reads exactly that, because a stream does not arrive in
/// packets (`raop_rtp_mirror.c:298-302` loops the same way).
///
/// # Errors
///
/// Returns [`MirrorError::TooShort`] for fewer than [`HEADER_LEN`] bytes and
/// [`MirrorError::TooLarge`] for a payload size beyond [`MAX_PAYLOAD`].
pub fn parse_header(datagram: &[u8]) -> Result<MirrorHeader, MirrorError> {
    if datagram.len() < HEADER_LEN {
        return Err(MirrorError::TooShort {
            length: datagram.len(),
        });
    }
    let header = MirrorHeader {
        payload_size: u32::from_be_bytes(datagram[0..4].try_into().expect("four bytes")),
        kind: MirrorKind::from_header(datagram[4], datagram[5]),
        option: u16::from_be_bytes(datagram[6..8].try_into().expect("two bytes")),
        timestamp: u64::from_be_bytes(datagram[8..16].try_into().expect("eight bytes")),
    };
    let promised = header.payload_size as usize;
    if promised > MAX_PAYLOAD {
        return Err(MirrorError::TooLarge {
            promised,
            maximum: MAX_PAYLOAD,
        });
    }
    Ok(header)
}

/// The key and IV of one mirror stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamKeys {
    /// AES-128-CTR key of the stream.
    pub key: [u8; STREAM_KEY_LEN],
    /// AES-128-CTR initial counter block of the stream.
    pub iv: [u8; STREAM_KEY_LEN],
}

/// Derives the keys of the mirror stream a client identified with
/// `connection_id`, using the audio key of the session it belongs to.
///
/// The decimal form of the identifier matters: upstream formats it with `%llu`,
/// so the hash covers text, not bytes. A port that hashed the number's bytes
/// would produce a stream the client cannot read, with no error anywhere.
pub fn stream_keys(connection_id: u64, audio_key: &[u8; STREAM_KEY_LEN]) -> StreamKeys {
    let key_material = format!("{KEY_PREFIX}{connection_id}");
    let iv_material = format!("{IV_PREFIX}{connection_id}");
    let key = sha512_concat(&[key_material.as_bytes(), audio_key]);
    let iv = sha512_concat(&[iv_material.as_bytes(), audio_key]);
    StreamKeys {
        key: key[..STREAM_KEY_LEN]
            .try_into()
            .expect("sixteen of sixty-four"),
        iv: iv[..STREAM_KEY_LEN]
            .try_into()
            .expect("sixteen of sixty-four"),
    }
}

/// The Annex-B start code that replaces a length prefix
/// (`upstream/UxPlay@3dbf7cee:lib/raop_rtp_mirror.c:453-454`, `nal_start_code`).
pub const START_CODE: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// Why a decrypted payload is not a sequence of NAL units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NalError {
    /// A length prefix claims bytes the payload does not have.
    LengthOutOfRange {
        /// Offset the prefix was read at.
        offset: usize,
        /// Length it claimed.
        claimed: usize,
        /// Bytes actually left after the prefix.
        available: usize,
    },
    /// The first byte of a unit has the bit set that has to be zero.
    ForbiddenBit {
        /// Offset of the unit's first byte.
        offset: usize,
    },
}

impl std::fmt::Display for NalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LengthOutOfRange {
                offset,
                claimed,
                available,
            } => write!(
                formatter,
                "the unit at {offset} claims {claimed} bytes but {available} are left"
            ),
            Self::ForbiddenBit { offset } => {
                write!(formatter, "the unit at {offset} has its forbidden bit set")
            }
        }
    }
}

impl std::error::Error for NalError {}

/// Rewrites the length prefixes of a decrypted mirror payload into Annex-B start
/// codes, in place, and reports how many units it holds.
///
/// The AirPlay mirror protocol prefixes every NAL unit with its length, which a
/// decoder does not expect; upstream replaces each prefix with `00 00 00 01`
/// while it checks the unit
/// (`upstream/UxPlay@3dbf7cee:lib/raop_rtp_mirror.c:453-474`). Doing the same
/// here means a caller hands the payload to a decoder unchanged after this.
///
/// The checks are upstream's, and they matter: a length read out of the payload
/// is only a length if the unit it claims fits in what is left, and a unit whose
/// first byte has the forbidden bit set is not a unit at all. A payload that
/// fails either check is reported instead of being passed on, because a decoder
/// fed a wrong byte stream does not fail cleanly.
///
/// # Errors
///
/// Returns [`NalError`] for a payload that is not a sequence of units.
pub fn into_annex_b(payload: &mut [u8]) -> Result<usize, NalError> {
    let mut offset = 0;
    let mut units = 0;
    while offset < payload.len() {
        if offset + 4 > payload.len() {
            return Err(NalError::LengthOutOfRange {
                offset,
                claimed: 0,
                available: payload.len() - offset,
            });
        }
        let claimed =
            u32::from_be_bytes(payload[offset..offset + 4].try_into().expect("four")) as usize;
        let available = payload.len() - offset - 4;
        if claimed > available {
            return Err(NalError::LengthOutOfRange {
                offset,
                claimed,
                available,
            });
        }
        payload[offset..offset + 4].copy_from_slice(&START_CODE);
        offset += 4;
        units += 1;
        // The first bit of an H.264 unit has to be zero. Upstream reads the byte
        // even for a unit of length zero, so this does too.
        let first = payload.get(offset).copied().unwrap_or_default();
        if first & 0x80 != 0 {
            return Err(NalError::ForbiddenBit { offset });
        }
        offset += claimed;
    }
    Ok(units)
}
/// The keystream of one mirror stream.
///
/// Video packets are decrypted with AES-128-CTR, and the counter does **not**
/// restart at each packet: a packet continues where the previous one left the
/// keystream. Upstream keeps that position in `mirror_buffer` and throws it away
/// when the stream is re-keyed (`mirror_buffer.c`, which sets its counter to zero
/// there), and this type is that position, with [`StreamCipher::restart`] for the
/// re-key.
///
/// # One rule is deliberately missing
///
/// Upstream also accounts for a packet whose length is not a whole number of
/// blocks, because the cipher it drives advances by blocks: the leftover bytes of
/// the last block have to be skipped before the next packet starts. This type does
/// not do that yet, and the reason is that the exact rule has to be read out of
/// `mirror_buffer.c` rather than guessed - a guess here shows up as video that
/// decodes into noise from the second packet on, which is far harder to notice
/// than an error. `docs/S4_WORK_PLAN.md` records it as the next step of this
/// slice, and nothing wires this cipher into a stream before it lands.
pub struct StreamCipher {
    state: crate::crypto::Aes128CtrState,
    advanced: usize,
}

impl StreamCipher {
    /// Starts the keystream of a stream at its beginning.
    ///
    /// # Errors
    ///
    /// Returns [`crate::crypto::CryptoError`] when the keys have the wrong shape.
    pub fn new(keys: &StreamKeys) -> Result<Self, crate::crypto::CryptoError> {
        Ok(Self {
            state: crate::crypto::Aes128CtrState::new(&keys.key, &keys.iv)?,
            advanced: 0,
        })
    }

    /// Decrypts one payload in place, continuing the keystream.
    ///
    /// # Errors
    ///
    /// Returns [`crate::crypto::CryptoError`] when the cipher refuses the input.
    pub fn decrypt(&mut self, payload: &mut [u8]) -> Result<(), crate::crypto::CryptoError> {
        self.state.apply(payload);
        self.advanced += payload.len();
        Ok(())
    }

    /// Bytes the keystream has advanced, which is what a caller reports when it
    /// wants to know how far into a stream it is.
    pub fn advanced(&self) -> usize {
        self.advanced
    }

    /// Starts the keystream over, as a re-key does.
    ///
    /// # Errors
    ///
    /// Returns [`crate::crypto::CryptoError`] when the cipher refuses to reset.
    pub fn restart(&mut self) -> Result<(), crate::crypto::CryptoError> {
        self.state.reset()?;
        self.advanced = 0;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// A mirror packet with `payload` after the header upstream reads.
    fn packet(first: u8, second: u8, option: u16, timestamp: u64, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.push(first);
        bytes.push(second);
        bytes.extend_from_slice(&option.to_be_bytes());
        bytes.extend_from_slice(&timestamp.to_be_bytes());
        bytes.resize(HEADER_LEN, 0);
        bytes.extend_from_slice(payload);
        bytes
    }

    /// A payload of length-prefixed units, as the client sends them.
    fn units(units: &[&[u8]]) -> Vec<u8> {
        let mut payload = Vec::new();
        for unit in units {
            payload.extend_from_slice(&(unit.len() as u32).to_be_bytes());
            payload.extend_from_slice(unit);
        }
        payload
    }

    #[test]
    fn the_keystream_continues_from_one_packet_to_the_next() {
        let keys = stream_keys(7, &[0x31; STREAM_KEY_LEN]);
        let first = [0x11u8; 40];
        let second = [0x22u8; 24];

        // Two packets in sequence are the same as one payload of their length,
        // which is what "the counter does not restart per packet" means.
        let mut cipher = StreamCipher::new(&keys).expect("cipher");
        let mut one = first;
        let mut two = second;
        cipher.decrypt(&mut one).expect("first");
        cipher.decrypt(&mut two).expect("second");
        assert_eq!(cipher.advanced(), 64);

        let mut whole = Vec::new();
        whole.extend_from_slice(&first);
        whole.extend_from_slice(&second);
        let mut expected_a = [0x11u8; 40];
        let mut expected_b = [0x22u8; 24];
        let mut reference = StreamCipher::new(&keys).expect("cipher");
        let mut joined = Vec::new();
        joined.extend_from_slice(&expected_a);
        joined.extend_from_slice(&expected_b);
        reference.decrypt(&mut joined).expect("joined");
        expected_a.copy_from_slice(&joined[..40]);
        expected_b.copy_from_slice(&joined[40..]);
        assert_eq!(one, expected_a);
        assert_eq!(two, expected_b);
    }

    #[test]
    fn a_rekey_starts_the_keystream_over() {
        let keys = stream_keys(9, &[0x41; STREAM_KEY_LEN]);
        let mut cipher = StreamCipher::new(&keys).expect("cipher");
        let mut payload = [0x55u8; 32];
        cipher.decrypt(&mut payload).expect("first");
        assert_eq!(cipher.advanced(), 32);
        // The same bytes again: without the restart they would decrypt to
        // something else, because the keystream has moved on.
        cipher.restart().expect("restart");
        assert_eq!(cipher.advanced(), 0);
        let mut again = [0x55u8; 32];
        cipher.decrypt(&mut again).expect("again");
        assert_eq!(again, payload, "the stream began again");
    }

    #[test]
    fn rewrites_length_prefixes_into_start_codes() {
        // An IDR slice and a non-IDR slice, the two shapes a stream carries.
        let mut payload = units(&[&[0x65, 0xaa, 0xbb], &[0x41, 0xcc]]);
        assert_eq!(into_annex_b(&mut payload).expect("two units"), 2);
        assert_eq!(
            payload,
            [
                0x00, 0x00, 0x00, 0x01, 0x65, 0xaa, 0xbb, 0x00, 0x00, 0x00, 0x01, 0x41, 0xcc
            ]
        );
        // An empty payload holds no units and is not an error.
        assert_eq!(into_annex_b(&mut []).expect("empty"), 0);
    }

    #[test]
    fn refuses_a_length_that_does_not_fit() {
        // The prefix claims eight bytes but two follow.
        let mut payload = vec![0, 0, 0, 8, 0x65, 0xaa];
        assert_eq!(
            into_annex_b(&mut payload),
            Err(NalError::LengthOutOfRange {
                offset: 0,
                claimed: 8,
                available: 2
            })
        );
        // A payload that ends inside a prefix is refused too, rather than being
        // read as a length.
        let mut short = vec![0, 0, 0x65, 0xaa];
        assert!(matches!(
            into_annex_b(&mut short),
            Err(NalError::LengthOutOfRange { offset: 0, .. })
        ));
    }

    #[test]
    fn refuses_a_unit_whose_forbidden_bit_is_set() {
        // Upstream checks the bit before it looks at anything else in the unit
        // (raop_rtp_mirror.c:470-474).
        let mut payload = units(&[&[0x85, 0x00]]);
        assert_eq!(
            into_annex_b(&mut payload),
            Err(NalError::ForbiddenBit { offset: 4 })
        );
    }

    #[test]
    fn reads_the_header_upstream_reads() {
        let bytes = packet(0x00, 0x10, 0x0000, 0x1122_3344_5566_7788, &[1, 2, 3, 4]);
        let (header, payload) = parse_packet(&bytes).expect("packet");
        assert_eq!(header.payload_size, 4);
        assert_eq!(header.kind, MirrorKind::Video { idr: true });
        assert_eq!(header.option, 0);
        assert_eq!(header.timestamp, 0x1122_3344_5566_7788);
        assert_eq!(payload, [1, 2, 3, 4]);
    }

    #[test]
    fn tells_encrypted_video_from_what_must_stay_clear() {
        // The pairs upstream lists, and which of them a receiver decrypts.
        for (first, second, kind, encrypted) in [
            (0x00, 0x00, MirrorKind::Video { idr: false }, true),
            (0x00, 0x10, MirrorKind::Video { idr: true }, true),
            (0x01, 0x00, MirrorKind::CodecData, false),
            (0x02, 0x00, MirrorKind::KeepAlive, false),
            (0x05, 0x00, MirrorKind::StreamingReport, false),
            (
                0x07,
                0x03,
                MirrorKind::Unknown {
                    first: 0x07,
                    second: 0x03,
                },
                false,
            ),
        ] {
            let bytes = packet(first, second, 0, 0, &[0; 8]);
            let (header, _) = parse_packet(&bytes).expect("packet");
            assert_eq!(header.kind, kind, "{first:#04x} {second:#04x}");
            assert_eq!(header.kind.is_encrypted(), encrypted, "{kind:?}");
        }
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        assert_eq!(
            parse_packet(&[0; 127]),
            Err(MirrorError::TooShort { length: 127 })
        );
        // The header promises more than the packet carries.
        let mut bytes = packet(0x00, 0x00, 0, 0, &[0; 4]);
        bytes.truncate(HEADER_LEN + 2);
        assert_eq!(
            parse_packet(&bytes),
            Err(MirrorError::Truncated {
                promised: 4,
                present: 2
            })
        );
        // And one that promises more than a receiver reads, before allocating.
        let mut huge = vec![0u8; HEADER_LEN];
        huge[0..4].copy_from_slice(&(MAX_PAYLOAD as u32 + 1).to_be_bytes());
        assert_eq!(
            parse_packet(&huge),
            Err(MirrorError::TooLarge {
                promised: MAX_PAYLOAD + 1,
                maximum: MAX_PAYLOAD
            })
        );
    }

    #[test]
    fn parses_the_construction_upstream_writes_out() {
        // The test rebuilds the hash itself, so a change to the prefixes, the
        // decimal formatting or the audio key's place in the hash fails here
        // rather than on a device.
        let connection_id = 2_765_102_747_429_192_913u64;
        let audio_key = [0x5a; STREAM_KEY_LEN];
        let keys = stream_keys(connection_id, &audio_key);

        let expected_key = sha512_concat(&[
            format!("AirPlayStreamKey{connection_id}").as_bytes(),
            &audio_key,
        ]);
        let expected_iv = sha512_concat(&[
            format!("AirPlayStreamIV{connection_id}").as_bytes(),
            &audio_key,
        ]);
        assert_eq!(keys.key, expected_key[..STREAM_KEY_LEN]);
        assert_eq!(keys.iv, expected_iv[..STREAM_KEY_LEN]);
        assert_ne!(keys.key, keys.iv, "the two prefixes differ");
    }

    #[test]
    fn the_identifier_is_hashed_as_text() {
        // 123 hashed as text is not 123 hashed as bytes; the test pins the form
        // upstream uses so this cannot drift silently.
        let audio_key = [0x11; STREAM_KEY_LEN];
        let as_text = stream_keys(123, &audio_key);
        let mut as_bytes = [0u8; 8];
        as_bytes.copy_from_slice(&123u64.to_be_bytes());
        let mut material = b"AirPlayStreamKey".to_vec();
        material.extend_from_slice(&as_bytes);
        let hashed_bytes = sha512_concat(&[&material, &audio_key]);
        assert_ne!(as_text.key, hashed_bytes[..STREAM_KEY_LEN]);
    }

    #[test]
    fn a_different_stream_or_key_gives_different_material() {
        let audio_key = [0x22; STREAM_KEY_LEN];
        let first = stream_keys(1, &audio_key);
        let other_stream = stream_keys(2, &audio_key);
        let other_key = stream_keys(1, &[0x23; STREAM_KEY_LEN]);
        assert_ne!(first.key, other_stream.key);
        assert_ne!(first.key, other_key.key);
        assert_ne!(first.iv, other_stream.iv);
        // The same inputs always give the same material: a caller may derive it
        // again after a re-key without asking the client anything.
        assert_eq!(first, stream_keys(1, &audio_key));
    }
}
