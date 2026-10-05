// SPDX-License-Identifier: GPL-3.0-or-later
//! Apple-labelled key derivation and pairing transcript helpers.
//!
//! Each label in this module is a string upstream hashes with SHA-512 together
//! with a shared secret (`upstream/UxPlay/lib/pairing.c:29-30`, `:600-614`), so
//! the module maps one protocol step to one derivation instead of hiding them
//! behind a generic key schedule.

use crate::crypto::{Aes128CtrState, CryptoError, sha512_concat};

/// Length of an X25519 public key or private scalar.
pub const X25519_KEY_SIZE: usize = 32;

/// Length of an Ed25519 public key or private seed.
pub const ED25519_KEY_SIZE: usize = 32;

/// Length of the signature both sides of `/pair-verify` exchange.
pub const PAIRING_SIG_SIZE: usize = 64;

/// AES key and IV one pairing step derives from a shared secret.
///
/// Deliberately without `Debug`: the two fields are secret key material, and a
/// derived `Debug` would print them into any log that formats the value.
pub struct PairVerifyKeys {
    /// AES-128 key.
    pub key: [u8; 16],
    /// Initial counter block.
    pub iv: [u8; 16],
}

/// Key and IV of `/pair-verify`, derived from the X25519 shared secret
/// (`upstream/UxPlay/lib/pairing.c:68-85`).
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

/// Key and IV of `/pair-setup-pin` step 3, derived from the SRP session key
/// (`upstream/UxPlay/lib/pairing.c:600-614`).
pub fn derive_pair_setup_keys(session_key: &[u8]) -> PairVerifyKeys {
    let key_hash = sha512_concat(&[b"Pair-Setup-AES-Key", session_key]);
    let iv_hash = sha512_concat(&[b"Pair-Setup-AES-IV", session_key]);
    PairVerifyKeys {
        key: key_hash[..16].try_into().unwrap(),
        iv: iv_hash[..16].try_into().unwrap(),
    }
}

/// Pair-Setup AES-GCM nonces of `/pair-setup-pin` step 3, decrypt first.
///
/// Both directions derive one base IV from the `Pair-Setup-AES-IV` label and the
/// SRP session key. The client's key exchange key is decrypted with the base IV
/// incremented by one and the reply is encrypted with it incremented by two
/// (`upstream/UxPlay/lib/pairing.c:608-615` and `:635-637`, where the second
/// increment is the undocumented nonce adjustment). Neither nonce travels on the
/// wire.
pub fn pair_setup_gcm_nonces(session_key: &[u8]) -> ([u8; 16], [u8; 16]) {
    let base = derive_pair_setup_keys(session_key).iv;
    let mut decrypt = base;
    decrypt[15] = decrypt[15].wrapping_add(1);
    let mut encrypt = base;
    encrypt[15] = encrypt[15].wrapping_add(2);
    (decrypt, encrypt)
}

/// Key and IV a stream of one audio session uses, derived from the session's
/// audio key and connection identifier (`upstream/UxPlay/lib/raop_buffer.c`
/// labels its stream material the same way).
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

/// Signs the two key exchange keys of `/pair-verify` and encrypts the result.
///
/// The server signs `our_ecdh_public || peer_ecdh_public` and uses the first 64
/// bytes of the shared keystream (`upstream/UxPlay/lib/pairing.c:404-415`).
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

/// Decrypts the signature a client sends in the second `/pair-verify` step.
///
/// The server and the client share one AES-CTR keystream: the server's own
/// signature used its first 64 bytes, so the client's signature is decrypted
/// with the second 64 bytes. Upstream advances the stream with one throwaway
/// round before decrypting (`upstream/UxPlay/lib/pairing.c:440-443`); skipping
/// that step would make every real client's signature fail to verify while a
/// self-consistent encrypt/decrypt round trip still passes.
pub fn decrypt_pair_verify_signature(
    encrypted_signature: &[u8; PAIRING_SIG_SIZE],
    shared_secret: &[u8; 32],
) -> Result<[u8; PAIRING_SIG_SIZE], CryptoError> {
    let keys = derive_pair_verify_keys(shared_secret);
    let mut cipher = Aes128CtrState::new(&keys.key, &keys.iv)?;
    let mut discarded = [0u8; PAIRING_SIG_SIZE];
    cipher.apply(&mut discarded);
    let mut plain = *encrypted_signature;
    cipher.apply(&mut plain);
    Ok(plain)
}

/// Verifies the signature a client sends in the second `/pair-verify` step.
///
/// The client signs `peer_ecdh_public || our_ecdh_public`, the opposite order of
/// the server's own signature (`upstream/UxPlay/lib/pairing.c:447-448`).
pub fn verify_pair_signature(
    signature: &[u8; PAIRING_SIG_SIZE],
    peer_ecdh_public: &[u8; 32],
    our_ecdh_public: &[u8; 32],
    peer_ed25519_public: &[u8; 32],
) -> bool {
    let message = [*peer_ecdh_public, *our_ecdh_public].concat();
    crate::crypto::ed25519_verify(peer_ed25519_public, &message, signature)
}

