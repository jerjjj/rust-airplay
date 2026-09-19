// SPDX-License-Identifier: GPL-3.0-or-later
//! SRP-6a adapter and Apple's two-block SHA-1 session-key variant.
//!
//! The srp crate supplies RFC 5054 arithmetic. UxPlay uses SHA-1 with the
//! 2048-bit group and then expands the premaster integer to two SHA-1 blocks.
//! This module keeps that Apple-specific expansion explicit.

#![allow(missing_docs)]

use sha1::{Digest, Sha1};

pub const APPLE_SESSION_KEY_SIZE: usize = 40;

pub fn apple_session_key(premaster_secret_be: &[u8]) -> [u8; APPLE_SESSION_KEY_SIZE] {
    let mut output = [0u8; APPLE_SESSION_KEY_SIZE];
    let mut first = Sha1::new();
    first.update(premaster_secret_be);
    first.update([0, 0, 0, 0]);
    output[..20].copy_from_slice(&first.finalize());
    let mut second = Sha1::new();
    second.update(premaster_secret_be);
    second.update([0, 0, 0, 1]);
    output[20..].copy_from_slice(&second.finalize());
    output
}

pub fn apple_proof_key(session_key: &[u8; APPLE_SESSION_KEY_SIZE], label: &[u8]) -> [u8; 20] {
    let mut digest = Sha1::new();
    digest.update(label);
    digest.update(session_key);
    digest.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expands_secret_to_two_apple_blocks() {
        let key = apple_session_key(&[1, 2, 3]);
        assert_eq!(key.len(), APPLE_SESSION_KEY_SIZE);
        assert_ne!(&key[..20], &key[20..]);
    }
}
