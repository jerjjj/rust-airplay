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

impl Config {}