/// Lower-case hexadecimal MD5 of `input`, the encoding Digest authentication and
/// the RAOP device identifier use.
pub fn md5_hex(input: &[u8]) -> String {
    crate::crypto::md5_concat(&[input])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Digest response of a password-protected request
/// (`upstream/UxPlay/lib/http_handlers.h`), including the `qop=auth` variant.
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
    use crate::crypto::{aes128_gcm16_decrypt, aes128_gcm16_encrypt, ed25519_sign};

    /// Hexadecimal test vector as bytes.
    fn hex(text: &str) -> Vec<u8> {
        (0..text.len() / 2)
            .map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("hex"))
            .collect()
    }

    /// Fixed-size array from a hexadecimal test vector.
    fn bytes<const N: usize>(text: &str) -> [u8; N] {
        hex(text).try_into().expect("vector length")
    }

    /// Shared secret of the vectors below, also used by the pairing service
    /// tests. The values were reproduced independently with a second AES
    /// implementation before they were written down here.
    const VERIFY_SHARED: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

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
    fn pair_verify_keystream_splits_between_the_two_signatures() {
        let shared: [u8; 32] = bytes(VERIFY_SHARED);
        let keys = derive_pair_verify_keys(&shared);
        assert_eq!(keys.key, bytes("7ab12dc7046bf4a66df7fcd627142ba1"));
        assert_eq!(keys.iv, bytes("24309482c001edad6fdeb9e22f3f6468"));

        // Keystream blocks 1 and 2 of the shared AES-CTR stream.
        let mut cipher = Aes128CtrState::new(&keys.key, &keys.iv).expect("key and iv");
        let mut blocks = [0u8; 128];
        cipher.apply(&mut blocks);
        assert_eq!(
            &blocks[..64],
            &hex(concat!(
                "cee447115c4131af68db0486d99d7972b530389cff1bd79c5ef4772e60f76515",
                "0f7ba6f703a45fb1dadc107d107fbaa2c36a1e8d433ccc72aa212658c72ac8fe"
            ))[..]
        );
        assert_eq!(
            &blocks[64..],
            &hex(concat!(
                "1ce52e1bfb54d8e5ce173f7eb0e58754918df4e5c2ac4d0a97d87676ad5bd677",
                "68dde676c8df751b9aa9dd9287f199b4ae44d03daab685f44e2613bf6aef2725"
            ))[..]
        );

        // The server's own signature uses the first block.
        let identity: [u8; 32] =
            bytes("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
        let our_ecdh: [u8; 32] =
            bytes("202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f");
        let peer_ecdh: [u8; 32] =
            bytes("404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f");
        let signature = encrypt_pair_verify_signature(&identity, &our_ecdh, &peer_ecdh, &shared)
            .expect("signature");
        let mut expected = ed25519_sign(&identity, &[our_ecdh, peer_ecdh].concat());
        for (byte, keystream) in expected.iter_mut().zip(&blocks[..64]) {
            *byte ^= keystream;
        }
        assert_eq!(signature, expected);

        // The client's signature uses the second block, so a decrypt that starts
        // at offset zero would return a different plaintext.
        let plaintext: [u8; 64] = bytes(concat!(
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f"
        ));
        let mut client_signature = plaintext;
        for (byte, keystream) in client_signature.iter_mut().zip(&blocks[64..]) {
            *byte ^= keystream;
        }
        assert_eq!(
            decrypt_pair_verify_signature(&client_signature, &shared).expect("decrypt"),
            plaintext
        );
        let mut wrong_offset = client_signature;
        let mut offset_zero = Aes128CtrState::new(&keys.key, &keys.iv).expect("key and iv");
        offset_zero.apply(&mut wrong_offset);
        assert_ne!(wrong_offset, plaintext);
    }

    #[test]
    fn pair_setup_gcm_nonces_match_the_upstream_directions() {
        let session_key: [u8; 40] = bytes(
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f2021222324252627",
        );
        let keys = derive_pair_setup_keys(&session_key);
        assert_eq!(keys.key, bytes("707e65b3ccf0d4f5285711c73af9e9da"));
        assert_eq!(keys.iv, bytes("e975805955b609359bb91bc2e0df87d8"));

        let (decrypt_iv, encrypt_iv) = pair_setup_gcm_nonces(&session_key);
        assert_eq!(decrypt_iv[..15], keys.iv[..15]);
        assert_eq!(encrypt_iv[..15], keys.iv[..15]);
        assert_eq!(decrypt_iv[15], keys.iv[15].wrapping_add(1));
        assert_eq!(encrypt_iv[15], keys.iv[15].wrapping_add(2));

        // Request direction: the client's key exchange key, sealed with the
        // first nonce.
        let client_public: [u8; 32] =
            bytes("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
        let (epk, tag) =
            aes128_gcm16_encrypt(&keys.key, &decrypt_iv, &client_public).expect("seal");
        assert_eq!(
            epk,
            hex("4536c117786a8651cb186a72050576b52d5b806064e65e41beaba45f4fef9200")
        );
        assert_eq!(tag, bytes("168132b167324a180e7a6747a3729cde"));
        assert_eq!(
            aes128_gcm16_decrypt(&keys.key, &decrypt_iv, &epk, &tag).expect("open"),
            client_public
        );

        // Reply direction: the receiver's public key, sealed with the second
        // nonce, which is what a client authenticates the receiver with.
        let server_public: [u8; 32] =
            bytes("202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f");
        let (epk, tag) =
            aes128_gcm16_encrypt(&keys.key, &encrypt_iv, &server_public).expect("seal");
        assert_eq!(
            epk,
            hex("2bbfb37fc988be5ed3fdbffe590c81335bcddd7236ea37fef53621c26d77cddb")
        );
        assert_eq!(tag, bytes("0962549e68251e1994ba8cd0992187ef"));
        assert_ne!(
            epk,
            hex("6516e137584aa671eb384a52252556950d7ba04044c67e619e8b847f6fcfb220"),
            "sealing the reply with the request nonce would produce a different tag"
        );
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
