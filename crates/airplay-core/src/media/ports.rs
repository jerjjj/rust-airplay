// SPDX-License-Identifier: GPL-3.0-or-later
//! The sockets one media session listens on.
//!
//! A client does not send media anywhere it likes: `SETUP` tells it which ports
//! to use, and until now this port told it nothing, because nothing was
//! listening. This module binds the three sockets an audio session needs and
//! drives the receive path over them, so the ports a receiver advertises are the
//! ports that answer:
//!
//! - the **timing** port, where the clock exchange happens (`media::timing`),
//! - the **control** port, where resends and sync packets arrive
//!   (`media::control`),
//! - the **data** port, where the audio datagrams arrive (`media::audio`).
//!
//! Upstream opens exactly these three for an audio session
//! (`upstream/UxPlay@3dbf7cee:lib/raop_rtp.c:236-244`, which binds a control and
//! a data socket beside the timing socket of `raop_ntp.c`). The mirror stream
//! adds a TCP listener, and arrives with its own slice.
//!
//! Nothing here decides policy: the key and IV come from the `SETUP` that a
//! caller accepted, and frames go to a caller's callback, which is where a
//! renderer plugs in ([ADR-0003](../../../docs/ADR-0003-media-backend.md)).

use std::net::SocketAddr;

use tokio::net::UdpSocket;

use super::audio::{AudioFrame, AudioSession, WINDOW_CAPACITY};

/// Largest datagram this receiver reads.
///
/// Upstream reads into a fixed buffer too (`raop_rtp.c`), and an audio datagram
/// is far below this: the bound exists so a client cannot make a receiver
/// allocate by sending something enormous.
pub const MAX_DATAGRAM: usize = 2048;

/// The sockets one audio session listens on.
#[derive(Debug)]
pub struct MediaSockets {
    timing: UdpSocket,
    control: UdpSocket,
    data: UdpSocket,
}

impl MediaSockets {
    /// Binds the three sockets on `bind`, letting the system pick the ports.
    ///
    /// # Errors
    ///
    /// Returns the underlying error when a socket cannot be bound.
    pub async fn bind(bind: SocketAddr) -> std::io::Result<Self> {
        Ok(Self {
            timing: UdpSocket::bind(bind).await?,
            control: UdpSocket::bind(bind).await?,
            data: UdpSocket::bind(bind).await?,
        })
    }

    /// Port of the timing socket.
    ///
    /// # Errors
    ///
    /// Returns the underlying error when the socket has no local address.
    pub fn timing_port(&self) -> std::io::Result<u16> {
        Ok(self.timing.local_addr()?.port())
    }

    /// Port of the control socket.
    ///
    /// # Errors
    ///
    /// Returns the underlying error when the socket has no local address.
    pub fn control_port(&self) -> std::io::Result<u16> {
        Ok(self.control.local_addr()?.port())
    }

    /// Port of the data socket.
    ///
    /// # Errors
    ///
    /// Returns the underlying error when the socket has no local address.
    pub fn data_port(&self) -> std::io::Result<u16> {
        Ok(self.data.local_addr()?.port())
    }

    /// Receives one audio datagram and returns what became ready.
    ///
    /// # Errors
    ///
    /// Returns the underlying error when the socket cannot be read; a datagram
    /// that is not audio is not an error, it is ignored the way a receiver
    /// ignores traffic it cannot use.
    pub async fn receive_audio(
        &self,
        session: &mut AudioSession,
    ) -> std::io::Result<Vec<AudioFrame>> {
        let mut buffer = [0u8; MAX_DATAGRAM];
        let length = self.data.recv(&mut buffer).await?;
        Ok(session.receive(&buffer[..length]).unwrap_or_default())
    }

    /// Receives one control datagram and returns what became ready.
    ///
    /// # Errors
    ///
    /// Returns the underlying error when the socket cannot be read.
    pub async fn receive_control(
        &self,
        session: &mut AudioSession,
    ) -> std::io::Result<Vec<AudioFrame>> {
        let mut buffer = [0u8; MAX_DATAGRAM];
        let length = self.control.recv(&mut buffer).await?;
        Ok(session
            .receive_control(&buffer[..length])
            .unwrap_or_default())
    }

    /// Receives one timing datagram, which is an answer to a request this
    /// receiver sent.
    ///
    /// # Errors
    ///
    /// Returns the underlying error when the socket cannot be read.
    pub async fn receive_timing(&self) -> std::io::Result<Vec<u8>> {
        let mut buffer = [0u8; MAX_DATAGRAM];
        let length = self.timing.recv(&mut buffer).await?;
        Ok(buffer[..length].to_vec())
    }
}

