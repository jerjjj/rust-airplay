//! AES-CTR video stream decryption — simplified.

use aes::Aes128;
use ctr::cipher::{KeyIvInit, StreamCipher};
use sha2::{Digest, Sha512};

type Aes128Ctr = ctr::Ctr128BE<Aes128>;

pub struct VideoDecryptor {
    pub cipher: Option<Aes128Ctr>,
}

impl VideoDecryptor {
    pub fn new() -> Self { Self { cipher: None } }

    pub fn init(&mut self, aes_key: &[u8; 16], shared_secret: &[u8; 32], stream_connection_id: i64) {
        let mut h = Sha512::new();
        h.update(aes_key);
        h.update(shared_secret);
        let eaes = h.finalize();

        // Java: Long.toUnsignedString(streamConnectionID) — signed → unsigned decimal
        let conn_str = (stream_connection_id as u64).to_string();
        let prefix = format!("AirPlayStreamKey{}", conn_str);
        let mut h = Sha512::new();
        h.update(prefix.as_bytes());
        h.update(&eaes[..16]);
        let dk = h.finalize();

        let prefix = format!("AirPlayStreamIV{}", conn_str);
        let mut h = Sha512::new();
        h.update(prefix.as_bytes());
        h.update(&eaes[..16]);
        let di = h.finalize();

        let mut key = [0u8; 16];
        let mut iv = [0u8; 16];
        key.copy_from_slice(&dk[..16]);
        iv.copy_from_slice(&di[..16]);
        self.cipher = Some(Aes128Ctr::new(&key.into(), &iv.into()));
        tracing::warn!("Video KDF: aes={:02x?} ss={:02x?}... conn={} key={:02x?} iv={:02x?}",
            aes_key, &shared_secret[..8], stream_connection_id, &key, &iv);
    }

    pub fn decrypt(&mut self, payload: &mut [u8]) {
        if let Some(ref mut c) = self.cipher {
            c.apply_keystream(payload);
        }
    }
}

impl Default for VideoDecryptor {
    fn default() -> Self { Self::new() }
}
