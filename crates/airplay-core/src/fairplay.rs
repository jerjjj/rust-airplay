// SPDX-License-Identifier: LGPL-2.1-or-later
//! FairPlay /fp-setup request/response state handling.
//!
//! The fixed setup replies and handshake echo are protocol framing. The
//! PlayFair key derivation and 72-byte decrypt path are intentionally kept in
//! a separate implementation step so this state machine cannot report a
//! decrypted key before that transform is ported.

#![allow(missing_docs)]

use crate::fairplay_tables::{FP_HEADER, REPLY_MESSAGES};

pub const SETUP_REQUEST_LEN: usize = 16;
pub const SETUP_RESPONSE_LEN: usize = 142;
pub const HANDSHAKE_REQUEST_LEN: usize = 164;
pub const HANDSHAKE_RESPONSE_LEN: usize = 32;
pub const ENCRYPTED_KEY_LEN: usize = 72;
pub const DECRYPTED_KEY_LEN: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FairPlayError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u8),
    InvalidMode(u8),
    HandshakeRequired,
    PlayFairUnavailable,
}

impl std::fmt::Display for FairPlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    f,
                    "invalid FairPlay message length: expected {expected}, got {actual}"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported FairPlay version 0x{version:02x}")
            }
            Self::InvalidMode(mode) => write!(f, "invalid FairPlay setup mode {mode}"),
            Self::HandshakeRequired => f.write_str("FairPlay handshake has not completed"),
            Self::PlayFairUnavailable => f.write_str("PlayFair transform is not ported yet"),
        }
    }
}

impl std::error::Error for FairPlayError {}

#[derive(Debug, Default)]
pub struct FairPlaySession {
    key_message: Option<[u8; HANDSHAKE_REQUEST_LEN]>,
}

impl FairPlaySession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn setup(&mut self, request: &[u8]) -> Result<[u8; SETUP_RESPONSE_LEN], FairPlayError> {
        require_len(request, SETUP_REQUEST_LEN)?;
        require_version(request)?;
        let mode = request[14];
        if mode > 3 {
            return Err(FairPlayError::InvalidMode(mode));
        }

        let start = usize::from(mode) * SETUP_RESPONSE_LEN;
        let mut response = [0u8; SETUP_RESPONSE_LEN];
        response.copy_from_slice(&REPLY_MESSAGES[start..start + SETUP_RESPONSE_LEN]);
        self.key_message = None;
        Ok(response)
    }

    pub fn handshake(
        &mut self,
        request: &[u8],
    ) -> Result<[u8; HANDSHAKE_RESPONSE_LEN], FairPlayError> {
        require_len(request, HANDSHAKE_REQUEST_LEN)?;
        require_version(request)?;

        let mut key_message = [0u8; HANDSHAKE_REQUEST_LEN];
        key_message.copy_from_slice(request);
        self.key_message = Some(key_message);

        let mut response = [0u8; HANDSHAKE_RESPONSE_LEN];
        response[..FP_HEADER.len()].copy_from_slice(&FP_HEADER);
        response[FP_HEADER.len()..].copy_from_slice(&request[144..]);
        Ok(response)
    }

    pub fn decrypt(&self, encrypted_key: &[u8]) -> Result<[u8; DECRYPTED_KEY_LEN], FairPlayError> {
        require_len(encrypted_key, ENCRYPTED_KEY_LEN)?;
        if self.key_message.is_none() {
            return Err(FairPlayError::HandshakeRequired);
        }
        Err(FairPlayError::PlayFairUnavailable)
    }

    pub fn handshake_complete(&self) -> bool {
        self.key_message.is_some()
    }
}

fn require_len(input: &[u8], expected: usize) -> Result<(), FairPlayError> {
    if input.len() == expected {
        Ok(())
    } else {
        Err(FairPlayError::InvalidLength {
            expected,
            actual: input.len(),
        })
    }
}

fn require_version(input: &[u8]) -> Result<(), FairPlayError> {
    if input[4] == 0x03 {
        Ok(())
    } else {
        Err(FairPlayError::UnsupportedVersion(input[4]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_request(mode: u8) -> [u8; SETUP_REQUEST_LEN] {
        let mut request = [0u8; SETUP_REQUEST_LEN];
        request[4] = 0x03;
        request[14] = mode;
        request
    }

    #[test]
    fn setup_selects_fixed_reply_and_resets_handshake() {
        let mut session = FairPlaySession::new();
        let response = session.setup(&setup_request(2)).unwrap();
        assert_eq!(&response[..12], b"FPLY\x03\x01\x02\0\0\0\0\x82");
        assert!(!session.handshake_complete());
    }

    #[test]
    fn handshake_echoes_client_tail() {
        let mut session = FairPlaySession::new();
        let mut request = [0u8; HANDSHAKE_REQUEST_LEN];
        request[4] = 0x03;
        for (index, byte) in request[144..].iter_mut().enumerate() {
            *byte = index as u8;
        }

        let response = session.handshake(&request).unwrap();
        assert_eq!(&response[..12], &FP_HEADER);
        assert_eq!(&response[12..], &request[144..]);
        assert!(session.handshake_complete());
    }

    #[test]
    fn rejects_invalid_state_and_lengths() {
        let mut session = FairPlaySession::new();
        assert_eq!(
            session.decrypt(&[0u8; ENCRYPTED_KEY_LEN]),
            Err(FairPlayError::HandshakeRequired)
        );
        assert!(matches!(
            session.setup(&[0u8; SETUP_REQUEST_LEN - 1]),
            Err(FairPlayError::InvalidLength { .. })
        ));
        assert_eq!(
            session.setup(&setup_request(4)),
            Err(FairPlayError::InvalidMode(4))
        );
        assert_eq!(
            session.setup(&{
                let mut request = setup_request(0);
                request[4] = 2;
                request
            }),
            Err(FairPlayError::UnsupportedVersion(2))
        );
    }
}
