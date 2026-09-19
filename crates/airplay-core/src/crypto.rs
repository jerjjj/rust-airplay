// SPDX-License-Identifier: GPL-3.0-or-later
//! RustCrypto-backed primitives used by AirPlay pairing and media decryption.
//!
//! Protocol code chooses the exact mode, IV length, and key derivation labels.
//! These helpers intentionally do not silently pad, truncate, or normalize input.

#![allow(missing_docs)]

use aes::Aes128;
use aes_gcm::{
    AesGcm, Nonce, Tag,
    aead::{AeadInPlace, KeyInit},
};
use cbc::Decryptor;
use cipher::{BlockDecryptMut, KeyIvInit, StreamCipher, block_padding::NoPadding};
use ctr::Ctr128BE;
use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha512};
use x25519_dalek::{X25519_BASEPOINT_BYTES, x25519};

type Aes128Ctr = Ctr128BE<Aes128>;
type Aes128Gcm16 = AesGcm<Aes128, cipher::consts::U16>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    InvalidLength(&'static str),
    InvalidKey,
    AuthenticationFailed,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLength(name) => write!(f, "invalid length for {name}"),
            Self::InvalidKey => f.write_str("invalid key or IV"),
            Self::AuthenticationFailed => f.write_str("AEAD authentication failed"),
        }
    }
}
impl std::error::Error for CryptoError {}

