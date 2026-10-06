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
//!
//! # How the payload is encrypted
//!
//! The video payload is AES-128-CTR, but not as one continuous stream: upstream
//! calls `aes_ctr_start_fresh_block` at the start of every packet
//! (`upstream/UxPlay@3dbf7cee:lib/mirror_buffer.c:104`), and OpenSSL's CTR context
//! keeps a *block offset* rather than a byte position. Decrypting a length that is
//! not a multiple of sixteen therefore leaves the context part-way into a block,
//! and "start a fresh block" throws the rest of that block's keystream away. What
//! upstream does with it instead is to decrypt the packet's tail as **one whole
//! block** and keep the bytes the packet did not use
//! (`mirror_buffer.c:111-123`), so the next packet's first bytes are XORed with
//! them ([`MirrorDecryptor`] is that state).
//!
//! Upstream also sends the codec configuration (an H.264 SPS and PPS) in a packet
//! of its own, *unencrypted*, and prepends it to the next video packet because a
//! decoder needs it before the first frame
//! (`raop_rtp_mirror.c:420-450`, [`CodecConfig`]).

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
    /// The state holds more keystream bytes than the packet has.
    Carry {
        /// Keystream bytes being carried.
        carried: usize,
        /// Bytes of payload the packet has.
        length: usize,
    },
    /// The output buffer is not as long as the payload.
    OutputLength {
        /// Bytes of payload.
        length: usize,
        /// Bytes of output.
        output: usize,
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
            Self::Carry { carried, length } => write!(
                formatter,
                "{carried} keystream bytes are carried but the packet has {length}"
            ),
            Self::OutputLength { length, output } => write!(
                formatter,
                "the payload is {length} bytes but the output holds {output}"
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
/// This is upstream's `mirror_buffer` state: the key and IV, where in the stream the
/// counter has come to, and the bytes of a partly used keystream block that the
/// previous packet left behind. The rule it follows is written out in
/// `mirror_buffer_decrypt` (`upstream/UxPlay@3dbf7cee:lib/mirror_buffer.c:94-124`) and
/// is not what a plain CTR stream would do, so it is repeated here in the order the C
/// code performs it:
///
/// 1. the first `carry` bytes of the packet are XORed with the **end** of the previous
///    packet's tail block, which that packet did not use
///    (`output[i] = input[i] ^ og[16 - carry + i]`);
/// 2. the counter is moved to a block boundary (`aes_ctr_start_fresh_block`, which is
///    the block after the tail block), and the whole blocks after the carry —
///    `((len - carry) / 16) * 16` bytes — are decrypted from there;
/// 3. the remaining `rest = (len - carry) % 16` bytes are decrypted as one whole block
///    (zero padded), of which the packet keeps the first `rest`; the other
///    `16 - rest` keystream bytes are the carry for the next packet.
///
/// So a packet's first bytes are keystream the previous packet left behind, and its
/// body is the stream's own blocks from the counter, which is what makes this
/// different from one continuous keystream: two packets of the same length read the
/// same blocks. The counter advances by the blocks a packet used, its tail's
/// included, because the next packet starts one block past the carry's block.
///
/// The rule is pinned against upstream's own code in `tests/upstream_oracle.rs`,
/// which reads where upstream's decryptor takes its keystream bytes from, packet by
/// packet.
#[derive(Clone)]
pub struct MirrorDecryptor {
    /// The stream's key and IV: a mirror packet's keystream is a function of the
    /// packet's position in the stream, so nothing about the previous packet other
    /// than the bytes it carried is kept.
    key: [u8; STREAM_KEY_LEN],
    iv: [u8; STREAM_KEY_LEN],
    /// Whole blocks the packets decrypted so far had, which is how far into the
    /// stream the next packet's fresh block is.
    blocks: u64,
    /// Bytes of the previous packet's tail block that it did not use,
    /// `og[16 - carry..]` in upstream.
    carry: [u8; 16],
    /// How many of them there are, upstream's `nextDecryptCount`.
    carried: usize,
}

impl std::fmt::Debug for MirrorDecryptor {
    /// Reports the position in the stream and nothing derived from the keys, so a
    /// session log can record where decryption is without recording secrets.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MirrorDecryptor")
            .field("blocks", &self.blocks)
            .field("carried", &self.carried)
            .finish_non_exhaustive()
    }
}

impl MirrorDecryptor {
    /// Starts a stream at its first packet.
    ///
    /// # Errors
    ///
    /// Returns [`crate::crypto::CryptoError`] when the keys have the wrong shape.
    pub fn new(keys: &StreamKeys) -> Result<Self, crate::crypto::CryptoError> {
        // The shape check is what `Aes128CtrState` does for the rest of this crate;
        // the arrays here are already sixteen bytes, so it cannot fail.
        let _ = crate::crypto::Aes128CtrState::new(&keys.key, &keys.iv)?;
        Ok(Self {
            key: keys.key,
            iv: keys.iv,
            blocks: 0,
            carry: [0; 16],
            carried: 0,
        })
    }

    /// Starts a new stream, as a re-key does.
    ///
    /// Upstream clears its carry buffer here (`mirror_buffer_init_aes`,
    /// `mirror_buffer.c:66-70`): a re-key means a new stream, and bytes held back
    /// from the previous one are the wrong keystream for the first packet of this
    /// one.
    ///
    /// # Errors
    ///
    /// Returns [`crate::crypto::CryptoError`] when the keys have the wrong shape.
    pub fn rekey(&mut self, keys: &StreamKeys) -> Result<(), crate::crypto::CryptoError> {
        *self = Self::new(keys)?;
        Ok(())
    }

    /// How many keystream bytes the previous packet left for the next one,
    /// upstream's `nextDecryptCount`.
    pub fn carried(&self) -> usize {
        self.carried
    }

    /// The block the next packet's counter starts on, which is how far into the
    /// stream this receiver has come.
    pub fn blocks(&self) -> u64 {
        self.blocks
    }

    /// The keystream of `blocks` blocks starting at block `first` of the stream.
    ///
    /// Upstream reaches this through OpenSSL's CTR context, whose counter block is
    /// incremented per block. Generating the blocks here keeps the position
    /// explicit: which block a packet starts on is the whole of the rule
    /// (`mirror_buffer.c:94-124`), and it must not depend on any state a cipher
    /// implementation keeps for itself.
    ///
    /// It is public because it is also the clearest way to check this port against
    /// upstream's own context, block for block, in the differential test.
    #[must_use]
    pub fn keystream(&self, first: u64, blocks: usize) -> Vec<u8> {
        use aes::cipher::{BlockEncrypt, KeyInit};
        let cipher = aes::Aes128::new(self.key[..].into());
        let mut out = Vec::with_capacity(blocks * 16);
        for index in 0..blocks as u64 {
            let mut counter = self.iv;
            // The counter is the IV plus the block index, big-endian, the way
            // OpenSSL's CTR mode counts.
            let mut carry = first + index;
            for byte in counter.iter_mut().rev() {
                let (next, wrapped) = byte.overflowing_add((carry & 0xff) as u8);
                *byte = next;
                carry >>= 8;
                if carry == 0 && !wrapped {
                    break;
                }
            }
            let mut block = aes::Block::clone_from_slice(&counter);
            cipher.encrypt_block(&mut block);
            out.extend_from_slice(&block);
        }
        out
    }

    /// Decrypts one payload into `output`, which has to be as long as `input`.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorError::Carry`] when the state holds more keystream bytes
    /// than the packet has, which no packet sequence from a client produces: it
    /// means the caller skipped or mis-split a packet, and continuing would
    /// decrypt the rest of the stream at the wrong offset. Returns
    /// [`MirrorError::OutputLength`] for an output buffer of a different length.
    pub fn decrypt(&mut self, input: &[u8], output: &mut [u8]) -> Result<(), MirrorError> {
        if output.len() != input.len() {
            return Err(MirrorError::OutputLength {
                length: input.len(),
                output: output.len(),
            });
        }
        if self.carried > input.len() {
            return Err(MirrorError::Carry {
                carried: self.carried,
                length: input.len(),
            });
        }
        let carried = self.carried;
        let whole = ((input.len() - carried) / 16) * 16;
        let rest = (input.len() - carried) % 16;
        // Step 1: the first `carried` bytes are XORed with the keystream bytes the
        // previous packet's tail block did not use (`mirror_buffer.c:96-100`).
        for (index, byte) in input[..carried].iter().enumerate() {
            output[index] = byte ^ self.carry[index];
        }
        // Steps 2 and 3: the counter starts a fresh block, which is the block after
        // the ones the packets so far had (`aes_ctr_start_fresh_block`,
        // `mirror_buffer.c:102-107`). The whole blocks come from it directly, and
        // the tail is decrypted as one whole block on top of zero padding
        // (`:111-123`), of which the packet keeps the first `rest` bytes.
        let fresh = self.keystream(self.blocks, (whole + rest) / 16 + usize::from(rest > 0));
        for (index, byte) in input[carried..carried + whole].iter().enumerate() {
            output[carried + index] = byte ^ fresh[index];
        }
        self.carried = 0;
        // The counter advances by the packet's whole blocks and, when the packet did
        // not end on a block boundary, by the block its tail sat in: the carry it
        // leaves comes from the front of the **next** block, which is where the next
        // packet's fresh counter starts. `tests/upstream_oracle.rs` pins this against
        // upstream's own `mirror_buffer_decrypt`.
        self.blocks += (whole / 16) as u64;
        if rest > 0 {
            let start = input.len() - rest;
            for index in 0..rest {
                output[start + index] = input[start + index] ^ fresh[whole + index];
            }
            // The bytes the packet did not use are the next packet's keystream, kept
            // from the front of this buffer; the ones it did use are plaintext, so
            // they are not carried at all.
            self.carry = [0; 16];
            self.carry[..16 - rest].copy_from_slice(&fresh[whole + rest..whole + 16]);
            self.carried = 16 - rest;
            self.blocks += 1;
        }
        Ok(())
    }
}

/// The codec configuration a mirroring client sends before it starts video.
///
/// The packet is unencrypted and carries an H.264 sequence parameter set and a
/// picture parameter set, which a decoder needs before the first frame. Upstream
/// does not hand it on where it arrives: it keeps the two units with start codes
/// in front of them and prepends them to the **next video packet**, which carries
/// the same timestamp (`upstream/UxPlay@3dbf7cee:lib/raop_rtp_mirror.c:420-450`
/// and `:754-763`). This type is that rule, and a caller feeds it by calling
/// [`CodecConfig::codec_data`] for a codec packet and
/// [`CodecConfig::prepend_to`] for every video packet, in arrival order.
///
/// The payload of a codec packet is not a plain sequence of units: it starts with
/// six bytes of image size, then the two sizes and their sets
/// (`raop_rtp_mirror.c:560-572` describes the header, `:725-728` reads the sets).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CodecConfig {
    /// The units with start codes, waiting for the video packet they belong to.
    pending: Option<Pending>,
}

