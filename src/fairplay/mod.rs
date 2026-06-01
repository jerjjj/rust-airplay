//! FairPlay DRM module.
//!
//! Handles the 3-round fp-setup protocol and decrypts video/audio stream keys
//! using the reverse-engineered OmgHax algorithm.
//!
//! Based on:
//!   https://github.com/serezhka/java-airplay/blob/main/lib/src/main/java/com/github/serezhka/airplay/lib/internal/FairPlay.java

pub mod audio_decrypt;
pub mod hand_garble;
pub mod modified_md5;
pub mod omghax;
pub mod omghax_const;
pub mod playfair_ffi;
pub mod sap_hash;
pub mod video_decrypt;
#[cfg(test)] mod omghax_test;
#[cfg(test)] mod omghax_debug;

use crate::session::Session;

/// FairPlay handler that manages the fp-setup state machine.
pub struct FairPlayHandler;

impl FairPlayHandler {
    pub fn new() -> Self {
        Self
    }

    /// Process one fp-setup message from the client.
    /// Returns the response bytes to send back (empty for no content).
    pub fn handle_message(&self, data: &[u8], session: &mut Session) -> Vec<u8> {
        if data.len() < 14 {
            return vec![];
        }

        // Parse FairPlay header
        let magic = &data[0..4];
        if magic != crate::rtsp::types::FPLY_MAGIC {
            return vec![];
        }

        // Check version
        if data[4] != 3 {
            tracing::error!("FairPlay version {} not supported!", data[4]);
            return vec![];
        }

        if data.len() == 16 {
            // Round 1: Client sends mode selection
            let mode = data[14] as usize;
            tracing::info!("FairPlay round 1: mode={}", mode);
            let reply = get_reply_message(mode);
            tracing::info!(
                "FairPlay round 1 reply: {} bytes, first 16: {:02x?}",
                reply.len(),
                &reply[..16.min(reply.len())]
            );
            reply.to_vec()
        } else if data.len() == 164 {
            // Round 2: Client sends 164-byte key message
            tracing::info!("FairPlay round 2: received 164-byte keyMsg");
            let resp = self.handle_fp_setup2(data, session);
            tracing::info!(
                "FairPlay round 2 reply: {} bytes",
                resp.len()
            );
            resp
        } else {
            tracing::warn!("FairPlay: unknown message length {}", data.len());
            vec![]
        }
    }

    fn handle_fp_setup2(&self, data: &[u8], session: &mut Session) -> Vec<u8> {
        // Save the full 164-byte key message for later decryption
        session.key_msg = Some(data[..164].to_vec());

        // Return FPLY header + last 20 bytes of the message
        let mut response = Vec::with_capacity(34);
        let fp_header: [u8; 12] = [70, 80, 76, 89, 3, 1, 4, 0, 0, 0, 0, 20];
        response.extend_from_slice(&fp_header);
        response.extend_from_slice(&data[144..164]);
        response
    }

    /// Decrypt the AES stream key from the keyMsg and ekey.
    pub fn decrypt_aes_key(&self, key_msg: &[u8], ekey: &[u8]) -> [u8; 16] {
        // Use verified C implementation from UxPlay
        let mut out = [0u8; 16];
        unsafe {
            crate::fairplay::playfair_ffi::playfair_decrypt(
                key_msg.as_ptr(),
                ekey.as_ptr(),
                out.as_mut_ptr(),
            );
        }
        out
    }
}

