// SPDX-License-Identifier: GPL-3.0-or-later
//! Receiver entry point: serves the control plane and announces itself.
//!
//! This is the configuration layer the library deliberately does not have. It
//! owns the long-term identity seed (the library never touches the filesystem,
//! see `docs/ADR-0002-pairing-identity.md`), picks the port, and drives
//! discovery so a client can find the receiver.

use std::collections::HashSet;
use std::io::Write;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};

/// Locks a lock this process shares, ignoring poisoning.
////// A callback that panicked once must not turn every later connection into a
/// panic: the receiver keeps serving the clients it can.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
use std::time::Duration;

use airplay_core::crypto::sha512_concat;
use airplay_core::discovery::{Advertisement, Advertiser, Protection};
use airplay_core::server::{
    DeviceIdentity, PairingState, PinMode, RouteHandlers, Server, ServerConfig, SetupPorts,
};

/// The stream type a mirroring client asks for, whose entry carries the
/// `streamConnectionID` the mirror keys are derived from
/// (`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:954-983`).
const MIRROR_STREAM_TYPE: u16 = 110;

const USAGE: &str = "\
rust-airplay - protocol development receiver

Usage: rust-airplay [options]

Options:
  --name <name>          Name clients show (default: rust-airplay)
  --port <port>          Control port, 0 picks a free one (default: 7000)
  --pin [<digits>]       Ask for a PIN, optionally fixed (default: one-time PIN)
  --no-pin               Do not ask for a PIN (legacy pairing only)
  --password <password>  Also require the SETUP digest password
  --device-id <address>  Advertised hardware address (default: from the seed)
  --seed <hex>           Identity seed, 32 bytes of hex (default: stored file)
  --log <file>           Append the request trace to a file, for archiving a session
  --help                 Print this text

The receiver answers the control plane: /info, /fp-setup, /feedback, the pairing
routes and SETUP with its key derivation. It does not play media yet, so a client
that completes SETUP will not hear anything.";

/// Everything the receiver needs before it starts.
struct Options {
    name: String,
    port: u16,
    pin: PinMode,
    password: Option<String>,
    device_id: Option<String>,
    seed: Option<[u8; 32]>,
    log: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            name: "rust-airplay".to_owned(),
            port: 7000,
            // A current client asks for a PIN, and the PIN is printed here so the
            // user can type it.
            pin: PinMode::OneTime,
            password: None,
            device_id: None,
            seed: None,
            log: None,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("rust-airplay: {message}");
            ExitCode::FAILURE
        }
    }
}

