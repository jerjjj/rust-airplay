//! AES-CBC audio stream decryption — aligned with java-airplay FairPlayAudioDecryptor.
//!
//! Key derivation:
//!   eaesKey = SHA-512(aesKey || sharedSecret)[0..16]
//!
//! Decryption:
//!   AES-CBC(eaesKey, eiv) — re-initialized for EACH decrypt() call!
//!   Java: initAesCbcCipher() is called inside decrypt(), resetting the IV each time.
//!
//! Only decrypts 16-byte aligned portions of the audio data.
//! Trailing bytes (< 16) are left untouched.

use aes::Aes128;
use cbc::cipher::{KeyIvInit, BlockDecryptMut};

type Aes128Cbc = cbc::Decryptor<Aes128>;

/// Audio stream decryptor.
pub struct AudioDecryptor {
    /// AES key (16 bytes) — cached for re-init on each decrypt call.
    eaes_key: [u8; 16],
    /// AES IV (16 bytes) — cached for re-init on each decrypt call.
    iv: [u8; 16],
    /// Whether the key/iv have been set.
    initialized: bool,
}

impl AudioDecryptor {
    pub fn new() -> Self {
        Self {
            eaes_key: [0u8; 16],
            iv: [0u8; 16],
            initialized: false,
        }
    }

    /// Reset state — clear key/iv so decrypt is a no-op until next init().
    pub fn reset(&mut self) {
        self.initialized = false;
    }

    /// Initialize with the AES key, shared secret, and eiv.
    /// Derives eaesKey and caches key+iv for per-decrypt cipher construction.
    pub fn init(&mut self, aes_key: &[u8; 16], shared_secret: &[u8; 32], eiv: &[u8; 16]) {
        use sha2::{Sha512, Digest};

        // eaesKey = SHA-512(aesKey || sharedSecret)[0..16]
        let mut hasher = Sha512::new();
        hasher.update(aes_key);
        hasher.update(shared_secret);
        let result = hasher.finalize();

        self.eaes_key.copy_from_slice(&result[..16]);
        self.iv.copy_from_slice(eiv);
        self.initialized = true;
    }

    /// Decrypt audio data in-place.
    /// Java: re-initializes the AES-CBC cipher before each decrypt() call.
    /// This means each audio packet starts with a fresh IV (no chaining across packets).
    /// Within a single packet, CBC chains blocks normally.
    /// Only processes 16-byte aligned prefix; trailing bytes are untouched.
    pub fn decrypt(&mut self, data: &mut [u8]) {
        if !self.initialized {
            return;
        }

        let aligned_len = (data.len() / 16) * 16;
        if aligned_len == 0 {
            return;
        }

        // Java: initAesCbcCipher() — creates a FRESH cipher per decrypt() call.
        // This resets the IV for each audio packet but CBC chains within the packet.
        let mut cipher = Aes128Cbc::new(&self.eaes_key.into(), &self.iv.into());

        // Decrypt each 16-byte block in sequence (CBC chains blocks)
        for chunk in data[..aligned_len].chunks_mut(16) {
            let mut block = aes::Block::clone_from_slice(chunk);
            cipher.decrypt_block_mut(&mut block);
            chunk.copy_from_slice(&block);
        }
    }
}