/// Hardcoded reply messages for fp-setup round 1 (4 modes).
/// Extracted from FairPlay.java replyMessage[][].
fn get_reply_message(mode: usize) -> &'static [u8] {
    const REPLY_0: &[u8] = &[
        70, 80, 76, 89, 3, 1, 2, 0, 0, 0, 0, 0x82, 2, 0, 15, 0x9F,
        63, 0x9E, 10, 37, 33, 0xDB, 0xDF, 49, 42, 0xB2, 0xBF, 0xB2, 0x9E, 0x8D, 35, 43,
        99, 118, 0xA8, 0xC8, 24, 112, 29, 34, 0xAE, 0x93, 0xD8, 39, 55, 0xFE, 0xAF, 0x9D,
        0xB4, 0xFD, 0xF4, 28, 45, 0xBA, 0x9D, 31, 73, 0xCA, 0xAA, 0xBF, 101, 0x91, 0xAC,
        31, 123, 0xC6, 0xF7, 0xE0, 102, 61, 33, 0xAF, 0xE0, 21, 101, 0x95, 62, 0xAB,
        0x81, 0xF4, 24, 0xCE, 0xED, 9, 90, 0xDB, 124, 61, 14, 37, 73, 9, 0xA7, 0x98,
        49, 0xD4, 0x9C, 57, 0x82, 0x95, 52, 52, 0xFA, 0xCB, 66, 0xC6, 58, 28, 0xD9,
        17, 0xA6, 0xFE, 0x94, 26, 0x8A, 109, 74, 116, 59, 70, 0xC3, 0xA7, 100, 0x9E,
        68, 0xC7, 0x89, 85, 0xE4, 0x9D, 0x81, 85, 0, 0x95, 73, 0xC4, 0xE2, 0xF7, 0xA3,
        0xF6, 0xD5, 0xBA,
    ];

    const REPLY_1: &[u8] = &[
        70, 80, 76, 89, 3, 1, 2, 0, 0, 0, 0, 0x82, 2, 1, 0xCF, 50,
        0xA2, 87, 20, 0xB2, 82, 79, 0x8A, 0xA0, 0xAD, 122, 0xF1, 100, 0xE3, 123, 0xCF, 68,
        36, 0xE2, 0, 4, 126, 0xFC, 10, 0xD6, 122, 0xFC, 0xD9, 93, 0xED, 28, 39, 48,
        0xBB, 89, 27, 0x96, 46, 0xD6, 58, 0x9C, 77, 0xED, 0x88, 0xBA, 0x8F, 0xC7, 0x8D,
        0xE6, 77, 0x91, 0xCC, 0xFD, 92, 123, 86, 0xDA, 0x88, 0xE3, 31, 92, 0xCE, 0xAF,
        0xC7, 67, 25, 0x95, 0xA0, 22, 101, 0xA5, 78, 25, 57, 0xD2, 91, 0x94, 0xDB,
        100, 0xB9, 0xE4, 93, 0x8D, 6, 62, 30, 106, 0xF0, 126, 0x96, 86, 22, 43, 14,
        0xFA, 64, 66, 117, 0xEA, 90, 68, 0xD9, 89, 28, 114, 86, 0xB9, 0xFB, 0xE6,
        81, 56, 0x98, 0xB8, 2, 39, 114, 25, 0x88, 87, 22, 80, 0x94, 42, 0xD9, 70,
        104, 0x8A,
    ];

    const REPLY_2: &[u8] = &[
        70, 80, 76, 89, 3, 1, 2, 0, 0, 0, 0, 0x82, 2, 2, 0xC1, 105,
        0xA3, 82, 0xEE, 0xED, 53, 0xB1, 0x8C, 0xDD, 0x9C, 88, 0xD6, 79, 22, 0xC1, 81, 0x9A,
        0x89, 0xEB, 83, 23, 0xBD, 13, 67, 54, 0xCD, 104, 0xF6, 56, 0xFF, 0x9D, 1, 106,
        91, 82, 0xB7, 0xFA, 0x92, 22, 0xB2, 0xB6, 84, 0x82, 0xC7, 0x84, 68, 17, 0x81, 33,
        0xA2, 0xC7, 0xFE, 0xD8, 61, 0xB7, 17, 0x9E, 0x91, 0x82, 0xAA, 0xD7, 0xD1, 0x8C,
        112, 99, 0xE2, 0xA4, 87, 85, 89, 16, 0xAF, 0x9E, 14, 0xFC, 118, 52, 125, 22,
        64, 67, 0x80, 127, 88, 30, 0xE4, 0xFB, 0xE4, 44, 0xA9, 0xDE, 0xDC, 27, 94, 0xB2,
        0xA3, 0xAA, 61, 46, 0xCD, 89, 0xE7, 0xEE, 0xE7, 11, 54, 41, 0xF2, 42, 0xFD,
        22, 29, 0x87, 115, 83, 0xDD, 0xB9, 0x9A, 0xDC, 0x8E, 7, 0, 110, 86, 0xF8,
        80, 0xCE,
    ];

    const REPLY_3: &[u8] = &[
        70, 80, 76, 89, 3, 1, 2, 0, 0, 0, 0, 0x82, 2, 3, 0x90, 1,
        0xE1, 114, 126, 15, 87, 0xF7, 0xF5, 0x88, 13, 0xB1, 4, 0xA6, 37, 122, 35, 0xF5,
        0xCF, 0xFF, 26, 0xBB, 0xE1, 0xE9, 48, 69, 37, 26, 0xFB, 0x97, 0xEB, 0x9F, 0xC0,
        1, 30, 0xBE, 15, 58, 0x81, 0xDF, 91, 105, 29, 118, 0xAC, 0xB2, 0xF7, 0xA5,
        0xC7, 8, 0xE3, 0xD3, 40, 0xF5, 107, 0xB3, 0x9D, 0xBD, 0xE7, 0xF2, 0x9C, 0x8A,
        23, 0xF4, 0x81, 72, 126, 58, 0xE8, 99, 0xC6, 120, 50, 84, 34, 0xE6, 0xF7, 0x8E,
        22, 109, 24, 0xAA, 127, 0xD6, 54, 37, 0x8B, 0xCE, 40, 114, 111, 102, 31, 115,
        0x88, 0x93, 0xCE, 68, 49, 30, 75, 0xE6, 0xC0, 83, 81, 0x93, 0xE7, 0xEF, 114,
        0xE8, 104, 98, 51, 114, 0x9C, 34, 125, 0x82, 12, 0x97, 0x94, 69, 0xD8, 0x92,
        70, 0xC8, 0xC3, 89,
    ];

    match mode {
        0 => REPLY_0,
        1 => REPLY_1,
        2 => REPLY_2,
        3 => REPLY_3,
        _ => REPLY_3,
    }
}
