mod config;
mod fairplay;
mod mdns;
mod pairing;
mod rtsp;
mod session;
mod stream;
mod test_values;

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use config::Config;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::Mutex;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::fairplay::audio_decrypt::AudioDecryptor;
use crate::fairplay::video_decrypt::VideoDecryptor;

pub struct RuntimeState {
    pub video_data_port: u16,
    pub video_event_port: u16,
    pub audio_data_port: u16,
    pub audio_control_port: u16,
    pub video_decryptor: Arc<Mutex<VideoDecryptor>>,
    pub audio_decryptor: Arc<Mutex<AudioDecryptor>>,
    /// Set to true by video handler when decryption fails; RTSP handler closes connection on next msg.
    pub needs_retry: Arc<AtomicBool>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("airplay_service=info,mdns_sd=info,tracing=warn"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    let config = Config::default();

    log_interfaces();

    tracing::info!("Starting AirPlay service: {} ({}x{}@{})",
        config.server_name, config.width, config.height, config.fps);

    // Bind sockets once — no TOCTOU race
    let video_data = TcpListener::bind("0.0.0.0:0").await?;
    let video_event = TcpListener::bind("0.0.0.0:0").await?;
    let audio_data = UdpSocket::bind("0.0.0.0:0").await?;
    let audio_control = UdpSocket::bind("0.0.0.0:0").await?;

    let video_data_port = video_data.local_addr()?.port();
    let video_event_port = video_event.local_addr()?.port();
    let audio_data_port = audio_data.local_addr()?.port();
    let audio_control_port = audio_control.local_addr()?.port();

    let video_decryptor = Arc::new(Mutex::new(VideoDecryptor::new()));
    let audio_decryptor = Arc::new(Mutex::new(AudioDecryptor::new()));
    let needs_retry = Arc::new(AtomicBool::new(false));

    let runtime = Arc::new(RuntimeState {
        video_data_port, video_event_port, audio_data_port, audio_control_port,
        video_decryptor: video_decryptor.clone(),
        audio_decryptor: audio_decryptor.clone(),
        needs_retry: needs_retry.clone(),
    });

    let rtsp_handle = rtsp::start(&config, runtime.clone()).await?;
    let control_port = rtsp_handle.port;
    tracing::info!("RTSP control port: {}", control_port);



    let config_with_port = Config { airtunes_port: control_port, ..config.clone() };
    let mdns_handle = mdns::register(&config_with_port).await?;

    // Spawn stream servers (pass pre-bound sockets, no TOCTOU)
    tokio::spawn(async move { if let Err(e) = stream::video::run_video_server(video_data, video_decryptor, needs_retry).await { tracing::error!("Video server: {}", e); } });
    tokio::spawn(async move { if let Err(e) = stream::video::run_video_event_server(video_event).await { tracing::error!("Event server: {}", e); } });
    tokio::spawn(async move { if let Err(e) = stream::audio::run_audio_server(audio_data, audio_decryptor).await { tracing::error!("Audio server: {}", e); } });
    tokio::spawn(async move { if let Err(e) = stream::audio::run_audio_control_server(audio_control).await { tracing::error!("Audio ctl server: {}", e); } });

    tracing::info!("Ready. Try: curl http://{}:{}/info", local_ip(), control_port);

    tokio::signal::ctrl_c().await?;
    mdns_handle.unregister().await?;
    Ok(())
}

fn find_lan_ip() -> IpAddr {
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in &ifaces {
            if iface.is_loopback() { continue; }
            if let std::net::IpAddr::V4(ip) = iface.ip() {
                let o = ip.octets();
                if o[0] == 127 || o[0] == 169 || o[0] == 198 { continue; }
                if o[0] == 172 && (16..=31).contains(&o[1]) { return std::net::IpAddr::V4(ip); }
                if o[0] == 192 || o[0] == 10 { return std::net::IpAddr::V4(ip); }
            }
        }
        for iface in &ifaces {
            if !iface.is_loopback() { return iface.ip(); }
        }
    }
    std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))
}

fn local_ip() -> String { find_lan_ip().to_string() }

fn log_interfaces() {
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        tracing::info!("Network interfaces:");
        for i in &ifaces { tracing::info!("  {}: {} (loopback={})", i.name, i.ip(), i.is_loopback()); }
    }
}