pub fn sha512_concat(parts: &[&[u8]]) -> [u8; 64] {
    let mut hasher = Sha512::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

pub fn sha1_concat(parts: &[&[u8]]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

pub fn md5_concat(parts: &[&[u8]]) -> [u8; 16] {
    let mut hasher = Md5::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

pub fn sha512_derive(parts: &[&[u8]], output_len: usize) -> Result<Vec<u8>, CryptoError> {
    if output_len > 64 {
        return Err(CryptoError::InvalidLength("SHA-512 output"));
    }
    Ok(sha512_concat(parts)[..output_len].to_vec())
}

/// AES-CTR with the OpenSSL EVP counter convention used by UxPlay.
#[derive(Clone)]
pub struct Aes128CtrState {
    cipher: Aes128Ctr,
    key: [u8; 16],
    iv: [u8; 16],
}

impl Aes128CtrState {
    pub fn new(key: &[u8], iv: &[u8]) -> Result<Self, CryptoError> {
        let key: [u8; 16] = key
            .try_into()
            .map_err(|_| CryptoError::InvalidLength("AES-128 key"))?;
        let iv: [u8; 16] = iv
            .try_into()
            .map_err(|_| CryptoError::InvalidLength("AES-CTR IV"))?;
        let cipher = Aes128Ctr::new_from_slices(&key, &iv).map_err(|_| CryptoError::InvalidKey)?;
        Ok(Self { cipher, key, iv })
    }

    pub fn apply(&mut self, data: &mut [u8]) {
        self.cipher.apply_keystream(data);
    }

    pub fn reset(&mut self) -> Result<(), CryptoError> {
        self.cipher =
            Aes128Ctr::new_from_slices(&self.key, &self.iv).map_err(|_| CryptoError::InvalidKey)?;
        Ok(())
    }
}

pub fn aes128_cbc_decrypt(key: &[u8], iv: &[u8], input: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if input.len() % 16 != 0 {
        return Err(CryptoError::InvalidLength("AES-CBC blocks"));
    }
    let mut output = input.to_vec();
    let decrypted = Decryptor::<Aes128>::new_from_slices(key, iv)
        .map_err(|_| CryptoError::InvalidKey)?
        .decrypt_padded_mut::<NoPadding>(&mut output)
        .map_err(|_| CryptoError::InvalidLength("AES-CBC blocks"))?;
    Ok(decrypted.to_vec())
}

pub fn aes128_gcm16_decrypt(
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let key = aes_gcm::Key::<Aes128Gcm16>::from_slice(key);
    let cipher = Aes128Gcm16::new(key);
    let nonce = Nonce::<cipher::consts::U16>::from_slice(iv);
    let tag = Tag::<cipher::consts::U16>::from_slice(tag);
    let mut plaintext = ciphertext.to_vec();
    cipher
        .decrypt_in_place_detached(nonce, b"", &mut plaintext, tag)
        .map_err(|_| CryptoError::AuthenticationFailed)?;
    Ok(plaintext)
}

pub fn x25519_public(private_key: &[u8; 32]) -> [u8; 32] {
    x25519(*private_key, X25519_BASEPOINT_BYTES)
}

pub fn x25519_shared(private_key: &[u8; 32], peer_public: &[u8; 32]) -> [u8; 32] {
    x25519(*private_key, *peer_public)
}

pub fn ed25519_sign(private_key: &[u8; 32], message: &[u8]) -> [u8; 64] {
    use ed25519_dalek::{Signer, SigningKey};
    SigningKey::from_bytes(private_key).sign(message).to_bytes()
}

pub fn ed25519_verify(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let Ok(key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    key.verify(message, &Signature::from_bytes(signature))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_known_vectors() {
        assert_eq!(
            sha1_concat(&[b"abc"]),
            [
                0xa9, 0x99, 0x3e, 0x36, 0x47, 0x06, 0x81, 0x6a, 0xba, 0x3e, 0x25, 0x71, 0x78, 0x50,
                0xc2, 0x6c, 0x9c, 0xd0, 0xd8, 0x9d
            ]
            .map(|x| x)
        );
        assert_eq!(
            md5_concat(&[b"abc"]),
            [
                0x90, 0x01, 0x50, 0x98, 0x3c, 0xd2, 0x4f, 0xb0, 0xd6, 0x96, 0x3f, 0x7d, 0x28, 0xe1,
                0x7f, 0x72
            ]
        );
    }

    #[test]
    fn ctr_is_streaming_and_resettable() {
        let mut first = Aes128CtrState::new(&[0; 16], &[0; 16]).unwrap();
        let mut second = Aes128CtrState::new(&[0; 16], &[0; 16]).unwrap();
        let mut all = [0u8; 32];
        let mut split = [0u8; 32];
        first.apply(&mut all);
        second.apply(&mut split[..7]);
        second.apply(&mut split[7..]);
        assert_eq!(all, split);
        second.reset().unwrap();
        let mut reset = [0u8; 32];
        second.apply(&mut reset);
        assert_eq!(all, reset);
    }

    #[test]
    fn matches_aes_ctr_and_cbc_vectors() {
        let key = [
            0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf,
            0x4f, 0x3c,
        ];
        let iv = [
            0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd,
            0xfe, 0xff,
        ];
        let mut ctr = Aes128CtrState::new(&key, &iv).unwrap();
        let mut block = [
            0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93,
            0x17, 0x2a,
        ];
        ctr.apply(&mut block);
        assert_eq!(
            block,
            [
                0x87, 0x4d, 0x61, 0x91, 0xb6, 0x20, 0xe3, 0x26, 0x1b, 0xef, 0x68, 0x64, 0x99, 0x0d,
                0xb6, 0xce
            ]
        );
        let cbc_iv = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        let ciphertext = [
            0x76, 0x49, 0xab, 0xac, 0x81, 0x19, 0xb2, 0x46, 0xce, 0xe9, 0x8e, 0x9b, 0x12, 0xe9,
            0x19, 0x7d,
        ];
        assert_eq!(
            aes128_cbc_decrypt(&key, &cbc_iv, &ciphertext).unwrap(),
            vec![
                0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93,
                0x17, 0x2a
            ]
        );
    }

    #[test]
    fn ed25519_round_trip() {
        let private = [7u8; 32];
        let public = ed25519_dalek::SigningKey::from_bytes(&private)
            .verifying_key()
            .to_bytes();
        let signature = ed25519_sign(&private, b"airplay");
        assert!(ed25519_verify(&public, b"airplay", &signature));
        assert!(!ed25519_verify(&public, b"tampered", &signature));
    }
}
