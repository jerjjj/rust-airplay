//! RTSP message types and constants.

/// Stream type values used in RTSP SETUP.
pub mod stream_type {
    pub const AUDIO: u32 = 96;
    pub const VIDEO: u32 = 110;
}

/// Audio compression type codes.
pub mod audio_ct {
    pub const LPCM: u32 = 1;
    pub const ALAC: u32 = 2;
    pub const AAC: u32 = 4;
    pub const AAC_ELD: u32 = 8;
    pub const OPUS: u32 = 32;
}

/// Video payload type codes in the 128-byte stream header.
pub mod video_payload_type {
    pub const ENCRYPTED_DATA: u16 = 0;
    pub const SPS_PPS: u16 = 1;
}

/// FairPlay "FPLY" magic bytes.
pub const FPLY_MAGIC: [u8; 4] = [0x46, 0x50, 0x4C, 0x59];

/// FairPlay protocol version.
pub const FPLY_VERSION: u8 = 0x03;

/// FairPlay message types.
pub mod fp_msg_type {
    pub const SETUP1: u8 = 0x01;  // Client → Server first request
    pub const SETUP2: u8 = 0x03;  // Client → Server second request
    pub const SETUP3: u8 = 0x04;  // Server → Client response to second
}
