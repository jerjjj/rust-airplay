//! TCP video stream handler — decrypts and outputs H.264 Annex B to file.

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::fairplay::video_decrypt::VideoDecryptor;
use crate::stream::nal;

const HEADER_SIZE: usize = 128;

pub async fn run_video_event_server(port: u16) -> anyhow::Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    tracing::info!("Video event server on {}", port);
    loop {
        let (mut stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = [0u8; 256];
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });
    }
}

pub async fn run_video_server(
    port: u16,
    decryptor: Arc<Mutex<VideoDecryptor>>,
) -> anyhow::Result<()> {
    let addr4 = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr4).await?;
    tracing::warn!("Video server on {} (output: video.h264)", addr4);

    loop {
        let (stream, peer) = listener.accept().await?;
        tracing::warn!("Video connection from {}", peer);
        let dec = decryptor.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_video_connection(stream, dec).await {
                tracing::error!("Video error: {}", e);
            }
        });
    }
}

async fn handle_video_connection(
    mut stream: TcpStream,
    decryptor: Arc<Mutex<VideoDecryptor>>,
) -> anyhow::Result<()> {
    let mut header_buf = vec![0u8; HEADER_SIZE];
    let mut out = tokio::fs::File::create("video.h264").await?;
    let mut sps_pps: Option<Vec<u8>> = None;
    let mut frame_count = 0u64;
    let mut payload_buf = Vec::with_capacity(65536);

    loop {
        if let Err(e) = stream.read_exact(&mut header_buf).await {
            if e.kind() == std::io::ErrorKind::UnexpectedEof { break; }
            return Err(e.into());
        }

        let payload_size = u32::from_le_bytes([header_buf[0],header_buf[1],header_buf[2],header_buf[3]]) as usize;
        let payload_type = u16::from_le_bytes([header_buf[4], header_buf[5]]) & 0xFF;

        if payload_size == 0 { continue; }
        if payload_buf.len() < payload_size { payload_buf.resize(payload_size, 0); }
        stream.read_exact(&mut payload_buf[..payload_size]).await?;
        let payload = &mut payload_buf[..payload_size];

        match payload_type {
            0 => {
                let mut d = decryptor.lock().await;
                if d.cipher.is_none() {
                    tracing::error!("Video decryptor NOT initialized — skipping frame.");
                    continue;
                }
                d.decrypt(payload);
                drop(d);

                let annex = nal::nalus_to_annex_b(payload);
                if !annex.is_empty() {
                    if let Some(ref sps) = sps_pps {
                        if annex.windows(4).any(|w| (w[3] & 0x1F) == 5) {
                            out.write_all(sps).await?;
                        }
                    }
                    out.write_all(&annex).await?;
                    frame_count += 1;
                    if frame_count % 60 == 0 {
                        tracing::info!("Video: {} frames", frame_count);
                    }
                }
            }
            1 => {
                if let Some(annex) = nal::extract_sps_pps(payload) {
                    out.write_all(&annex).await?;
                    sps_pps = Some(annex);
                }
            }
            _ => {}
        }
    }
    out.flush().await?;
    tracing::warn!("Video done: {} frames", frame_count);
    Ok(())
}
