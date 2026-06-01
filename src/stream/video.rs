//! TCP video stream handler — decrypts and outputs H.264 Annex B to file.

use std::io::Write;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::fairplay::video_decrypt::VideoDecryptor;
use crate::stream::nal;

const HEADER_SIZE: usize = 128;

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
    let mut out = std::fs::File::create("video.h264")?;
    let mut sps_pps: Option<Vec<u8>> = None;
    let mut frame_count = 0u64;

    loop {
        if let Err(e) = stream.read_exact(&mut header_buf).await {
            if e.kind() == std::io::ErrorKind::UnexpectedEof { break; }
            return Err(e.into());
        }

        let payload_size = u32::from_le_bytes([header_buf[0],header_buf[1],header_buf[2],header_buf[3]]) as usize;
        let payload_type = u16::from_le_bytes([header_buf[4], header_buf[5]]) & 0xFF;

        if payload_size == 0 { continue; }
        let mut payload = vec![0u8; payload_size];
        stream.read_exact(&mut payload).await?;

        match payload_type {
            0 => {
                // Save first encrypted frame to file for debugging
                if frame_count == 0 {
                    let mut f = std::fs::File::create("encrypted_frame.bin").unwrap();
                    std::io::Write::write_all(&mut f, &payload).unwrap();
                    tracing::warn!("Saved encrypted frame: {} bytes to encrypted_frame.bin", payload.len());
                    tracing::warn!("Encrypted first 32: {:02x?}", &payload[..32.min(payload.len())]);
                }
                let has_cipher = { let d = decryptor.lock().await; d.cipher.is_some() };
                if !has_cipher {
                    tracing::error!("Video decryptor NOT initialized — waiting for ekey+streams SETUP. Skipping frame.");
                    continue;
                }
                { let mut d = decryptor.lock().await; d.decrypt(&mut payload); }
                if frame_count == 0 {
                    tracing::warn!("Decrypted first 32: {:02x?}", &payload[..32.min(payload.len())]);
                }
                let annex = nal::nalus_to_annex_b(&payload);
                if frame_count < 3 {
                    let nalu0 = if payload.len() >= 4 { u32::from_be_bytes([payload[0],payload[1],payload[2],payload[3]]) } else { 0 };
                    tracing::warn!("Frame {}: {}B nalu0_size={} annex={}", 
                        frame_count, payload_size, nalu0, annex.len());
                }
                if !annex.is_empty() {
                    // Prepend SPS/PPS for key frames
                    if let Some(ref sps) = sps_pps {
                        if annex.windows(4).any(|w| {
                            (w[3] & 0x1F) == 5 // IDR slice
                        }) {
                            out.write_all(sps)?;
                        }
                    }
                    out.write_all(&annex)?;
                    frame_count += 1;
                    if frame_count % 60 == 0 {
                        tracing::info!("Video: {} frames written", frame_count);
                    }
                }
            }
            1 => {
                if let Some(annex) = nal::extract_sps_pps(&payload) {
                    tracing::warn!("SPS/PPS: {} bytes", annex.len());
                    out.write_all(&annex)?;
                    sps_pps = Some(annex);
                }
            }
            _ => {}
        }
    }
    out.flush()?;
    tracing::warn!("Video done: {} frames to video.h264", frame_count);
    Ok(())
}
