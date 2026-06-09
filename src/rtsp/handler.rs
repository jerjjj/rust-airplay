//! RTSP control server — raw TCP with manual RTSP parsing.
//! AirPlay uses RTSP protocol, NOT HTTP.

use std::sync::Arc;

use anyhow::Result;
use ctr::cipher::StreamCipher;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::config::Config;
use crate::pairing::PairingKeys;
use crate::rtsp::plist;
use crate::session::Session;
use crate::fairplay::FairPlayHandler;
use crate::RuntimeState;

pub struct RtspHandle { pub port: u16 }

pub struct AppState {
    pub config: Config,
    pub pairing_keys: PairingKeys,
    pub session: Mutex<Option<Session>>,
    pub fp_handler: FairPlayHandler,
    pub runtime: Arc<RuntimeState>,
}

pub async fn serve(config: &Config, runtime: Arc<RuntimeState>) -> Result<RtspHandle> {
    let state = Arc::new(AppState {
        config: config.clone(),
        pairing_keys: PairingKeys::generate(),
        session: Mutex::new(None),
        fp_handler: FairPlayHandler::new(),
        runtime,
    });

    let addr = format!("0.0.0.0:{}", config.airtunes_port);
    let listener = TcpListener::bind(&addr).await?;
    let port = listener.local_addr()?.port();
    tracing::info!("RTSP server on 0.0.0.0:{}", port);

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let s = state.clone();
                    tokio::spawn(async move { let _ = handle_connection(stream, peer, s).await; });
                }
                Err(e) => tracing::error!("Accept error: {}", e),
            }
        }
    });

    Ok(RtspHandle { port })
}

async fn handle_connection(mut stream: TcpStream, peer: std::net::SocketAddr, state: Arc<AppState>) -> Result<()> {
    let mut buf = vec![0u8; 65536];
    let mut len = 0usize;
    loop {
        let n = stream.read(&mut buf[len..]).await?;
        if n == 0 { break; }
        len += n;
        tracing::debug!("Received {} bytes from {}, total buffer: {}", n, peer, len);
        // Process complete messages
        while let Some(end) = find_msg_end(&buf[..len]) {
            let msg = &buf[..end];
            tracing::warn!("=== RTSP from {}: {} bytes ===", peer, msg.len());
            // Log first 200 bytes of message for debugging
            let preview = String::from_utf8_lossy(&msg[..msg.len().min(200)]);
            tracing::warn!("Message preview: {}", preview.replace('\r', "\\r").replace('\n', "\\n"));
            let _ = process_msg(&mut stream, &state, msg).await;
            let rem = len - end;
            if rem > 0 { buf.copy_within(end..len, 0); }
            len = rem;
        }
        if len > 60000 { 
            tracing::warn!("Buffer overflow, closing connection");
            break; 
        }
    }
    tracing::warn!("Connection from {} closed", peer);
    Ok(())
}

fn find_msg_end(data: &[u8]) -> Option<usize> {
    let he = data.windows(4).position(|w| w == b"\r\n\r\n")?;
    let headers = &data[..he];
    let cl = std::str::from_utf8(headers)
        .unwrap_or("")
        .lines()
        .find(|l| l.len() > 15 && l[..15].eq_ignore_ascii_case("content-length:"))
        .and_then(|l| l[15..].trim().parse::<usize>().ok())
        .unwrap_or(0);
    let total = he + 4 + cl;
    if data.len() >= total { Some(total) } else { None }
}

