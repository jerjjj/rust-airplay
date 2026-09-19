// SPDX-License-Identifier: GPL-3.0-or-later
//! Apple-labelled key derivation and pairing transcript helpers.

#![allow(missing_docs)]

use crate::crypto::{Aes128CtrState, CryptoError, sha512_concat};

pub const X25519_KEY_SIZE: usize = 32;
pub const ED25519_KEY_SIZE: usize = 32;
pub const PAIRING_SIG_SIZE: usize = 64;

pub struct PairVerifyKeys {
    pub key: [u8; 16],
    pub iv: [u8; 16],
}

pub fn derive_pair_verify_keys(shared_secret: &[u8; X25519_KEY_SIZE]) -> PairVerifyKeys {
    PairVerifyKeys {
        key: sha512_concat(&[b"Pair-Verify-AES-Key", shared_secret])[..16]
            .try_into()
            .unwrap(),
        iv: sha512_concat(&[b"Pair-Verify-AES-IV", shared_secret])[..16]
            .try_into()
            .unwrap(),
    }
}

pub fn derive_pair_setup_keys(session_key: &[u8]) -> PairVerifyKeys {
    let key_hash = sha512_concat(&[b"Pair-Setup-AES-Key", session_key]);
    let iv_hash = sha512_concat(&[b"Pair-Setup-AES-IV", session_key]);
    PairVerifyKeys {
        key: key_hash[..16].try_into().unwrap(),
        iv: iv_hash[..16].try_into().unwrap(),
    }
}

pub fn derive_stream_material(
    audio_key: &[u8; 16],
    stream_connection_id: u64,
) -> ([u8; 16], [u8; 16]) {
    let key_label = format!("AirPlayStreamKey{stream_connection_id}");
    let iv_label = format!("AirPlayStreamIV{stream_connection_id}");
    let key_hash = sha512_concat(&[key_label.as_bytes(), audio_key]);
    let iv_hash = sha512_concat(&[iv_label.as_bytes(), audio_key]);
    (
        key_hash[..16].try_into().unwrap(),
        iv_hash[..16].try_into().unwrap(),
    )
}

pub fn encrypt_pair_verify_signature(
    private_key: &[u8; 32],
    our_ecdh_public: &[u8; 32],
    peer_ecdh_public: &[u8; 32],
    shared_secret: &[u8; 32],
) -> Result<[u8; PAIRING_SIG_SIZE], CryptoError> {
    let message = [*our_ecdh_public, *peer_ecdh_public].concat();
    let mut signature = crate::crypto::ed25519_sign(private_key, &message);
    let keys = derive_pair_verify_keys(shared_secret);
    let mut cipher = Aes128CtrState::new(&keys.key, &keys.iv)?;
    cipher.apply(&mut signature);
    Ok(signature)
}

pub fn decrypt_pair_verify_signature(
    encrypted_signature: &[u8; PAIRING_SIG_SIZE],
    shared_secret: &[u8; 32],
) -> Result<[u8; PAIRING_SIG_SIZE], CryptoError> {
    let keys = derive_pair_verify_keys(shared_secret);
    let mut plain = *encrypted_signature;
    let mut cipher = Aes128CtrState::new(&keys.key, &keys.iv)?;
    cipher.apply(&mut plain);
    Ok(plain)
}

pub fn verify_pair_signature(
    signature: &[u8; PAIRING_SIG_SIZE],
    peer_ecdh_public: &[u8; 32],
    our_ecdh_public: &[u8; 32],
    peer_ed25519_public: &[u8; 32],
) -> bool {
    let message = [*peer_ecdh_public, *our_ecdh_public].concat();
    crate::crypto::ed25519_verify(peer_ed25519_public, &message, signature)
}

pub fn md5_hex(input: &[u8]) -> String {
    crate::crypto::md5_concat(&[input])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn digest_auth_response(
    method: &str,
    username: &str,
    realm: &str,
    password: &str,
    nonce: &str,
    uri: &str,
    qop: Option<&str>,
    nonce_count: Option<&str>,
    client_nonce: Option<&str>,
) -> String {
    let ha1 = md5_hex(format!("{username}:{realm}:{password}").as_bytes());
    let ha2 = md5_hex(format!("{method}:{uri}").as_bytes());
    let tail = match (qop, nonce_count, client_nonce) {
        (Some(qop), Some(nc), Some(cnonce)) => format!("{nonce}:{nc}:{cnonce}:{qop}:{ha2}"),
        _ => format!("{nonce}:{ha2}"),
    };
    md5_hex(format!("{ha1}:{tail}").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derives_distinct_labelled_material() {
        let secret = [7u8; 32];
        let keys = derive_pair_verify_keys(&secret);
        assert_ne!(keys.key, keys.iv);
        assert_ne!(keys.key, derive_pair_setup_keys(&secret).key);
    }
    #[test]
    fn stream_material_is_deterministic() {
        let (key_a, iv_a) = derive_stream_material(&[1; 16], 42);
        let (key_b, iv_b) = derive_stream_material(&[1; 16], 42);
        assert_eq!((key_a, iv_a), (key_b, iv_b));
        assert_ne!((key_a, iv_a), derive_stream_material(&[1; 16], 43));
    }
    #[test]
    fn digest_auth_matches_rfc_example() {
        let response = digest_auth_response(
            "GET",
            "Mufasa",
            "testrealm@host.com",
            "Circle Of Life",
            "dcd98b7102dd2f0e8b11d0f600bfb0c093",
            "/dir/index.html",
            Some("auth"),
            Some("00000001"),
            Some("0a4f113b"),
        );
        assert_eq!(response, "6629fae49393a05397450978507c4ef1");
    }
}
