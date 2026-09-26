// SPDX-License-Identifier: GPL-3.0-or-later
//! End-to-end checks of the network control service.
//!
//! Every case opens a real TCP connection to a [`Server`] the test bound to an
//! ephemeral port, so the framing, the bounded reads and the connection
//! lifecycle are exercised the way a client exercises them, not through the
//! control layer alone.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use airplay_core::plist::Plist;
use airplay_core::server::{DeviceIdentity, RouteHandlers, Server, ServerConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// A bound server plus the address it listens on.
struct TestServer {
    address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<io::Result<()>>,
}

impl TestServer {
    async fn start(config: ServerConfig, handlers: RouteHandlers) -> Self {
        let server = Server::bind(config).await.expect("bind");
        let address = server.local_addr().expect("local_addr");
        let identity = handlers.identity().clone();
        let task = tokio::spawn(async move {
            server
                .run(move || {
                    let mut handlers = RouteHandlers::new(identity.clone());
                    handlers.set_feedback_callback(|| {});
                    handlers
                })
                .await
        });
        Self { address, task }
    }

    async fn connect(&self) -> TcpStream {
        TcpStream::connect(self.address).await.expect("connect")
    }

    fn shutdown(self) {
        self.task.abort();
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Reads one complete response from `stream`, keeping unconsumed bytes in
/// `pending`.
///
/// Returns `None` when the server closed the connection, or when nothing
/// arrived within `wait`.
async fn read_response(
    stream: &mut TcpStream,
    pending: &mut Vec<u8>,
    wait: Duration,
) -> Option<Vec<u8>> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        if let Some(end) = find(pending, b"\r\n\r\n") {
            let declared = header_value(&pending[..end], "content-length")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let total = end + 4 + declared;
            if pending.len() >= total {
                let message = pending[..total].to_vec();
                pending.drain(..total);
                return Some(message);
            }
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let mut chunk = [0u8; 4096];
        match timeout(remaining, stream.read(&mut chunk)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return None,
            Ok(Ok(read)) => pending.extend_from_slice(&chunk[..read]),
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn header_value<'a>(head: &'a [u8], name: &str) -> Option<&'a str> {
    let text = std::str::from_utf8(head).ok()?;
    text.split("\r\n").skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn identity() -> DeviceIdentity {
    DeviceIdentity::new("aa:bb:cc:dd:ee:ff", "Integration Receiver", vec![9; 32])
}

fn local_config() -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".parse().expect("address"),
        idle_timeout: Duration::from_secs(30),
        ..ServerConfig::default()
    }
}

#[tokio::test]
async fn answers_get_info_over_the_socket() {
    let server = TestServer::start(local_config(), RouteHandlers::new(identity())).await;
    let mut client = server.connect().await;
    client
        .write_all(b"GET /info RTSP/1.0\r\nCSeq: 1\r\n\r\n")
        .await
        .expect("write");

    let mut pending = Vec::new();
    let response = read_response(&mut client, &mut pending, Duration::from_secs(5))
        .await
        .expect("response");
    // The body is a binary plist, so only the head is text.
    let head_end = find(&response, b"\r\n\r\n").expect("header terminator");
    assert!(
        response.starts_with(b"RTSP/1.0 200 OK\r\n"),
        "unexpected status line: {}",
        String::from_utf8_lossy(&response[..head_end])
    );
    assert_eq!(
        header_value(&response[..head_end], "Audio-Jack-Status"),
        Some("connected; type=digital")
    );
    assert_eq!(
        header_value(&response[..head_end], "Server"),
        Some("AirTunes/220.68")
    );
    assert_eq!(header_value(&response[..head_end], "CSeq"), Some("1"));
    assert_eq!(
        header_value(&response[..head_end], "Content-Length"),
        Some((response.len() - head_end - 4).to_string().as_str())
    );

    let parsed = Plist::parse(&response[head_end + 4..]).expect("binary plist");
    assert_eq!(
        parsed.get_string("deviceID").expect("deviceID"),
        "aa:bb:cc:dd:ee:ff"
    );
    assert_eq!(parsed.get_data("pk").expect("pk"), vec![9; 32]);
    server.shutdown();
}

#[tokio::test]
async fn frames_two_pipelined_requests_written_at_once() {
    let server = TestServer::start(local_config(), RouteHandlers::new(identity())).await;
    let mut client = server.connect().await;
    client
        .write_all(b"OPTIONS * RTSP/1.0\r\nCSeq: 1\r\n\r\nGET /info RTSP/1.0\r\nCSeq: 2\r\n\r\n")
        .await
        .expect("write");

    let mut pending = Vec::new();
    let first = read_response(&mut client, &mut pending, Duration::from_secs(5))
        .await
        .expect("first response");
    let second = read_response(&mut client, &mut pending, Duration::from_secs(5))
        .await
        .expect("second response");

    let first = String::from_utf8_lossy(&first).to_string();
    assert!(first.contains("CSeq: 1\r\n"));
    assert!(first.contains(
        "Public: SETUP, RECORD, FLUSH, TEARDOWN, OPTIONS, GET_PARAMETER, SET_PARAMETER\r\n"
    ));
    let second = String::from_utf8_lossy(&second).to_string();
    assert!(second.contains("CSeq: 2\r\n"));
    assert!(second.contains("Content-Type: application/x-apple-binary-plist\r\n"));
    server.shutdown();
}

#[tokio::test]
async fn waits_for_a_request_split_across_writes() {
    let server = TestServer::start(local_config(), RouteHandlers::new(identity())).await;
    let mut client = server.connect().await;
    client
        .write_all(b"GET /info RTSP/1.0\r\nCSeq: ")
        .await
        .expect("first half");
    let mut pending = Vec::new();
    assert!(
        read_response(&mut client, &mut pending, Duration::from_millis(150))
            .await
            .is_none(),
        "a partial request must not be answered"
    );
    client.write_all(b"7\r\n\r\n").await.expect("second half");
    let response = read_response(&mut client, &mut pending, Duration::from_secs(5))
        .await
        .expect("response");
    assert!(String::from_utf8_lossy(&response).contains("CSeq: 7\r\n"));
    server.shutdown();
}

#[tokio::test]
async fn rejects_an_oversized_head_and_closes() {
    let server = TestServer::start(local_config(), RouteHandlers::new(identity())).await;
    let mut client = server.connect().await;
    let oversized = vec![b'a'; 80 * 1024];
    // The server closes as soon as it rejects the head, so the tail of this
    // write may fail with a broken pipe; the response is what matters.
    let _ = client.write_all(&oversized).await;

    let mut pending = Vec::new();
    let response = read_response(&mut client, &mut pending, Duration::from_secs(5))
        .await
        .expect("rejection");
    let text = String::from_utf8_lossy(&response).to_string();
    assert!(
        text.starts_with("RTSP/1.0 431 Request Header Fields Too Large\r\n"),
        "{text}"
    );
    assert!(!text.contains("Server:"));
    server.shutdown();
}

#[tokio::test]
async fn rejects_a_malformed_request_and_closes() {
    let server = TestServer::start(local_config(), RouteHandlers::new(identity())).await;
    let mut client = server.connect().await;
    client
        .write_all(b"GET /info RTSP/1.0\r\nBad Header\r\n\r\n")
        .await
        .expect("write");

    let mut pending = Vec::new();
    let response = read_response(&mut client, &mut pending, Duration::from_secs(5))
        .await
        .expect("rejection");
    assert!(
        String::from_utf8_lossy(&response).starts_with("RTSP/1.0 400 Bad Request\r\n"),
        "{}",
        String::from_utf8_lossy(&response)
    );
    // The connection is closed, so the next read sees end of stream.
    let mut rest = Vec::new();
    assert_eq!(client.read_to_end(&mut rest).await.expect("read"), 0);
    server.shutdown();
}

#[tokio::test]
async fn teardown_closes_the_connection() {
    let server = TestServer::start(local_config(), RouteHandlers::new(identity())).await;
    let mut client = server.connect().await;
    client
        .write_all(b"TEARDOWN * RTSP/1.0\r\nCSeq: 4\r\n\r\n")
        .await
        .expect("write");

    let mut pending = Vec::new();
    let response = read_response(&mut client, &mut pending, Duration::from_secs(5))
        .await
        .expect("response");
    assert!(String::from_utf8_lossy(&response).contains("Connection: close\r\n"));
    let mut rest = Vec::new();
    assert_eq!(client.read_to_end(&mut rest).await.expect("read"), 0);
    server.shutdown();
}

#[tokio::test]
async fn reports_feedback_to_the_application() {
    let counter = Arc::new(AtomicUsize::new(0));
    let server = Server::bind(local_config()).await.expect("bind");
    let address = server.local_addr().expect("local_addr");
    let task = {
        let counter = Arc::clone(&counter);
        let identity = identity();
        tokio::spawn(async move {
            server
                .run(move || {
                    let mut handlers = RouteHandlers::new(identity.clone());
                    let counter = Arc::clone(&counter);
                    handlers.set_feedback_callback(move || {
                        counter.fetch_add(1, Ordering::SeqCst);
                    });
                    handlers
                })
                .await
        })
    };

    let mut client = TcpStream::connect(address).await.expect("connect");
    client
        .write_all(b"POST /feedback RTSP/1.0\r\nCSeq: 9\r\nContent-Length: 0\r\n\r\n")
        .await
        .expect("write");
    let mut pending = Vec::new();
    let response = read_response(&mut client, &mut pending, Duration::from_secs(5))
        .await
        .expect("response");
    assert!(String::from_utf8_lossy(&response).starts_with("RTSP/1.0 200 OK\r\n"));
    // The callback runs on the server task, so give it a moment to be observed.
    for _ in 0..50 {
        if counter.load(Ordering::SeqCst) == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn answers_fairplay_setup_with_its_opaque_reply() {
    let server = TestServer::start(local_config(), RouteHandlers::new(identity())).await;
    let mut client = server.connect().await;
    let mut challenge = [0u8; 16];
    challenge[4] = 0x03;
    let mut request = Vec::new();
    request.extend_from_slice(b"POST /fp-setup RTSP/1.0\r\nCSeq: 3\r\nContent-Length: 16\r\n\r\n");
    request.extend_from_slice(&challenge);
    client.write_all(&request).await.expect("write");

    let mut pending = Vec::new();
    let response = read_response(&mut client, &mut pending, Duration::from_secs(5))
        .await
        .expect("response");
    let head_end = find(&response, b"\r\n\r\n").expect("header terminator");
    assert_eq!(
        header_value(&response[..head_end], "Content-Type"),
        Some("application/octet-stream")
    );
    assert_eq!(response.len() - head_end - 4, 142);
    server.shutdown();
}

#[tokio::test]
async fn serves_no_more_than_the_configured_number_of_connections() {
    let config = ServerConfig {
        max_connections: 1,
        ..local_config()
    };
    let server = TestServer::start(config, RouteHandlers::new(identity())).await;

    // This connection takes the only slot and then stays quiet.
    let mut first = server.connect().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut second = server.connect().await;
    second
        .write_all(b"GET /info RTSP/1.0\r\nCSeq: 5\r\n\r\n")
        .await
        .expect("write");
    let mut pending = Vec::new();
    assert!(
        read_response(&mut second, &mut pending, Duration::from_millis(300))
            .await
            .is_none(),
        "a connection past the limit must not be served"
    );

    // Closing the first connection frees the slot, and the second is served.
    first.shutdown().await.expect("shutdown");
    drop(first);
    let response = read_response(&mut second, &mut pending, Duration::from_secs(5))
        .await
        .expect("response once a slot is free");
    assert!(String::from_utf8_lossy(&response).starts_with("RTSP/1.0 200 OK\r\n"));
    server.shutdown();
}
