// SPDX-License-Identifier: GPL-3.0-or-later
//! Receiver entry point: serves the control plane and announces itself.
//!
//! This is the configuration layer the library deliberately does not have. It
//! owns the long-term identity seed (the library never touches the filesystem,
//! see `docs/ADR-0002-pairing-identity.md`), picks the port, and drives
//! discovery so a client can find the receiver.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use airplay_core::crypto::sha512_concat;
use airplay_core::discovery::{Advertisement, Advertiser, Protection};
use airplay_core::server::{
    DeviceIdentity, PairingState, PinMode, RouteHandlers, Server, ServerConfig,
};

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

    let mut state = PairingState::new(seed, options.pin);
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
    let result = server
        .run(move || {
            let mut handlers =
                RouteHandlers::with_pairing(identity.clone(), Arc::clone(&handlers_state));
            // A trace names what a client asked and what it got, which is what
            // makes a device session readable; it never carries a body or key.
            handlers.set_trace_callback(|trace| {
                println!(
                    "  -> {:<14} {:<24} cseq={:<6} body={:<6} {}{}{} => {}",
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
                );
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