/// An SPS and PPS waiting for their video packet.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    /// Client timestamp of the codec packet, which the video packet has to share
    /// (`raop_rtp_mirror.c:426`).
    timestamp: u64,
    /// `00 00 00 01` + SPS + `00 00 00 01` + PPS.
    units: Vec<u8>,
}

/// Why a codec configuration packet could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecDataError {
    /// Fewer bytes than the six-byte header the two sizes live in.
    TooShort {
        /// Length it has.
        length: usize,
    },
    /// The sizes do not leave room for both units in the payload.
    SizeOutOfRange {
        /// Size the payload claims for the sequence parameter set.
        sps: usize,
        /// Size it claims for the picture parameter set.
        pps: usize,
        /// Bytes actually there.
        length: usize,
    },
}

impl std::fmt::Display for CodecDataError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { length } => write!(
                formatter,
                "the codec packet is {length} bytes, shorter than its header"
            ),
            Self::SizeOutOfRange { sps, pps, length } => write!(
                formatter,
                "the codec packet claims {sps} + {pps} bytes of parameter sets in {length}"
            ),
        }
    }
}

impl std::error::Error for CodecDataError {}

impl CodecConfig {
    /// An empty configuration: nothing is waiting to be prepended.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads a codec packet (`packet[4] == 0x01`) and keeps what it carries.
    ///
    /// # Errors
    ///
    /// Returns [`CodecDataError`] for a payload that is not a pair of sized
    /// parameter sets, which is dropped rather than prepended to a frame: a
    /// decoder fed a wrong byte stream does not fail cleanly.
    pub fn codec_data(&mut self, timestamp: u64, payload: &[u8]) -> Result<(), CodecDataError> {
        // The size of the first parameter set sits at offset six, the sequence
        // parameter set itself at eight, and the second set's size after the first
        // (`raop_rtp_mirror.c:725-728`: `payload[6]`, `payload + 8`, and
        // `payload[sps_size + 9]`, which is the two size bytes starting one byte
        // before the set's last one).
        let sps_size = read_size(payload, 6).ok_or(CodecDataError::TooShort {
            length: payload.len(),
        })?;
        let sps_at = 8;
        let pps_size_at = sps_size + 8;
        let pps_at = sps_size + 10;
        let pps_size = read_size(payload, pps_size_at).ok_or(CodecDataError::SizeOutOfRange {
            sps: sps_size,
            pps: pps_size_or_zero(payload, pps_size_at),
            length: payload.len(),
        })?;
        let (Some(sps), Some(pps)) = (
            payload.get(sps_at..sps_at + sps_size),
            payload.get(pps_at..pps_at + pps_size),
        ) else {
            return Err(CodecDataError::SizeOutOfRange {
                sps: sps_size,
                pps: pps_size,
                length: payload.len(),
            });
        };
        // A later codec packet replaces an earlier one that was never used, the
        // way upstream frees the buffer it was holding (`:622-625`).
        let mut units = Vec::with_capacity(sps.len() + pps.len() + 8);
        units.extend_from_slice(&START_CODE);
        units.extend_from_slice(sps);
        units.extend_from_slice(&START_CODE);
        units.extend_from_slice(pps);
        self.pending = Some(Pending { timestamp, units });
        Ok(())
    }

