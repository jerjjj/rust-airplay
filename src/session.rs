//! Per-client session state tracking pairing, FairPlay, and stream state.

use crate::pairing::PairingKeys;
use x25519_dalek::StaticSecret as X25519Secret;

/// State for one connected iOS client.
pub struct Session {
    /// Unique session identifier.
    pub id: String,
    /// The server's long-term Ed25519 keys (shared across all sessions).
    pub pairing_keys: PairingKeys,
    /// Whether pair-verify completed successfully.
    pub pair_verified: bool,
    /// Client's Curve25519 ECDH public key from pair-verify round 1.
    pub client_ecdh_public: Option<[u8; 32]>,
    /// Client's Ed25519 public key from pair-verify round 1.
    pub client_ed25519_public: Option<[u8; 32]>,
    /// Our ephemeral Curve25519 ECDH private key.
    pub ecdh_private: Option<X25519Secret>,
    /// Our ephemeral Curve25519 ECDH public key.
    pub ecdh_public: Option<[u8; 32]>,
    /// ECDH shared secret (32 bytes).
    pub shared_secret: Option<[u8; 32]>,
    /// FairPlay aesKey (16 bytes) — extracted by OmgHax.
    pub aes_key: Option<[u8; 16]>,
    /// FairPlay eiv (16 bytes) — from RTSP SETUP.
    pub eiv: Option<[u8; 16]>,
    /// FairPlay keyMsg buffer (164 bytes from fp-setup round 2).
    pub key_msg: Option<Vec<u8>>,
    /// Stream connection ID from client SETUP.
    pub stream_connection_id: Option<u64>,
    /// TCP video data port (assigned by server).
    pub video_data_port: Option<u16>,
    /// UDP audio data port (assigned by server).
    pub audio_data_port: Option<u16>,
    /// UDP audio control port (assigned by server).
    pub audio_control_port: Option<u16>,
}

impl Session {
    pub fn new(pairing_keys: PairingKeys) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            pairing_keys,
            pair_verified: false,
            client_ecdh_public: None,
            client_ed25519_public: None,
            ecdh_private: None,
            ecdh_public: None,
            shared_secret: None,
            aes_key: None,
            eiv: None,
            key_msg: None,
            stream_connection_id: None,
            video_data_port: None,
            audio_data_port: None,
            audio_control_port: None,
        }
    }
}
