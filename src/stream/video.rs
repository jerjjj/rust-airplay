//! TCP video stream handler — decrypts and outputs H.264 Annex B to file.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::fairplay::video_decrypt::VideoDecryptor;
use crate::stream::nal;

const HEADER_SIZE: usize = 128;

pub async fn run_video_event_server(listener: TcpListener) -> anyhow::Result<()> {
    tracing::warn!("Video EVENT server listening on {}", listener.local_addr()?);
    loop {
        let (mut stream, peer) = listener.accept().await?;
        tracing::warn!("Video EVENT connection from {} (client should now send video)", peer);
        tokio::spawn(async move {
            let mut buf = [0u8; 256];
            // Some clients expect a small reply on the event port
            let _ = stream.try_write(b"\x00");
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
    listener: TcpListener,
    decryptor: Arc<Mutex<VideoDecryptor>>,
    needs_retry: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    tracing::warn!("Video server on {} (output: video.h264)", listener.local_addr()?);

    loop {
        let (stream, peer) = listener.accept().await?;
        tracing::warn!("Video connection from {}", peer);
        let dec = decryptor.clone();
        let nr = needs_retry.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_video_connection(stream, dec, nr).await {
                tracing::error!("Video error: {}", e);
            }
        });
    }
}

async fn handle_video_connection(
    mut stream: TcpStream,
    decryptor: Arc<Mutex<VideoDecryptor>>,
    needs_retry: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let mut header_buf = vec![0u8; HEADER_SIZE];
    let mut out = tokio::fs::File::create("video.h264").await?;
    let mut sps_pps: Option<Vec<u8>> = None;
    let mut frame_count = 0u64;
    let mut payload_buf = Vec::with_capacity(65536);

    loop {
        if frame_count == 0 {
            match tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut header_buf)).await {
                Ok(Ok(_n)) => {}
                Ok(Err(e)) => {
                    tracing::warn!("First frame read error: {}", e);
                    if e.kind() == std::io::ErrorKind::UnexpectedEof { break; }
                    return Err(e.into());
                }
                Err(_) => {
                    tracing::warn!("No video data after 10s — client may not be sending");
                    return Err(anyhow::anyhow!("Video data timeout"));
                }
            }
        } else {
            if let Err(e) = stream.read_exact(&mut header_buf).await {
                if e.kind() == std::io::ErrorKind::UnexpectedEof { break; }
                return Err(e.into());
            }
        }

        let payload_size = u32::from_le_bytes([header_buf[0],header_buf[1],header_buf[2],header_buf[3]]) as usize;
        let payload_type = u16::from_le_bytes([header_buf[4], header_buf[5]]) & 0xFF;

        if payload_size == 0 { continue; }
        if payload_buf.len() < payload_size { payload_buf.resize(payload_size, 0); }
        stream.read_exact(&mut payload_buf[..payload_size]).await?;

        match payload_type {
            0 => {
                if frame_count == 0 {
                    let _ = std::fs::write("encrypted_frame.bin", &payload_buf[..payload_size]);
                }
                let payload = &mut payload_buf[..payload_size];
                let mut d = decryptor.lock().await;
                if d.cipher.is_none() {
                    tracing::error!("Video decryptor NOT initialized at frame {} — skipping.", frame_count);
                    continue;
                }
                d.decrypt(payload);
                drop(d);

                let annex = nal::nalus_to_annex_b(payload);
                if frame_count == 0 && annex.is_empty() {
                    tracing::error!("Bad decryption — triggering auto-retry");
                    needs_retry.store(true, Ordering::SeqCst);
                    return Err(anyhow::anyhow!("Decryption failed — session needs restart"));
                }
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
                if let Some(annex) = nal::extract_sps_pps(&payload_buf[..payload_size]) {
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