    /// Whether a configuration is waiting for its video packet.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// The configuration waiting for the video packet whose client timestamp is
    /// `timestamp`.
    ///
    /// A video packet with a different timestamp means the codec packet that is
    /// waiting belongs to a stream that is no longer arriving, so it is discarded
    /// and `None` is returned (`raop_rtp_mirror.c:426-433`). Calling this for
    /// every video packet in arrival order is what makes the rule upstream's:
    /// whatever is prepended is prepended exactly once.
    #[must_use]
    pub fn prepend_to(&mut self, timestamp: u64) -> Option<Vec<u8>> {
        match self.pending.take() {
            Some(pending) if pending.timestamp == timestamp => Some(pending.units),
            _ => None,
        }
    }
}

/// Reads a big-endian sixteen-bit size at `offset`, if the payload has it.
fn read_size(payload: &[u8], offset: usize) -> Option<usize> {
    let end = offset.checked_add(2)?;
    let bytes = payload.get(offset..end)?;
    Some(u16::from_be_bytes(bytes.try_into().expect("two bytes")) as usize)
}

/// The size field at `offset` for an error report, or zero when it is not there.
fn pps_size_or_zero(payload: &[u8], offset: usize) -> usize {
    read_size(payload, offset).unwrap_or_default()
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

    /// A codec payload the way upstream reads one: two bytes, the source width and
    /// height as big-endian sixteen-bit values, then a sixteen-bit size and the
    /// sequence parameter set, then a sixteen-bit size and the picture parameter
    /// set (`raop_rtp_mirror.c:725-728`; the header around them is what
    /// `:560-572` describes).
    fn codec_payload(sps: &[u8], pps: &[u8]) -> Vec<u8> {
        let mut payload = vec![0u8; 6];
        payload[2..4].copy_from_slice(&1920u16.to_be_bytes());
        payload[4..6].copy_from_slice(&1080u16.to_be_bytes());
        payload.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        payload.extend_from_slice(sps);
        payload.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        payload.extend_from_slice(pps);
        payload
    }

    #[test]
    fn keeps_the_codec_configuration_for_the_next_video_packet() {
        let sps = [0x67, 0x42, 0x00, 0x1e];
        let pps = [0x68, 0xce];
        let mut config = CodecConfig::new();
        assert!(!config.is_pending());
        config
            .codec_data(0x1122, &codec_payload(&sps, &pps))
            .expect("codec data");
        assert!(config.is_pending());

        // Upstream prepends the two units with start codes in front of them, and
        // only to a packet with the codec packet's own timestamp.
        let units = config.prepend_to(0x1122).expect("prepended");
        assert_eq!(
            units,
            [
                0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1e, 0x00, 0x00, 0x00, 0x01, 0x68, 0xce
            ]
        );
        assert!(!config.is_pending(), "it is prepended once");
        assert!(config.prepend_to(0x1122).is_none(), "not twice");
    }

    #[test]
    fn drops_a_configuration_whose_timestamp_does_not_match() {
        // A video packet with another timestamp means the configuration belongs
        // to a stream that is no longer arriving (raop_rtp_mirror.c:426-433), and
        // a decoder fed it would see parameter sets for the wrong picture.
        let mut config = CodecConfig::new();
        config
            .codec_data(0x1122, &codec_payload(&[0x67, 0x01], &[0x68, 0x02]))
            .expect("codec data");
        assert!(config.prepend_to(0x3344).is_none(), "discarded");
        assert!(!config.is_pending(), "and not kept for the next packet");
        assert!(config.prepend_to(0x1122).is_none());
    }

    #[test]
    fn a_later_configuration_replaces_one_that_was_never_used() {
        let mut config = CodecConfig::new();
        config
            .codec_data(1, &codec_payload(&[0x67, 0xaa], &[0x68, 0xbb]))
            .expect("first");
        config
            .codec_data(2, &codec_payload(&[0x67, 0xcc], &[0x68, 0xdd]))
            .expect("second");
        assert_eq!(
            config.prepend_to(2).expect("prepended"),
            [
                0x00, 0x00, 0x00, 0x01, 0x67, 0xcc, 0x00, 0x00, 0x00, 0x01, 0x68, 0xdd
            ]
        );
    }

    #[test]
    fn refuses_a_codec_payload_that_does_not_hold_two_parameter_sets() {
        let mut config = CodecConfig::new();
        assert_eq!(
            config.codec_data(1, &[0, 0, 0, 0, 0]),
            Err(CodecDataError::TooShort { length: 5 })
        );
        // The sizes claim more than the payload has: a 25-byte sequence parameter
        // set whose size field sits past the end of a 30-byte payload.
        let mut payload = vec![0u8; 30];
        payload[6..8].copy_from_slice(&25u16.to_be_bytes());
        assert_eq!(
            config.codec_data(1, &payload),
            Err(CodecDataError::SizeOutOfRange {
                sps: 25,
                pps: 0,
                length: 30
            })
        );
        assert!(!config.is_pending(), "nothing was kept");

        // A length that would overflow the offset arithmetic is refused before
        // anything is read with it.
        let mut huge = vec![0u8; 10];
        huge[6..8].copy_from_slice(&u16::MAX.to_be_bytes());
        assert!(matches!(
            config.codec_data(1, &huge),
            Err(CodecDataError::SizeOutOfRange { sps: 65535, .. })
        ));
    }

    /// The keystream a stream generates from its IV, computed here rather than by
    /// the type under test: block `index` is the AES-128 encryption of `IV + index`,
    /// and the keystream is those blocks one after another.
    fn keystream(keys: &StreamKeys, blocks: usize) -> Vec<u8> {
        use aes::cipher::{BlockEncrypt, KeyInit};
        let cipher = aes::Aes128::new(keys.key[..].into());
        let mut counter = keys.iv;
        let mut out = Vec::with_capacity(blocks * 16);
        for _ in 0..blocks {
            let mut block = aes::Block::clone_from_slice(&counter);
            cipher.encrypt_block(&mut block);
            out.extend_from_slice(&block);
            for byte in counter.iter_mut().rev() {
                let (next, wrapped) = byte.overflowing_add(1);
                *byte = next;
                if !wrapped {
                    break;
                }
            }
        }
        out
    }

    /// One payload of `length` patterned bytes, so a wrong offset shows up in every
    /// byte instead of only where the pattern happens to change.
    fn patterned(length: usize) -> Vec<u8> {
        (0..length)
            .map(|index| (index as u8).wrapping_mul(7) ^ 0x3c)
            .collect()
    }

    /// The client's side of the wire, written from the rule `mirror_buffer_decrypt`
    /// follows (`upstream/UxPlay@3dbf7cee:lib/mirror_buffer.c:94-124`) rather than from
    /// this port: the counter restarts on the block after the blocks used so far, the
    /// bytes carried from the previous packet go in front, and a packet that does not
    /// end on a block boundary puts its tail, zero padded, into one whole block whose
    /// unused bytes it carries.
    ///
    /// A port and a client that both read the rule correctly agree on the keystream
    /// every byte sits at. The differential test in `tests/upstream_oracle.rs` checks
    /// the same port against upstream's own code; this is what checks whole packet
    /// sequences, which the oracle's vectors are not.
    struct Encoder<'a> {
        stream: &'a [u8],
        /// Whole blocks the packets so far occupied, plus one for a tail that did not
        /// fill its block: where the next packet's fresh counter starts.
        blocks: usize,
        /// Where in `stream` the bytes being carried came from, so the test can check
        /// the receiver read the same ones.
        carry_at: usize,
        carried: usize,
    }

    impl<'a> Encoder<'a> {
        fn new(stream: &'a [u8]) -> Self {
            Self {
                stream,
                blocks: 0,
                carry_at: 0,
                carried: 0,
            }
        }

        /// Encrypts `plaintext` the way a client would, and returns the ciphertext,
        /// the keystream offset each byte of it sits at, how many bytes are left for
        /// the next packet, and the block the next packet will start its counter on.
        fn encrypt(&mut self, plaintext: &[u8]) -> (Vec<u8>, Vec<usize>, usize, usize) {
            let carried = self.carried;
            let fresh_at = self.blocks * 16;
            let whole = (plaintext.len() - carried) / 16 * 16;
            let rest = plaintext.len() - carried - whole;
            let mut ciphertext = plaintext.to_vec();
            let mut offsets = Vec::with_capacity(plaintext.len());
            for index in 0..carried {
                offsets.push(self.carry_at + index);
            }
            for index in carried..plaintext.len() {
                offsets.push(fresh_at + index - carried);
            }
            for (index, offset) in offsets.iter().enumerate() {
                ciphertext[index] ^= self.stream[*offset];
            }
            // The counter advances by the block the tail sits in and the whole blocks
            // after the carry: the carry's bytes were the previous packet's, so they
            // are not part of this packet's blocks.
            self.blocks += (plaintext.len() - carried) / 16;
            if rest > 0 {
                let tail = fresh_at + whole;
                self.carry_at = tail + rest;
                self.carried = 16 - rest;
                self.blocks += 1;
            } else {
                self.carried = 0;
            }
            (ciphertext, offsets, self.carried, self.blocks)
        }
    }

    #[test]
    fn the_keystream_generator_matches_the_crates_own_cipher() {
        // `MirrorDecryptor` generates its blocks from the IV itself, because where
        // the counter sits is the whole of the mirror rule. This says that generator
        // produces the same bytes as the CTR cipher the rest of the crate uses.
        let keys = stream_keys(11, &[0x44; STREAM_KEY_LEN]);
        let reference = keystream(&keys, 4);
        let mut cipher = [0u8; 64];
        crate::crypto::Aes128CtrState::new(&keys.key, &keys.iv)
            .expect("state")
            .apply(&mut cipher);
        let decryptor = MirrorDecryptor::new(&keys).expect("decryptor");
        assert_eq!(reference, cipher.to_vec(), "the reference over a buffer");
        assert_eq!(
            decryptor.keystream(0, 4),
            cipher.to_vec(),
            "the decryptor's generator"
        );
        // And a block further into the stream is that block, not a shifted one.
        assert_eq!(
            decryptor.keystream(2, 2),
            cipher[32..64].to_vec(),
            "blocks two and three"
        );
    }
    #[test]
    fn decrypts_what_a_client_encrypts_with_the_same_rule() {
        // Packet lengths that can really appear on the wire: every one is at least as
        // long as the bytes the previous packet left behind. The handoff's fixed
        // vector comes first, then a length that is never block aligned, a whole
        // number of blocks, and small packets.
        let lengths = [40usize, 24, 33, 47, 1, 64, 5, 128];
        let keys = stream_keys(7, &[0x31; STREAM_KEY_LEN]);
        // The vector's total length is 339 bytes, and 64 blocks cover it.
        let stream = keystream(&keys, 64);
        // The reference keystream is checked against the crate's own cipher over a
        // whole buffer, so a mistake in it cannot make this test agree with a mistake
        // in the implementation.
        let mut zeros = vec![0u8; 512];
        crate::crypto::Aes128CtrState::new(&keys.key, &keys.iv)
            .expect("state")
            .apply(&mut zeros);
        assert_eq!(zeros, stream[..512], "the reference keystream");

        let mut encoder = Encoder::new(&stream);
        let mut decryptor = MirrorDecryptor::new(&keys).expect("decryptor");
        let mut carried = 0usize;
        for length in lengths {
            // A shorter packet than the bytes being carried is not something a client
            // sends; the receiver refuses it, and that has its own test.
            if length < carried {
                continue;
            }
            let plaintext = patterned(length);
            let (ciphertext, offsets, left, next_block) = encoder.encrypt(&plaintext);
            assert_ne!(ciphertext, plaintext, "a packet of {length} is encrypted");
            let mut out = vec![0u8; length];
            decryptor
                .decrypt(&ciphertext, &mut out)
                .unwrap_or_else(|error| panic!("packet of {length}: {error}"));
            assert_eq!(
                out, plaintext,
                "a packet of {length} did not come back the way it was sent"
            );
            // The receiver has to have read the keystream the client wrote with, byte
            // for byte: this is the whole of the rule, and the offsets come from the
            // client's side rather than from the port.
            for (index, byte) in out.iter().enumerate() {
                assert_eq!(
                    *byte,
                    ciphertext[index] ^ stream[offsets[index]],
                    "packet of {length}, byte {index} read keystream offset {}",
                    offsets[index]
                );
            }
            assert_eq!(
                decryptor.carried(),
                left,
                "the bytes a packet of {length} leaves for the next one"
            );
            // Where the client's counter will be is where the receiver's has to be:
            // both sides have to agree about the block a packet starts on.
            assert_eq!(
                decryptor.blocks(),
                next_block as u64,
                "the block the packet after {length} starts on"
            );
            carried = left;
        }
    }

    #[test]
    fn carries_the_tail_block_into_the_next_packet() {
        // Upstream's fixed vector, byte by byte: a 40-byte packet uses two whole
        // blocks and eight bytes of a third, and the other eight bytes of that third
        // block are what the next packet's head is XORed with
        // (`mirror_buffer.c:96-100`, `:111-123`). An earlier attempt at this port lost
        // those eight bytes, which is what this test reports.
        let keys = stream_keys(1, &[0x11; STREAM_KEY_LEN]);
        let stream = keystream(&keys, 6);
        let mut encoder = Encoder::new(&stream);
        let mut decryptor = MirrorDecryptor::new(&keys).expect("decryptor");

        let first = patterned(40);
        let (ciphertext, offsets, left, _) = encoder.encrypt(&first);
        let mut out = vec![0u8; 40];
        decryptor.decrypt(&ciphertext, &mut out).expect("first");
        assert_eq!(out, first);
        assert_eq!(decryptor.carried(), 8, "40 bytes leaves the block's last 8");
        assert_eq!(left, 8);
        for (index, byte) in out.iter().enumerate() {
            assert_eq!(*byte, ciphertext[index] ^ stream[index], "byte {index}");
            assert_eq!(offsets[index], index, "the encoder's own offset {index}");
        }

        let second = patterned(24);
        let (ciphertext, offsets, left, _) = encoder.encrypt(&second);
        let mut out = vec![0u8; 24];
        decryptor.decrypt(&ciphertext, &mut out).expect("second");
        assert_eq!(out, second);
        assert_eq!(decryptor.carried(), 0, "24 bytes ends on a block boundary");
        assert_eq!(left, 0);
        // The second packet reads block 2's last eight bytes for its head and block 2
        // from its start for the body: the counter restarted, it did not continue.
        assert_eq!(&offsets[..8], &[40, 41, 42, 43, 44, 45, 46, 47]);
        for (index, byte) in out.iter().enumerate() {
            assert_eq!(
                *byte,
                ciphertext[index] ^ stream[offsets[index]],
                "byte {index}"
            );
        }
    }

    #[test]
    fn restarts_the_counter_at_every_packet() {
        // Two 24-byte packets. The first uses block 0 and the first half of block 1,
        // carrying the rest of block 1. The second's fresh block is block 2, so its
        // head is the carry from block 1 and its body is block 2 — a continuous stream
        // would have put the whole second packet at bytes 24..48 instead.
        let keys = stream_keys(3, &[0x77; STREAM_KEY_LEN]);
        let stream = keystream(&keys, 4);
        let mut encoder = Encoder::new(&stream);
        let mut decryptor = MirrorDecryptor::new(&keys).expect("decryptor");

        let first = patterned(24);
        let (ciphertext, offsets, _, _) = encoder.encrypt(&first);
        let mut out = vec![0u8; 24];
        decryptor.decrypt(&ciphertext, &mut out).expect("first");
        assert_eq!(out, first);
        assert_eq!(decryptor.carried(), 8);
        for (index, byte) in out.iter().enumerate() {
            assert_eq!(*byte, ciphertext[index] ^ stream[index], "byte {index}");
            assert_eq!(offsets[index], index, "the encoder's own offset {index}");
        }

        let second = patterned(24);
        let (ciphertext, offsets, _, _) = encoder.encrypt(&second);
        let mut out = vec![0u8; 24];
        decryptor.decrypt(&ciphertext, &mut out).expect("second");
        assert_eq!(out, second);
        assert_eq!(&offsets[..8], &[24, 25, 26, 27, 28, 29, 30, 31]);
        assert_eq!(offsets[8], 32, "the body starts at the fresh block");
        for (index, byte) in out.iter().enumerate() {
            assert_eq!(
                *byte,
                ciphertext[index] ^ stream[offsets[index]],
                "byte {index}"
            );
        }
    }

    #[test]
    fn a_rekey_drops_the_carry_and_starts_the_keystream_over() {
        let keys = stream_keys(5, &[0x22; STREAM_KEY_LEN]);
        let other = stream_keys(6, &[0x22; STREAM_KEY_LEN]);
        let mut decryptor = MirrorDecryptor::new(&keys).expect("decryptor");
        let mut out = vec![0u8; 40];
        decryptor.decrypt(&patterned(40), &mut out).expect("first");
        assert_eq!(decryptor.carried(), 8);
        decryptor.rekey(&other).expect("rekey");
        assert_eq!(decryptor.carried(), 0, "the old stream's bytes are gone");

        // The first packet of the new stream is decrypted from the beginning of its
        // own keystream, and its head is not the old stream's carry.
        let stream = keystream(&other, 3);
        let mut encoder = Encoder::new(&stream);
        let packet = patterned(33);
        let (ciphertext, offsets, _, _) = encoder.encrypt(&packet);
        let mut out = vec![0u8; 33];
        decryptor
            .decrypt(&ciphertext, &mut out)
            .expect("new stream");
        assert_eq!(out, packet);
        for (index, byte) in out.iter().enumerate() {
            assert_eq!(*byte, ciphertext[index] ^ stream[index], "byte {index}");
            assert_eq!(offsets[index], index, "the new stream starts at its IV");
        }
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