#[tokio::main]
async fn run() -> Result<(), String> {
    let options = match parse_arguments(std::env::args().skip(1))? {
        Some(options) => options,
        None => {
            println!("{USAGE}");
            return Ok(());
        }
    };

    let seed = match options.seed {
        Some(seed) => seed,
        None => load_or_create_seed()?,
    };
    let device_id = options
        .device_id
        .clone()
        .unwrap_or_else(|| derived_device_id(&seed));

    // The registry lives here, not in the library: a receiver decides for itself
    // how it remembers clients (`docs/ADR-0002-pairing-identity.md`). A client a
    // previous run verified may start `/pair-verify` on a fresh connection; one
    // that never completed a session is not known yet, which is the honest limit
    // of remembering keys only once a session carries them.
    let clients_path = clients_path()?;
    let known = Arc::new(Mutex::new(load_clients(&clients_path)));
    println!(
        "clients   {} remembered, {}",
        lock(&known).len(),
        clients_path.display()
    );

    let mut state = PairingState::new(seed, options.pin).with_client_registered({
        let known = Arc::clone(&known);
        move |key| lock(&known).contains(key)
    });
    if let Some(password) = options.password.as_deref() {
        state = state.with_password(password);
    }
    let state = Arc::new(state);
    let public_key = state.identity_public();

    let identity = DeviceIdentity::new(&device_id, &options.name, public_key.to_vec());
    let protection = match (options.pin, &options.password) {
        (PinMode::Disabled, None) => Protection::None,
        (PinMode::Disabled, Some(_)) => Protection::Password,
        _ => Protection::Pin,
    };

    let config = ServerConfig {
        bind: format!("0.0.0.0:{}", options.port)
            .parse()
            .map_err(|error| format!("bind address: {error}"))?,
        device: identity.clone(),
        ..ServerConfig::default()
    };
    let server = Server::bind(config)
        .await
        .map_err(|error| format!("cannot bind port {}: {error}", options.port))?;
    let address = server
        .local_addr()
        .map_err(|error| format!("cannot read the bound address: {error}"))?;

    println!("receiver  {} ({device_id})", options.name);
    println!("control   {address}");
    println!("identity  {}", hex(&public_key));
    match protection {
        Protection::Pin => println!("pairing   PIN required, watch this console"),
        Protection::Password => println!("pairing   password required"),
        Protection::None => println!("pairing   open (legacy pairing only)"),
    }

    // A device session is easier to archive than to describe, so the trace can be
    // appended to a file as well as the console. Nothing written there is a
    // secret: the lines carry what the client sent in the clear and the status it
    // got, and never a body, a key or a signature.
    let log = match options.log.as_deref() {
        Some(path) => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|error| format!("cannot open {path}: {error}"))?;
            println!("logging   {path}");
            Some(Arc::new(Mutex::new(file)))
        }
        None => None,
    };

    let advertisement = Advertisement::new(&identity, address.port(), protection);
    // The same value answers `GET /info?txtAirPlay` and is registered below, so a
    // client that reads the record from either place sees one advertisement.
    let (txt_airplay, txt_raop) = advertisement.txt_records();
    let identity = DeviceIdentity {
        txt_airplay: Some(txt_airplay),
        txt_raop: Some(txt_raop),
        ..identity
    };
    let advertiser = Advertiser::register(&advertisement)
        .map_err(|error| format!("cannot announce the receiver: {error}"))?;
    for fullname in advertiser.fullnames() {
        println!("advertised {fullname}");
    }

    // The PIN only exists once a client asks for one, so it is printed when it
    // appears rather than up front.
    let pin_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let mut shown: Option<String> = None;
        loop {
            let pin = pin_state.pending_pin();
            if pin != shown {
                if let Some(pin) = pin.as_deref() {
                    println!("*** enter PIN {pin} on the client ***");
                }
                shown = pin;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });

    let handlers_state = Arc::clone(&state);
    println!("waiting for a client (Ctrl+C to stop)");
    // Every line carries the time since the receiver started serving, because
    // "the client takes ages to connect" is a question about elapsed time and a
    // log without timestamps cannot answer it.
    let started = std::time::Instant::now();
    let trace_log = log.clone();

    // The media sockets are bound before anything is advertised, so the ports
    // `SETUP` names are ports that already answer. The receive loop waits for the
    // first `SETUP` that carries a key, because until then there is nothing to
    // decrypt with.
    let media =
        airplay_core::media::ports::MediaSockets::bind("0.0.0.0:0".parse().expect("bind address"))
            .await
            .map_err(|error| format!("cannot bind the media ports: {error}"))?;
    let setup_ports = SetupPorts {
        timing_port: Some(
            media
                .timing_port()
                .map_err(|error| format!("timing port: {error}"))?,
        ),
        control_port: Some(
            media
                .control_port()
                .map_err(|error| format!("control port: {error}"))?,
        ),
        data_port: Some(
            media
                .data_port()
                .map_err(|error| format!("data port: {error}"))?,
        ),
        // A mirroring client is told where to connect, so its stream arrives on a
        // port this receiver is already listening on.
        mirror_data_port: Some(
            media
                .mirror_port()
                .map_err(|error| format!("mirror port: {error}"))?,
        ),
        ..SetupPorts::default()
    };
    println!(
        "media     timing {} control {} data {} mirror {}",
        setup_ports.timing_port.unwrap_or_default(),
        setup_ports.control_port.unwrap_or_default(),
        setup_ports.data_port.unwrap_or_default(),
        setup_ports.mirror_data_port.unwrap_or_default()
    );
    // The three datagram sockets and the mirror listener are shared by the audio loop
    // and the mirror loop, which watch different ones of them.
    let media = Arc::new(media);
    let (media_key, media_ready) = tokio::sync::oneshot::channel::<([u8; 16], [u8; 16])>();
    let media_key = Arc::new(Mutex::new(Some(media_key)));
    // The mirror stream is keyed by the connection identifier a client sends in its
    // `type: 110` stream entry, so its keys can only be derived once that `SETUP`
    // has been accepted.
    let (mirror_keys, mirror_ready) =
        tokio::sync::oneshot::channel::<airplay_core::media::mirror::StreamKeys>();
    let mirror_keys = Arc::new(Mutex::new(Some(mirror_keys)));
    let audio_sockets = Arc::clone(&media);
    tokio::spawn(async move {
        let Ok((key, iv)) = media_ready.await else {
            return;
        };
        let mut session = airplay_core::media::ports::session(key, iv);
        let mut frames: u64 = 0;
        let outcome = audio_sockets
            .serve(&mut session, |_frame| {
                frames += 1;
                if frames % 500 == 0 {
                    println!("media     {frames} audio frames received");
                }
            })
            .await;
        if let Err(error) = outcome {
            eprintln!("rust-airplay: the media session stopped: {error}");
        }
    });

    // A mirroring client connects over TCP after its `SETUP`, and until it does
    // there is nothing to read from. Each connection gets a session of its own: a
    // client that reconnects starts its stream over.
    let mirror_sockets = Arc::clone(&media);
    tokio::spawn(async move {
        let Ok(keys) = mirror_ready.await else {
            return;
        };
        println!("*** mirror stream keyed, video can arrive ***");
        loop {
            let mut stream = match mirror_sockets.accept_mirror().await {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!("rust-airplay: the mirror listener stopped: {error}");
                    return;
                }
            };
            let mut session = match airplay_core::media::mirror::MirrorSession::new(&keys) {
                Ok(session) => session,
                Err(error) => {
                    eprintln!("rust-airplay: cannot start the mirror session: {error}");
                    return;
                }
            };
            let mut frames: u64 = 0;
            let outcome = stream
                .serve(&mut session, |frame| {
                    frames += 1;
                    if frames == 1 {
                        println!(
                            "media     first video frame: {} bytes, {} units, idr={}, ts={}",
                            frame.data.len(),
                            frame.units,
                            frame.idr,
                            frame.timestamp
                        );
                    } else if frames % 200 == 0 {
                        println!("media     {frames} video frames received");
                    }
                })
                .await;
            match outcome {
                // The client closing the connection is how a mirror session ends.
                Ok(()) => println!("media     the mirror stream ended after {frames} frames"),
                Err(error) => eprintln!("rust-airplay: the mirror stream stopped: {error}"),
            }
        }
    });

    let result = server
        .run(move || {
            let log = trace_log.clone();
            let mut handlers =
                RouteHandlers::with_pairing(identity.clone(), Arc::clone(&handlers_state));
            // The ports this run actually listens on, so a client is told where
            // to send instead of guessing.
            handlers.set_setup_ports(setup_ports);
            // A session that carries a verified client key is a client worth
            // remembering, so the next run can answer for it.
            {
                let known = Arc::clone(&known);
                let clients_path = clients_path.clone();
                let media_key = Arc::clone(&media_key);
                let mirror_keys = Arc::clone(&mirror_keys);
                handlers.set_setup_callback(move |request| {
                    // The session key starts the receive loop, once, however many
                    // connections a client opens.
                    if let (Some(key), Some(iv)) = (request.audio_key, request.audio_iv) {
                        if let Some(sender) = lock(&media_key).take() {
                            let _ = sender.send((key, iv));
                            println!("*** media session keyed, audio can arrive ***");
                        }
                    }
                    // The mirror stream's key is the audio key hashed with the
                    // connection identifier the client chose, so it is derived here
                    // where both are known.
                    let mirror_id = request
                        .streams
                        .iter()
                        .find(|stream| stream.kind == MIRROR_STREAM_TYPE)
                        .and_then(|stream| stream.stream_connection_id);
                    if let (Some(id), Some(key)) = (mirror_id, request.audio_key) {
                        if let Some(sender) = lock(&mirror_keys).take() {
                            let keys = airplay_core::media::mirror::stream_keys(id, &key);
                            let _ = sender.send(keys);
                        }
                    }
                    let Some(client) = request.client_identity else {
                        return;
                    };
                    if lock(&known).insert(client) {
                        println!("*** client {} is now remembered ***", hex(&client));
                        if let Err(error) = record_client(&clients_path, &client) {
                            eprintln!("rust-airplay: cannot record the client: {error}");
                        }
                    }
                });
            }
            // A trace names what a client asked and what it got, which is what
            // makes a device session readable; it never carries a body or key.
            handlers.set_trace_callback(move |trace| {
                // `Active-Remote`/`DACP-ID` name the remote-control connection a
                // client uses; they are shown because a device session is hard to
                // follow without them, not because they prove anything.
                let remote = match (trace.active_remote, trace.dacp_id) {
                    (None, None) => String::new(),
                    (active, dacp) => {
                        format!(" remote={}/{}", active.unwrap_or("-"), dacp.unwrap_or("-"))
                    }
                };
                let line = format!(
                    "[{:>7}ms] -> {:<14} {:<40} cseq={:<6} body={:<6} {}{}{} => {}{}",
                    started.elapsed().as_millis(),
                    trace.method,
                    trace.target,
                    trace
                        .cseq
                        .map(|cseq| cseq.to_string())
                        .unwrap_or_else(|| "-".to_owned()),
                    trace.body_len,
                    trace.content_type.unwrap_or("-"),
                    trace
                        .user_agent
                        .map(|agent| format!(" ua={agent}"))
                        .unwrap_or_default(),
                    if trace.status == 200 { "" } else { " !" },
                    trace.status,
                    remote,
                );
                println!("{line}");
                if let Some(log) = log.as_ref() {
                    let _ = writeln!(&mut *lock(log), "{line}");
                }
            });
            handlers
        })
        .await;
    advertiser.unregister().ok();
    result.map_err(|error| format!("the service stopped: {error}"))
}

