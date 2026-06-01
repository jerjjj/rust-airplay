use std::net::SocketAddr;

/// Server configuration for the AirPlay mirroring service.
#[derive(Debug, Clone)]
pub struct Config {
    /// Display width in pixels reported to the client.
    pub width: u32,
    /// Display height in pixels reported to the client.
    pub height: u32,
    /// Maximum frames per second reported to the client.
    pub fps: u32,
    /// Display refresh rate reported to the client.
    pub refresh_rate: u32,
    /// Server name shown in the iOS Control Center.
    pub server_name: String,
    /// MAC address string (colon-separated) used as device identifier.
    pub mac_address: String,
    /// AirTunes/AirPlay service port.
    pub airtunes_port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 60,
            refresh_rate: 60,
            server_name: "Rust AirPlay".to_string(),
            mac_address: "AA:BB:CC:DD:EE:FF".to_string(),
            airtunes_port: 5002, // fixed port like java-airplay uses 5001
        }
    }
}

impl Config {
    /// The features bitmask: `0x5A7FFFF7` (video, photo, audio, screen mirror, HLS, encryption, FairPlay 1&2).
    pub fn features_hex(&self) -> String {
        "0x5A7FFFF7,0x1E".to_string()
    }

    /// The larger features integer for the `/info` response — `0x1E5A7FFFF7`.
    pub fn features_int(&self) -> u64 {
        0x1E5A7FFFF7u64
    }

    /// Audio format bitmask: ALAC 44.1kHz stereo + AAC-ELD 44.1kHz stereo + OPUS 48kHz mono.
    pub fn audio_formats(&self) -> u32 {
        0x3FFFDFCu32
    }

    /// Build the RTSP control server address.
    pub fn control_addr(&self) -> SocketAddr {
        SocketAddr::from(([0, 0, 0, 0], self.airtunes_port))
    }
}