/// Creates a session for a `SETUP` that carried `key` and `iv`.
pub fn session(key: [u8; 16], iv: [u8; 16]) -> AudioSession {
    AudioSession::new(key, iv, WINDOW_CAPACITY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::audio::AUDIO_PAYLOAD_TYPE;

    /// A datagram with the audio header upstream documents, encrypted with the
    /// port's own primitive so the test speaks the client's side of the wire.
    fn datagram(sequence: u16, timestamp: u32, key: &[u8; 16], iv: &[u8; 16]) -> Vec<u8> {
        use aes::cipher::{BlockEncryptMut, KeyIvInit};
        let plaintext = [0x8c; 16];
        let mut block = plaintext;
        cbc::Encryptor::<aes::Aes128>::new(key.into(), iv.into())
            .encrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut block, 16)
            .expect("one block");
        let mut bytes = vec![0x80, AUDIO_PAYLOAD_TYPE];
        bytes.extend_from_slice(&sequence.to_be_bytes());
        bytes.extend_from_slice(&timestamp.to_be_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(&block);
        bytes
    }

    #[tokio::test]
    async fn binds_three_distinct_ports() {
        let sockets = MediaSockets::bind("127.0.0.1:0".parse().expect("address"))
            .await
            .expect("bind");
        let ports = [
            sockets.timing_port().expect("timing"),
            sockets.control_port().expect("control"),
            sockets.data_port().expect("data"),
        ];
        assert!(ports.iter().all(|port| *port != 0));
        let mut unique = ports.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 3, "each socket has its own port");
    }

    #[tokio::test]
    async fn audio_arrives_decrypted_and_in_order_over_the_data_port() {
        let key = [0x21; 16];
        let iv = [0x43; 16];
        let sockets = MediaSockets::bind("127.0.0.1:0".parse().expect("address"))
            .await
            .expect("bind");
        let mut session = session(key, iv);
        let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");
        let data: SocketAddr = format!("127.0.0.1:{}", sockets.data_port().expect("port"))
            .parse()
            .expect("address");

        // The first packet fixes the starting point, so it arrives first; the
        // third one then waits for the second.
        client
            .send_to(&datagram(1, 480, &key, &iv), data)
            .await
            .expect("send");
        let first = sockets.receive_audio(&mut session).await.expect("receive");
        assert_eq!(first.len(), 1);
        client
            .send_to(&datagram(3, 1440, &key, &iv), data)
            .await
            .expect("send");
        let frames = sockets.receive_audio(&mut session).await.expect("receive");
        assert!(frames.is_empty(), "3 waits for 2");

        client
            .send_to(&datagram(2, 960, &key, &iv), data)
            .await
            .expect("send");
        let frames = sockets.receive_audio(&mut session).await.expect("receive");
        assert_eq!(
            frames
                .iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert!(
            frames.iter().all(|frame| frame.payload == vec![0x8c; 16]),
            "delivered decrypted"
        );
    }

    #[tokio::test]
    async fn a_resend_on_the_control_port_closes_a_gap() {
        let key = [0x65; 16];
        let iv = [0x87; 16];
        let sockets = MediaSockets::bind("127.0.0.1:0".parse().expect("address"))
            .await
            .expect("bind");
        let mut session = session(key, iv);
        let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");
        let data: SocketAddr = format!("127.0.0.1:{}", sockets.data_port().expect("port"))
            .parse()
            .expect("address");
        let control: SocketAddr = format!("127.0.0.1:{}", sockets.control_port().expect("port"))
            .parse()
            .expect("address");

        client
            .send_to(&datagram(1, 480, &key, &iv), data)
            .await
            .expect("send");
        assert_eq!(
            sockets
                .receive_audio(&mut session)
                .await
                .expect("first")
                .len(),
            1
        );
        // 3 arrives, 2 is missing and the receiver would ask for it.
        client
            .send_to(&datagram(3, 1440, &key, &iv), data)
            .await
            .expect("send");
        assert!(
            sockets
                .receive_audio(&mut session)
                .await
                .expect("third")
                .is_empty()
        );

        // The client answers on the control port, with the audio at offset four.
        let mut wrapped = vec![0x80, 0x56 | 0x80, 0x00, 0x00];
        wrapped.extend_from_slice(&datagram(2, 960, &key, &iv));
        client.send_to(&wrapped, control).await.expect("send");
        let frames = sockets.receive_control(&mut session).await.expect("resent");
        assert_eq!(
            frames
                .iter()
                .map(|frame| frame.sequence)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert!(session.take_lost().is_empty(), "the gap was filled");
    }

    #[tokio::test]
    async fn the_timing_port_hands_the_datagram_to_the_caller() {
        let sockets = MediaSockets::bind("127.0.0.1:0".parse().expect("address"))
            .await
            .expect("bind");
        let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");
        let timing: SocketAddr = format!("127.0.0.1:{}", sockets.timing_port().expect("port"))
            .parse()
            .expect("address");
        let response = [0x80, 0x53 | 0x80, 0x00, 0x07];
        client.send_to(&response, timing).await.expect("send");
        assert_eq!(
            sockets.receive_timing().await.expect("timing"),
            response.to_vec()
        );
    }
}