async fn process_msg(stream: &mut TcpStream, state: &Arc<AppState>, msg: &[u8]) -> Result<()> {
    // Auto-retry: if video decryption failed, full reset and close connection
    if state.runtime.needs_retry.load(std::sync::atomic::Ordering::SeqCst) {
        state.runtime.needs_retry.store(false, std::sync::atomic::Ordering::SeqCst);
        tracing::warn!("Auto-retry: fully resetting session and closing connection");
        // Full reset — set session to None so next pair-verify creates a fresh one
        *state.session.lock().await = None;
        state.runtime.video_decryptor.lock().await.cipher = None;
        state.runtime.audio_decryptor.lock().await.reset();
        // Close the connection so iPad reconnects fresh
        stream.write_all(b"RTSP/1.0 200 OK\r\nConnection: close\r\n\r\n").await?;
        let _ = stream.shutdown().await;
        return Ok(()); // Don't error — this is normal retry flow
    }

    let he = msg.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(msg.len());
    let body = if he + 4 < msg.len() { &msg[he + 4..] } else { &[] };
    let first = String::from_utf8_lossy(&msg[..he.min(256)])
        .lines().next().unwrap_or("").to_string();
    let parts: Vec<&str> = first.split_whitespace().collect();
    if parts.len() < 2 {
        stream.write_all(b"RTSP/1.0 400 Bad Request\r\n\r\n").await?;
        return Ok(());
    }
    let (method, path) = (parts[0], parts[1]);
    tracing::info!("RTSP {} {} body={}B", method, path, body.len());

    match (method, path) {
        ("OPTIONS", _) => {
            stream.write_all(b"RTSP/1.0 200 OK\r\nPublic: GET, POST, OPTIONS, SETUP, TEARDOWN, RECORD, GET_PARAMETER, SET_PARAMETER\r\n\r\n").await?;
        }
        ("GET", "/info") => {
            let data = plist::build_info_plist(&state.config);
            write_rtsp(stream, 200, plist::APPLE_BPLIST, &data).await?;
        }
        ("POST", "/pair-setup") => {
            let pk = state.pairing_keys.public_key_bytes();
            write_rtsp(stream, 200, "application/octet-stream", &pk).await?;
        }
        ("POST", "/pair-verify") => {
            handle_pair_verify(stream, state, body).await?;
        }
        ("POST", "/fp-setup") => {
            handle_fp_setup(stream, state, body).await?;
        }
        ("GET_PARAMETER", _) => {
            let req = String::from_utf8_lossy(body);
            tracing::warn!("GET_PARAMETER: '{}'", req.trim());
            // Respond with volume=0.0
            let answer = "volume: 0.000000\r\n";
            write_rtsp(stream, 200, "text/parameters", answer.as_bytes()).await?;
        }
        ("SET_PARAMETER", _) => {
            stream.write_all(b"RTSP/1.0 200 OK\r\n\r\n").await?;
        }
        ("SETUP", _) => {
            let mut sg = state.session.lock().await;
            if sg.as_ref().map_or(true, |s| !s.pair_verified) {
                tracing::warn!("SETUP rejected: not paired");
                stream.write_all(b"RTSP/1.0 401 Unauthorized\r\n\r\n").await?;
                return Ok(());
            }
            drop(sg);
            handle_setup(stream, state, body).await?;
        }
        ("TEARDOWN", _) => {
            tracing::info!("TEARDOWN received, clearing session and decryptors");
            let mut sg = state.session.lock().await;
            *sg = None;
            drop(sg);
            state.runtime.video_decryptor.lock().await.cipher = None;
            state.runtime.audio_decryptor.lock().await.reset();
            stream.write_all(b"RTSP/1.0 200 OK\r\n\r\n").await?;
        }
        ("RECORD", _) => {
            // RECORD — last chance to init video decryptor
            tracing::info!("RECORD ack");
            let mut sg = state.session.lock().await;
            if let Some(ref s) = *sg {
                if let (Some(ref ak), Some(ref ss), Some(conn_id)) = (&s.aes_key, &s.shared_secret, s.stream_connection_id) {
                    let mut vd = state.runtime.video_decryptor.lock().await;
                    if vd.cipher.is_none() {
                        vd.init(ak, ss, conn_id as i64);
                        tracing::warn!("Video decryptor INITIALIZED at RECORD (late init)");
                    }
                }
            }
            drop(sg);
            stream.write_all(b"RTSP/1.0 200 OK\r\n\r\n").await?;
        }
        ("POST", _) => {
            // POST /feedback, etc. — just ack
            tracing::debug!("POST {} ack", path);
            stream.write_all(b"RTSP/1.0 200 OK\r\n\r\n").await?;
        }
        _ => {
            tracing::warn!("Unknown: {} {}", method, path);
            stream.write_all(b"RTSP/1.0 501 Not Implemented\r\n\r\n").await?;
        }
    }
    Ok(())
}

