//! EdDSA + Curve25519 ECDH pairing protocol.
//!
//! Implements the three-step pair-verify handshake:
//!   1. `/pair-setup` — server sends its Ed25519 public key.
//!   2. `/pair-verify` round 1 — ECDH key exchange + encrypted EdDSA signature.
//!   3. `/pair-verify` round 2 — client sends encrypted signature; server verifies.

use aes::Aes128;
use ctr::cipher::{KeyIvInit, StreamCipher};
use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer, Verifier};
use rand::rngs::OsRng;
use sha2::{Sha512, Digest};
use x25519_dalek::{StaticSecret as X25519Secret, PublicKey as X25519Public};

pub(crate) type Aes128Ctr = ctr::Ctr128BE<Aes128>;

/// The server's long-term Ed25519 signing key.
pub struct PairingKeys {
    pub ed25519_signing: SigningKey,
    pub ed25519_verifying: VerifyingKey,
}

impl PairingKeys {
    /// Generate a fresh Ed25519 keypair.
    pub fn generate() -> Self {
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        Self {
            ed25519_signing: signing,
            ed25519_verifying: verifying,
        }
    }

    /// Raw 32-byte Ed25519 public key (the `A` value).
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.ed25519_verifying.to_bytes()
    }
}

/// An in-progress pair-verify session.
pub struct PairVerifySession {
    /// Our ephemeral Curve25519 ECDH private key.
    pub ecdh_private: X25519Secret,
    /// Derived AES-CTR cipher for encrypting/decrypting signatures.
    pub cipher: Aes128Ctr,
}

/// Result of pair-verify round 1.
pub struct Round1Result {
    pub server_ecdh_public: [u8; 32],
    pub encrypted_signature: [u8; 64],
    pub shared_secret: [u8; 32],
}

impl PairVerifySession {
    /// Perform round 1 of pair-verify: receive client's Curve25519 + Ed25519 public keys,
    /// derive shared secret, build AES-CTR cipher, encrypt our EdDSA signature.
    pub fn round1(
        keys: &PairingKeys,
        client_ecdh_public: &[u8; 32],
    ) -> Round1Result {
        let ecdh_private = X25519Secret::random_from_rng(OsRng);
        let ecdh_public = X25519Public::from(&ecdh_private);

        // shared_secret = Curve25519.agree(client_pub, our_priv)
        let client_pub = X25519Public::from(*client_ecdh_public);
        let shared_secret = ecdh_private.diffie_hellman(&client_pub);
        tracing::debug!(
            "pair-verify round1: shared_secret={:02x?}..{:02x?}",
            &shared_secret.as_bytes()[..4],
            &shared_secret.as_bytes()[28..32]
        );

        // Derive aesKey = SHA-512("Pair-Verify-AES-Key" || shared_secret)[0..16]
        let aes_key = kdf_16(b"Pair-Verify-AES-Key", shared_secret.as_bytes());
        // Derive aesIV  = SHA-512("Pair-Verify-AES-IV"  || shared_secret)[0..16]
        let aes_iv = kdf_16(b"Pair-Verify-AES-IV", shared_secret.as_bytes());

        let cipher = Aes128Ctr::new(&aes_key.into(), &aes_iv.into());

        // data_to_sign = our_ecdh_pub || client_ecdh_pub
        let mut data_to_sign = Vec::with_capacity(64);
        data_to_sign.extend_from_slice(ecdh_public.as_bytes());
        data_to_sign.extend_from_slice(client_ecdh_public);
        let signature: Signature = keys.ed25519_signing.sign(&data_to_sign);

        // Encrypt signature
        let mut encrypted_sig = signature.to_bytes();
        let mut enc_cipher = cipher.clone();
        enc_cipher.apply_keystream(&mut encrypted_sig);
        // Self-test: decrypt back to verify round-trip
        let mut test = encrypted_sig;
        cipher.clone().apply_keystream(&mut test);
        debug_assert_eq!(test, signature.to_bytes(), "AES-CTR round-trip failed!");

        let mut shared_secret_bytes = [0u8; 32];
        shared_secret_bytes.copy_from_slice(shared_secret.as_bytes());

        Round1Result {
            server_ecdh_public: ecdh_public.to_bytes(),
            encrypted_signature: encrypted_sig,
            shared_secret: shared_secret_bytes,
        }
    }

    /// Perform round 2 of pair-verify: decrypt client's signature and verify.
    pub fn round2(
        cipher: &mut Aes128Ctr,
        client_encrypted_sig: &[u8; 64],
        client_ed25519_public: &[u8; 32],
        client_ecdh_public: &[u8; 32],
        server_ecdh_public: &[u8; 32],
    ) -> bool {
        let mut sig_bytes = *client_encrypted_sig;
        cipher.apply_keystream(&mut sig_bytes);
        // Apple devices may set high bits in S; clear them per RFC 8032
        sig_bytes[63] &= 0b0001_1111;

        tracing::info!("round2 decrypted sig: {:02x?}...", &sig_bytes[..8]);
        tracing::info!("round2 client_ed_pub: {:02x?}...", &client_ed25519_public[..8]);
        tracing::info!("round2 client_ecdh: {:02x?}...", &client_ecdh_public[..8]);
        tracing::info!("round2 server_ecdh: {:02x?}...", &server_ecdh_public[..8]);

        let signature = match Signature::from_slice(&sig_bytes) {
            Ok(sig) => {
                tracing::info!("round2 signature parsed OK");
                sig
            }
            Err(e) => {
                tracing::error!("round2 signature parse error: {}", e);
                return false;
            }
        };

        let verifying_key = match VerifyingKey::from_bytes(client_ed25519_public) {
            Ok(vk) => {
                tracing::info!("round2 verifying_key parsed OK");
                vk
            }
            Err(e) => {
                tracing::error!("round2 verifying_key parse error: {}", e);
                return false;
            }
        };

        // Server verifies client's sig of: client_ecdh || server_ecdh
        // (from server's perspective: 对方=client, 己方=server)
        let mut sig_message = Vec::with_capacity(64);
        sig_message.extend_from_slice(client_ecdh_public);
        sig_message.extend_from_slice(server_ecdh_public);

        match verifying_key.verify(&sig_message, &signature) {
            Ok(()) => {
                tracing::info!("round2 signature VERIFIED");
                true
            }
            Err(e) => {
                tracing::error!("round2 signature verify error: {}", e);
                false
            }
        }
    }
}

/// SHA-512 KDF: digest(prefix || secret), return first 16 bytes.
pub fn kdf_16(prefix: &[u8], secret: &[u8]) -> [u8; 16] {
    let mut hasher = Sha512::new();
    hasher.update(prefix);
    hasher.update(secret);
    let result = hasher.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&result[..16]);
    out
}


