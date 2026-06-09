//! AES-CTR video stream decryption — aligned with java-airplay FairPlayVideoDecryptor.
//!
//! Key derivation:
//!   eaesKey = SHA-512(aesKey || sharedSecret)[0..16]
//!   key     = SHA-512("AirPlayStreamKey" + connID || eaesKey)[0..16]
//!   iv      = SHA-512("AirPlayStreamIV"  + connID || eaesKey)[0..16]
//!
//! Handles partial-block CTR overflow using og[16] buffer + next_decrypt_count,
//! exactly matching java-airplay FairPlayVideoDecryptor.decrypt().

use aes::Aes128;
use ctr::cipher::{KeyIvInit, StreamCipher};
use sha2::{Digest, Sha512};

type Aes128Ctr = ctr::Ctr128BE<Aes128>;

pub struct VideoDecryptor {
    pub cipher: Option<Aes128Ctr>,
    og: [u8; 16],
    next_decrypt_count: usize,
}

impl VideoDecryptor {
    pub fn new() -> Self {
        Self { cipher: None, og: [0u8; 16], next_decrypt_count: 0 }
    }

    pub fn init(&mut self, aes_key: &[u8; 16], shared_secret: &[u8; 32], stream_connection_id: i64) {
        let mut h = Sha512::new();
        h.update(aes_key); h.update(shared_secret);
        let eaes = h.finalize();
        let conn_str = (stream_connection_id as u64).to_string();

        let mut h = Sha512::new();
        h.update(format!("AirPlayStreamKey{}", conn_str).as_bytes());
        h.update(&eaes[..16]);
        let dk = h.finalize();

        let mut h = Sha512::new();
        h.update(format!("AirPlayStreamIV{}", conn_str).as_bytes());
        h.update(&eaes[..16]);
        let di = h.finalize();

        let mut key = [0u8; 16]; let mut iv = [0u8; 16];
        key.copy_from_slice(&dk[..16]); iv.copy_from_slice(&di[..16]);
        self.cipher = Some(Aes128Ctr::new(&key.into(), &iv.into()));
        self.next_decrypt_count = 0; self.og = [0u8; 16];
        tracing::info!("Video KDF: conn={} key={:02x?} iv={:02x?}", stream_connection_id, &key, &iv);
    }

    pub fn decrypt(&mut self, payload: &mut [u8]) {
        let cipher = match &mut self.cipher { Some(c) => c, None => return };
        let total = payload.len();
        let mut off = 0usize;

        if self.next_decrypt_count > 0 {
            let n = self.next_decrypt_count.min(total);
            for i in 0..n { payload[i] ^= self.og[16-self.next_decrypt_count+i]; }
            self.next_decrypt_count -= n; off = n;
        }
        if off >= total { return; }
        let rem = total - off;
        let aligned = rem / 16 * 16;
        if aligned > 0 { cipher.apply_keystream(&mut payload[off..off+aligned]); off += aligned; }
        let rest = total - off;
        if rest > 0 {
            self.og = [0u8; 16]; self.og[..rest].copy_from_slice(&payload[off..]);
            cipher.apply_keystream(&mut self.og);
            payload[off..].copy_from_slice(&self.og[..rest]);
            self.next_decrypt_count = 16 - rest;
        } else { self.next_decrypt_count = 0; }
    }
}

impl Default for VideoDecryptor {
    fn default() -> Self { Self::new() }
}
