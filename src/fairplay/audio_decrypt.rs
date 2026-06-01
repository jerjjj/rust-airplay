//! AES-CBC audio stream decryption.
//!
//! Translated from FairPlayAudioDecryptor.java.
//!
//! Key derivation (simplified vs video):
//!   eaesKey = SHA-512(aesKey || sharedSecret)[0..16]
//!
//! Decryption:
//!   cipher = AES-CBC(eaesKey, eiv)
//!   Only decrypts 16-byte aligned portions of the audio data.

use aes::Aes128;
use cbc::cipher::{KeyIvInit, BlockDecryptMut};

type Aes128Cbc = cbc::Decryptor<Aes128>;

/// Audio stream decryptor.
pub struct AudioDecryptor {
    cipher: Option<Aes128Cbc>,
}

impl AudioDecryptor {
    pub fn new() -> Self {
        Self { cipher: None }
    }

    /// Initialize with the AES key, shared secret, and eiv.
    pub fn init(&mut self, aes_key: &[u8; 16], shared_secret: &[u8; 32], eiv: &[u8; 16]) {
        use sha2::{Sha512, Digest};

        // eaesKey = SHA-512(aesKey || sharedSecret)[0..16]
        let mut hasher = Sha512::new();
        hasher.update(aes_key);
        hasher.update(shared_secret);
        let result = hasher.finalize();
        let mut eaes_key = [0u8; 16];
        eaes_key.copy_from_slice(&result[..16]);

        let mut iv = [0u8; 16];
        iv.copy_from_slice(eiv);

        self.cipher = Some(Aes128Cbc::new(&eaes_key.into(), &iv.into()));
    }

    /// Decrypt audio data in-place.
    /// Only processes 16-byte aligned prefix; trailing bytes are untouched.
    pub fn decrypt(&mut self, data: &mut [u8]) {
        let cipher = match &mut self.cipher {
            Some(c) => c,
            None => return,
        };

        let aligned_len = data.len() / 16 * 16;
        if aligned_len > 0 {
            // AES-CBC decrypt each 16-byte block
            for chunk in data[..aligned_len].chunks_mut(16) {
                // Decrypt each 16-byte block
                let mut block = aes::Block::clone_from_slice(chunk);
                cipher.decrypt_block_mut(&mut block);
                chunk.copy_from_slice(&block);
            }
        }
    }
}

impl Default for AudioDecryptor {
    fn default() -> Self {
        Self::new()
    }
}
