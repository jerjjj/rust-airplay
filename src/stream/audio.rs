//! UDP audio stream handler.
//!
//! Receives audio packets with 12-byte RTP-like headers:
//!   Offset 0:     flags (1 byte)
//!   Offset 1:     type = header[1] & 0x7F (1 byte)
//!   Offset 2-3:   sequenceNumber (u16, big-endian)
//!   Offset 4-7:   timestamp (u32, big-endian)
//!   Offset 8-11:  SSRC (u32, big-endian)
//!   Offset 12+:   encrypted audio data (AES-CBC)
//!
//! Includes a 512-slot reorder buffer to handle UDP out-of-order delivery.

use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

use crate::fairplay::audio_decrypt::AudioDecryptor;

/// Audio packet with sequence number for reorder buffer.
#[derive(Debug, Clone)]
pub(crate) struct AudioPacket {
    seq: u16,
    data: Vec<u8>,
}

/// Reorder buffer with 512 slots.
const BUFFER_SIZE: usize = 512;

pub struct AudioReorderBuffer {
    buffer: [Option<AudioPacket>; BUFFER_SIZE],
    prev_seq: u16,
}

impl AudioReorderBuffer {
    pub fn new() -> Self {
        Self {
            buffer: std::array::from_fn(|_| None),
            prev_seq: 0,
        }
    }

    /// Insert a packet into the buffer.
    pub fn insert(&mut self, seq: u16, data: Vec<u8>) {
        let idx = (seq as usize) % BUFFER_SIZE;
        self.buffer[idx] = Some(AudioPacket { seq, data });
    }

    /// Dequeue packets in order. Returns all ready packets.
    pub fn dequeue(&mut self) -> Vec<AudioPacket> {
        let mut ready = Vec::new();
        loop {
            let next_seq = self.prev_seq.wrapping_add(1);
            let idx = (next_seq as usize) % BUFFER_SIZE;
            match self.buffer[idx].take() {
                Some(pkt) if pkt.seq == next_seq => {
                    ready.push(pkt);
                    self.prev_seq = next_seq;
                }
                Some(pkt) => {
                    // Wrong sequence — put it back
                    self.buffer[idx] = Some(pkt);
                    break;
                }
                None => break,
            }
        }
        ready
    }
}

/// Run the UDP audio server on the given port.
pub async fn run_audio_server(
    data_port: u16,
    _control_port: u16,
    decryptor: Arc<Mutex<AudioDecryptor>>,
) -> anyhow::Result<()> {
    let addr4 = format!("0.0.0.0:{}", data_port);
    let socket = UdpSocket::bind(&addr4).await?;
    let mut audio_file = std::fs::OpenOptions::new().create(true).append(true).open("audio.alac").ok();
    tracing::info!("Audio server listening on {}", addr4);

    let mut buf = vec![0u8; 65536];
    let mut reorder = AudioReorderBuffer::new();

    loop {
        let (len, peer) = socket.recv_from(&mut buf).await?;
        let data = &buf[..len];

        if data.len() < 12 { continue; }

        let seq = u16::from_be_bytes([data[2], data[3]]);
        let mut audio_data = data[12..].to_vec();

        {
            let mut dec = decryptor.lock().await;
            dec.decrypt(&mut audio_data);
        }

        reorder.insert(seq, audio_data);

        for pkt in reorder.dequeue() {
            if let Some(ref mut f) = audio_file {
                let _ = std::io::Write::write_all(f, &pkt.data);
            }
        }
    }
}

/// Run the UDP audio control server on the given port (lightweight).
pub async fn run_audio_control_server(port: u16) -> anyhow::Result<()> {
    let addr4 = format!("0.0.0.0:{}", port);
    let socket = UdpSocket::bind(&addr4).await?;
    tracing::info!("Audio control server listening on {}", addr4);

    let mut buf = vec![0u8; 2048];
    loop {
        let (len, peer) = socket.recv_from(&mut buf).await?;
        tracing::debug!("Audio control packet from {}: {} bytes", peer, len);
        // Audio control packets are used for sync/timing feedback.
        // For a basic implementation, we just acknowledge them.
    }
}