/// Parses the command line, or returns `None` when the user asked for help.
fn parse_arguments(arguments: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options::default();
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--help" | "-h" => return Ok(None),
            "--name" => options.name = next_value(&mut arguments, "--name")?,
            "--port" => {
                options.port = next_value(&mut arguments, "--port")?
                    .parse()
                    .map_err(|error| format!("--port: {error}"))?;
            }
            "--no-pin" => options.pin = PinMode::Disabled,
            "--pin" => {
                // The digits are optional: without them the receiver draws a
                // one-time PIN and prints it when a client asks for one.
                options.pin = match arguments.peek() {
                    Some(value) if !value.starts_with('-') => {
                        let digits: u16 = next_value(&mut arguments, "--pin")?
                            .parse()
                            .map_err(|error| format!("--pin: {error}"))?;
                        PinMode::Fixed(digits)
                    }
                    _ => PinMode::OneTime,
                };
            }
            "--password" => options.password = Some(next_value(&mut arguments, "--password")?),
            "--device-id" => options.device_id = Some(next_value(&mut arguments, "--device-id")?),
            "--seed" => options.seed = Some(parse_seed(&next_value(&mut arguments, "--seed")?)?),
            "--log" => options.log = Some(next_value(&mut arguments, "--log")?),
            other => return Err(format!("unknown option {other}\n\n{USAGE}")),
        }
    }
    Ok(Some(options))
}

