mod config;
mod fairplay;
mod mdns;
mod pairing;
mod rtsp;
mod session;
mod stream;
mod test_values;

use std::net::{SocketAddr, IpAddr};
use std::sync::Arc;

use anyhow::Result;
use std::env;
use config::Config;
use tokio::sync::Mutex;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::fairplay::audio_decrypt::AudioDecryptor;
use crate::fairplay::video_decrypt::VideoDecryptor;

/// Hardcoded AES key from command line (for testing without FairPlay)
pub static mut HARDCODED_AES_KEY: Option<[u8; 16]> = None;

pub struct RuntimeState {
    pub video_data_port: u16,
    pub video_event_port: u16,
    pub audio_data_port: u16,
    pub audio_control_port: u16,
    pub video_decryptor: Arc<Mutex<VideoDecryptor>>,
    pub audio_decryptor: Arc<Mutex<AudioDecryptor>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            "airplay_service=info,mdns_sd=info,tracing=warn".parse().unwrap()
        });
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    let config = Config::default();

    // Check for --aes-key argument
    let args: Vec<String> = env::args().collect();
    if args.len() > 2 && args[1] == "--aes-key" {
        let hex = &args[2];
        if hex.len() == 32 {
            let mut key = [0u8; 16];
            for i in 0..16 {
                key[i] = u8::from_str_radix(&hex[i*2..i*2+2], 16).unwrap_or(0);
            }
            unsafe { HARDCODED_AES_KEY = Some(key); }
            tracing::warn!("Using hardcoded AES key: {:02x?}", &key[..]);
        }
    }

    log_interfaces();

    tracing::info!("Starting AirPlay service: {} ({}x{}@{})",
        config.server_name, config.width, config.height, config.fps);

    let (video_data_port, _) = bind_tcp("0.0.0.0:0").await?;
    let (video_event_port, _) = bind_tcp("0.0.0.0:0").await?;
    let (audio_data_port, _) = bind_udp("0.0.0.0:0").await?;
    let (audio_control_port, _) = bind_udp("0.0.0.0:0").await?;

    let video_decryptor = Arc::new(Mutex::new(VideoDecryptor::new()));
    let audio_decryptor = Arc::new(Mutex::new(AudioDecryptor::new()));

    let runtime = Arc::new(RuntimeState {
        video_data_port, video_event_port, audio_data_port, audio_control_port,
        video_decryptor: video_decryptor.clone(),
        audio_decryptor: audio_decryptor.clone(),
    });

    let rtsp_handle = rtsp::start(&config, runtime.clone()).await?;
    let control_port = rtsp_handle.port;
    tracing::info!("RTSP control port: {}", control_port);



    let config_with_port = Config { airtunes_port: control_port, ..config.clone() };
    let mdns_handle = mdns::register(&config_with_port).await?;

    // Spawn stream servers
    tokio::spawn(async move { let _ = stream::video::run_video_server(video_data_port, video_decryptor).await; });
    tokio::spawn(async move { let _ = stream::audio::run_audio_server(audio_data_port, audio_control_port, audio_decryptor).await; });
    tokio::spawn(async move { let _ = stream::audio::run_audio_control_server(audio_control_port).await; });

    tracing::info!("Ready. Try: curl http://{}:{}/info", local_ip(), control_port);

    tokio::signal::ctrl_c().await?;
    mdns_handle.unregister().await?;
    Ok(())
}

async fn bind_tcp(addr: &str) -> Result<(u16, SocketAddr)> {
    let l = tokio::net::TcpListener::bind(addr).await?;
    let a = l.local_addr()?;
    drop(l);
    Ok((a.port(), a))
}

async fn bind_udp(addr: &str) -> Result<(u16, SocketAddr)> {
    let s = tokio::net::UdpSocket::bind(addr).await?;
    let a = s.local_addr()?;
    drop(s);
    Ok((a.port(), a))
}

fn find_lan_ip() -> IpAddr {
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in &ifaces {
            if iface.is_loopback() { continue; }
            if let std::net::IpAddr::V4(ip) = iface.ip() {
                let o = ip.octets();
                if o[0] == 198 || o[0] == 169 || o[0] == 172 || o[0] == 127 { continue; }
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
