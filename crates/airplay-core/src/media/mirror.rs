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

#[cfg(test)]
mod tests {
    use super::*;

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