fn next_value(
    arguments: &mut std::iter::Peekable<impl Iterator<Item = String>>,
    option: &str,
) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{option} needs a value"))
}

/// Reads a 32 byte seed from hex.
fn parse_seed(text: &str) -> Result<[u8; 32], String> {
    let text = text.trim();
    if text.len() != 64 {
        return Err("--seed needs 64 hex characters (32 bytes)".to_owned());
    }
    let mut seed = [0u8; 32];
    for (index, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|error| format!("--seed: {error}"))?;
    }
    Ok(seed)
}

/// Path of the identity seed inside the user's local application data.
fn seed_path() -> Result<std::path::PathBuf, String> {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "neither LOCALAPPDATA nor HOME is set".to_owned())?;
    Ok(std::path::Path::new(&base)
        .join("rust-airplay")
        .join("identity.seed"))
}

/// Loads the identity seed, creating and storing one on first run.
///
/// The library never touches the filesystem; a receiver that had to be paired
/// again after every restart is what this file prevents
/// (`docs/ADR-0002-pairing-identity.md`).
fn load_or_create_seed() -> Result<[u8; 32], String> {
    let path = seed_path()?;
    if let Ok(text) = std::fs::read_to_string(&path) {
        return parse_seed(&text);
    }
    let mut seed = [0u8; 32];
    // `getrandom` is the same source the pairing service uses.
    getrandom::getrandom(&mut seed).map_err(|error| format!("no random bytes: {error}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    std::fs::write(&path, hex(&seed))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    println!("created a new identity at {}", path.display());
    Ok(seed)
}

/// Hardware address shown to clients, derived from the seed so that it stays the
/// same across restarts.
///
/// The first octet is a locally administered unicast address, because the value
/// is not a real interface address.
fn derived_device_id(seed: &[u8; 32]) -> String {
    let digest = sha512_concat(&[b"rust-airplay-device-id", seed]);
    let mut octets = [0u8; 6];
    octets.copy_from_slice(&digest[..6]);
    octets[0] = (octets[0] & 0xfc) | 0x02;
    octets
        .iter()
        .map(|octet| format!("{octet:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Path of the client registry.
fn clients_path() -> Result<std::path::PathBuf, String> {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "neither LOCALAPPDATA nor HOME is set".to_owned())?;
    Ok(std::path::Path::new(&base)
        .join("rust-airplay")
        .join("clients"))
}

/// Reads the remembered client keys, one 64 character hex key per line.
///
/// A line that is not a key is skipped rather than failing the receiver: a file
/// somebody edited by hand must not stop a receiver from starting.
fn load_clients(path: &std::path::Path) -> HashSet<[u8; 32]> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashSet::new();
    };
    text.lines()
        .filter_map(|line| parse_seed(line).ok())
        .collect()
}

/// Appends a client key, creating the file if it is the first one.
fn record_client(path: &std::path::Path, client: &[u8; 32]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    writeln!(file, "{}", hex(client))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rust-airplay-{name}-{}", std::process::id()))
    }

    #[test]
    fn a_recorded_client_is_remembered() {
        let path = temporary_path("registry");
        let _ = std::fs::remove_file(&path);
        let client = [0x2a; 32];
        assert!(load_clients(&path).is_empty(), "nothing is known at first");
        record_client(&path, &client).expect("record");
        let known = load_clients(&path);
        assert!(known.contains(&client));
        assert_eq!(known.len(), 1);
        // Recording the same client again does not fail, and does not duplicate.
        record_client(&path, &client).expect("record again");
        assert_eq!(load_clients(&path).len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_line_that_is_not_a_key_is_skipped() {
        let path = temporary_path("registry-edited");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            format!("# a comment\n{}\nnot a key\n", "ab".repeat(32)),
        )
        .expect("write");
        let known = load_clients(&path);
        assert_eq!(known.len(), 1, "the hand-edited lines are ignored");
        let _ = std::fs::remove_file(&path);
    }
}