async fn write_rtsp(stream: &mut TcpStream, status: u16, ct: &str, body: &[u8]) -> Result<()> {
    let header = format!("RTSP/1.0 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n",
        status, if status == 200 { "OK" } else { "No Content" }, ct, body.len());
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

async fn handle_pair_verify(stream: &mut TcpStream, state: &Arc<AppState>, body: &[u8]) -> Result<()> {
    use crate::pairing::PairVerifySession;
    use aes::cipher::KeyIvInit;

    if body.len() < 68 { stream.write_all(b"RTSP/1.0 400 Bad Request\r\n\r\n").await?; return Ok(()); }
    tracing::info!("pair-verify: {} bytes", body.len());

    let mut sg = state.session.lock().await;
    if sg.is_none() { *sg = Some(Session::new(PairingKeys::generate())); }
    let Some(s) = sg.as_mut() else { stream.write_all(b"RTSP/1.0 500\r\n\r\n").await?; return Ok(()); };

    let flag = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    if flag == 1 {
        tracing::info!("pair-verify ROUND1");
        let mut ce = [0u8;32]; let mut cd = [0u8;32];
        ce.copy_from_slice(&body[4..36]); cd.copy_from_slice(&body[36..68]);
        let r = PairVerifySession::round1(&state.pairing_keys, &ce);
        s.client_ecdh_public = Some(ce);
        s.client_ed25519_public = Some(cd);
        s.ecdh_public = Some(r.server_ecdh_public);
        s.shared_secret = Some(r.shared_secret);
        let mut rb = Vec::with_capacity(96);
        rb.extend_from_slice(&r.server_ecdh_public);
        rb.extend_from_slice(&r.encrypted_signature);
        write_rtsp(stream, 200, "application/octet-stream", &rb).await?;
        tracing::info!("pair-verify ROUND1 done");
    } else if flag == 0 {
        tracing::info!("pair-verify ROUND2");
        let mut cs = [0u8;64]; cs.copy_from_slice(&body[4..68]);
        let ce = s.client_ecdh_public.unwrap_or([0;32]);
        let cd = s.client_ed25519_public.unwrap_or([0;32]);
        let se = s.ecdh_public.unwrap_or([0;32]);
        let ss = s.shared_secret.unwrap_or([0;32]);
        let ak = crate::pairing::kdf_16(b"Pair-Verify-AES-Key", &ss);
        let ai = crate::pairing::kdf_16(b"Pair-Verify-AES-IV", &ss);
        let mut c = crate::pairing::Aes128Ctr::new(&ak.into(), &ai.into());
        // Skip first 64 bytes of keystream (used by round 1 for server's signature)
        let mut dummy = [0u8; 64];
        c.apply_keystream(&mut dummy);
        if PairVerifySession::round2(&mut c, &cs, &cd, &ce, &se) {
            s.pair_verified = true;
            stream.write_all(b"RTSP/1.0 200 OK\r\n\r\n").await?;
            tracing::info!("pair-verify ROUND2 OK");
        } else {
            stream.write_all(b"RTSP/1.0 401 Unauthorized\r\n\r\n").await?;
            tracing::error!("pair-verify ROUND2 FAIL");
        }
    } else {
        stream.write_all(b"RTSP/1.0 400 Bad Request\r\n\r\n").await?;
    }
    Ok(())
}

async fn handle_fp_setup(stream: &mut TcpStream, state: &Arc<AppState>, body: &[u8]) -> Result<()> {
    tracing::info!("fp-setup: {} bytes", body.len());
    let mut sg = state.session.lock().await;
    if sg.is_none() { *sg = Some(Session::new(PairingKeys::generate())); }
    let s = sg.as_mut().unwrap();
    let resp = state.fp_handler.handle_message(body, s);
    if resp.is_empty() {
        stream.write_all(b"RTSP/1.0 204 No Content\r\n\r\n").await?;
    } else {
        write_rtsp(stream, 200, "application/octet-stream", &resp).await?;
    }
    Ok(())
}

async fn handle_setup(stream: &mut TcpStream, state: &Arc<AppState>, body: &[u8]) -> Result<()> {
    tracing::warn!("=== SETUP/TEARDOWN: {} bytes ===", body.len());

    // Check if we need pairing
    let needs_pairing = {
        let sg = state.session.lock().await;
        sg.as_ref().map_or(true, |s| !s.pair_verified)
    };

    if body.is_empty() {
        stream.write_all(b"RTSP/1.0 200 OK\r\n\r\n").await?;
        return Ok(());
    }

    // If not paired and body contains keys (not ekey), require pairing
    if needs_pairing {
        if let Ok(v) = plist::Value::from_reader(std::io::Cursor::new(body)) {
            if let Some(d) = v.as_dictionary() {
                if d.contains_key("streams") {
                    tracing::warn!("SETUP rejected: not paired");
                    stream.write_all(b"RTSP/1.0 401 Unauthorized\r\n\r\n").await?;
                    return Ok(());
                }
            }
        }
    }
    if let Ok(v) = plist::Value::from_reader(std::io::Cursor::new(body)) {
        if let Some(d) = v.as_dictionary() {
            let keys: Vec<&str> = d.keys().map(|s| s.as_str()).collect();
            tracing::info!("SETUP keys: {:?}", keys);

            if d.contains_key("ekey") {
                let mut sg = state.session.lock().await;
                if let Some(ref mut s) = *sg {
                    if let Some(v) = d.get("eiv").and_then(|v| v.as_data()) { if v.len()>=16 { let mut e=[0u8;16]; e.copy_from_slice(&v[..16]); s.eiv=Some(e); } }
                    if let Some(v) = d.get("ekey").and_then(|v| v.as_data()) {
                        tracing::warn!("ekey len={} first 16: {:02x?}", v.len(), &v[..16.min(v.len())]);
                        let ak = match s.key_msg {
                            Some(ref km) => state.fp_handler.decrypt_aes_key(km, v),
                            None => {
                                tracing::warn!("No key_msg — fp-setup not completed, rejecting SETUP");
                                stream.write_all(b"RTSP/1.0 503 Service Unavailable\r\n\r\n").await?;
                                return Ok(());
                            }
                        };
                        s.aes_key = Some(ak);
                        tracing::warn!("AES key: {:02x?}", &ak[..]);
                            if let (Some(ref ss), Some(ref ei)) = (s.shared_secret, s.eiv) {
                                tracing::warn!("shared_secret FULL: {:02x?}", &ss[..]);
                                tracing::warn!("eiv: {:02x?}", &ei[..]);
                                state.runtime.audio_decryptor.lock().await.init(&ak, ss, ei);
                            }
                            // Also init video decryptor if stream_connection_id already received
                            if let (Some(ref ss), Some(conn_id)) = (s.shared_secret, s.stream_connection_id) {
                                let conn_id_signed = conn_id as i64;
                                tracing::warn!("Video decryptor INITIALIZED (ekey arrived after streams)");
                                state.runtime.video_decryptor.lock().await.init(&ak, ss, conn_id_signed);
                            }
                        }
                }
                stream.write_all(b"RTSP/1.0 200 OK\r\n\r\n").await?;
                return Ok(());
            }

            if d.contains_key("streams") {
                tracing::warn!("SETUP contains streams key — processing stream negotiation");
                if let Some(arr) = d.get("streams").and_then(|v| v.as_array()) {
                    for sv in arr {
                        if let Some(sd) = sv.as_dictionary() {
                            let st = sd.get("type").and_then(|v| v.as_unsigned_integer().map(|u| u as u32)
                                .or_else(|| v.as_signed_integer().map(|i| i as u32)));
                            match st {
                                Some(110) => {
                                    tracing::warn!("=== VIDEO STREAM SETUP ===");
                                    // Get streamConnectionID from request
                                    // Try unsigned first (plist often stores large IDs as unsigned), then signed
                                    let conn_id = sd.get("streamConnectionID")
                                        .and_then(|v| v.as_unsigned_integer().map(|u| u as i64)
                                            .or_else(|| v.as_signed_integer()))
                                        .unwrap_or(0);
                                    tracing::warn!("streamConnectionID: {}", conn_id);
                                    
                                    // Store stream info; video decryptor inits lazily when all keys ready
                                    let mut sg = state.session.lock().await;
                                    if let Some(ref mut s) = *sg {
                                        s.stream_connection_id = Some(conn_id as u64);
                                        
                                        if let (Some(ref ak), Some(ref ss)) = (s.aes_key, s.shared_secret) {
                                            tracing::warn!("Using session AES key: {:02x?}", &ak[..]);
                                            tracing::warn!("Using session shared_secret: {:02x?}", &ss[..]);
                                            state.runtime.video_decryptor.lock().await.init(ak, ss, conn_id);
                                            tracing::warn!("Video decryptor INITIALIZED with session values");
                                        } else {
                                            tracing::warn!("AES key not yet available — video decryptor will init when ekey arrives");
                                        }
                                    }
                                    
                                    let vp = state.runtime.video_data_port;
                                    let ep = state.runtime.video_event_port;
                                    tracing::warn!("Responding with video port={} event port={}", vp, ep);
                                    let b = plist::build_video_setup_response(vp, ep);
                                    write_rtsp(stream, 200, plist::APPLE_BPLIST, &b).await?;
                                    return Ok(());
                                }
                                Some(96) => {
                                    tracing::warn!("=== AUDIO STREAM SETUP ===");
                                    let ap = state.runtime.audio_data_port;
                                    let cp = state.runtime.audio_control_port;
                                    tracing::warn!("Responding with audio port={} control port={}", ap, cp);
                                    let b = plist::build_audio_setup_response(ap, cp);
                                    write_rtsp(stream, 200, plist::APPLE_BPLIST, &b).await?;
                                    return Ok(());
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }
    stream.write_all(b"RTSP/1.0 200 OK\r\n\r\n").await?;
    Ok(())
}
