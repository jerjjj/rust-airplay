// SPDX-License-Identifier: GPL-3.0-or-later
//! A command-line AirPlay receiver.
//!
//! Everything that makes a receiver a receiver is in the `rust-airplay` library: the
//! protocol, discovery, the media session and the renderer boundary. This program is the
//! policy around it — it parses arguments, decides where an identity lives, picks a
//! renderer, and prints. It contains no protocol code at all.
//!
//! See `README.md` for what the receiver does and `docs/API.md` for the library.

use std::collections::HashSet;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use rust_airplay::server::PinMode;
use rust_airplay::{AudioCodec, Renderer};

/// The receiver usage text.
const USAGE: &str = "\
rust-airplay - an AirPlay receiver

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
  --render <backend>     What plays the media: null (default) or gst
  --audio-sink <name>    GStreamer audio sink
  --video-sink <name>    GStreamer video sink
  --video-parser <name>  GStreamer parser for the mirror stream
  --video-decoder <name> GStreamer decoder for the mirror stream
  --warm-video           Start the video pipeline with the receiver
  --pace-video           Hold video to its capture spacing (smoother, later)

  --audio-codec <codec>  alac or aac-eld, what the client's frames are
  --help                 Print this text

`--render null` reads the media, decrypts and orders it, and plays nothing, which is how
a session is checked without a sound card. `--render gst` hands it to GStreamer, and only
a build with the `gst` feature has that backend.

Which audio coding a client sends is not something SETUP says, so it is given here:
`aac-eld` is what a current client sends, `alac` is what an AirPlay 1 one does.";

/// Everything the receiver needs before it starts.
#[derive(Debug, Clone)]
struct Options {
    name: String,
    port: u16,
    pin: PinMode,
    password: Option<String>,
    device_id: Option<String>,
    seed: Option<[u8; 32]>,
    log: Option<String>,
    render: RenderBackend,
    audio_codec: AudioCodec,
    /// Overrides for the GStreamer backend, when that is the one chosen.
    audio_sink: Option<String>,
    video_sink: Option<String>,
    video_parser: Option<String>,
    video_decoder: Option<String>,
    warm_video: bool,
    /// Whether the mirror stream is paced by its timestamps.
    pace_video: bool,
}

/// What plays the media a client sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderBackend {
    /// Read it, decrypt it, order it, and play nothing.
    Null,
    /// Hand it to GStreamer, in a build that has the backend.
    Gst,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            name: "rust-airplay".to_owned(),
            port: 7000,
            // A current client asks for a PIN, and the PIN is printed so the user can
            // type it.
            pin: PinMode::OneTime,
            password: None,
            device_id: None,
            seed: None,
            log: None,
            render: RenderBackend::Null,
            audio_sink: None,
            video_sink: None,
            video_parser: None,
            video_decoder: None,
            warm_video: false,
            pace_video: false,

            // AirPlay 2 clients send AAC-ELD, which is what an iPhone or iPad paired with
            // this receiver sends.
            audio_codec: AudioCodec::AacEld,
        }
    }
}

/// The renderer a session plays through, chosen from the options.
///
/// `--render gst` in a build without the feature is refused rather than quietly
/// downgraded: a receiver that plays nothing while its user believes it should be playing
/// is worse than one that says which build it is.
fn renderer(backend: RenderBackend, options: &Options) -> Result<Arc<dyn Renderer>, String> {
    match backend {
        RenderBackend::Null => Ok(Arc::new(rust_airplay::NullRenderer::new())),
        RenderBackend::Gst => open_gst(options),
    }
}

/// The GStreamer backend, when this build has it.
#[cfg(feature = "gst")]
fn open_gst(options: &Options) -> Result<Arc<dyn Renderer>, String> {
    // The defaults come from the environment, and a flag overrides one of them.
    let mut backend = rust_airplay_gstreamer::Options::default();
    if let Some(sink) = &options.audio_sink {
        backend.audio_sink = sink.clone();
    }
    // The sink decides the decoder: replacing one without the other would ask a software
    // decoder to feed a sink that only takes hardware memory, or the reverse.
    if let Some(sink) = &options.video_sink {
        backend = backend.with_video_sink(sink);
    }
    if let Some(parser) = &options.video_parser {
        backend = backend.with_video_parser(parser);
    }
    if let Some(decoder) = &options.video_decoder {
        backend = backend.with_video_decoder(decoder);
    }
    if options.warm_video {
        backend.warm_video = true;
    }

    let program = rust_airplay_gstreamer::GstRenderer::find_launcher()
        .map_err(|error| format!("--render gst: {error}"))?;
    let renderer = rust_airplay_gstreamer::GstRenderer::with_options(&program, backend)
        .map_err(|error| format!("--render gst: {error}"))?;
    println!(
        "render    gst    {} -> audio {}, video {}",
        renderer.program(),
        renderer.audio_sink(),
        renderer.video_sink()
    );
    // Whether the video pipeline is already up is the difference between a picture that
    // appears at once and one that takes about a second, so it is worth saying which of
    // the two a session will get.
    println!(
        "render    video  {}",
        if renderer.video_is_warm() {
            "warm (started before any client, so a connection does not wait for a window)"
        } else {
            "cold (started when a client connects; AIRPLAY_GST_VIDEO_WARM=1 starts it now)"
        }
    );
    Ok(Arc::new(renderer))
}

/// The GStreamer backend, which this build does not have.
#[cfg(not(feature = "gst"))]
fn open_gst(options: &Options) -> Result<Arc<dyn Renderer>, String> {
    let _ = options;
    Err("--render gst needs a build with the `gst` feature: \
         cargo run -p rust-airplay-cli --features gst"
        .to_owned())
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

/// Starts a receiver from the command line.
#[tokio::main]
async fn run() -> Result<(), String> {
    let options = match parse_arguments(std::env::args().skip(1))? {
        Some(options) => options,
        None => {
            println!("{USAGE}");
            return Ok(());
        }
    };
    install_logger();

    let seed = match options.seed {
        Some(seed) => seed,
        None => load_or_create_seed()?,
    };
    // The renderer is chosen before anything is advertised, because a backend this build
    // or this machine cannot provide is something to say now rather than when a client is
    // already waiting for sound.
    let renderer = renderer(options.render, &options)?;
    if options.render == RenderBackend::Null {
        println!("render    null   media is read and played nowhere");
    }

    // The registry lives here, not in the library: a receiver decides for itself how it
    // remembers clients (`docs/ARCHITECTURE.md`). The library is handed the
    // set, and a callback that writes a newly verified key.
    let clients_file = clients_path()?;
    let clients = Arc::new(Mutex::new(load_clients(&clients_file)));
    let on_client: rust_airplay::receiver::ClientCallback = {
        let path = clients_file.clone();
        Arc::new(move |client: &[u8; 32]| {
            if let Err(error) = record_client(&path, client) {
                eprintln!("rust-airplay: cannot record the client: {error}");
            }
        })
    };

    let config = rust_airplay::Config {
        name: options.name,
        port: options.port,
        seed,
        device_id: options.device_id,
        pin: options.pin,
        password: options.password,
        audio_codec: options.audio_codec,
        renderer,
        clients,
        on_client: Some(on_client),
        trace_log: options.log.map(std::path::PathBuf::from),
        dump_video: std::env::var_os("AIRPLAY_DUMP_VIDEO").map(std::path::PathBuf::from),
        dump_audio: std::env::var_os("AIRPLAY_DUMP_AUDIO").map(std::path::PathBuf::from),
        pace_video: options.pace_video,
    };
    rust_airplay::run(config)
        .await
        .map_err(|error| error.to_string())
}

/// Prints the library's records the way a terminal wants them.
///
/// The library emits `log` records rather than printing, because a library that writes to
/// standard output takes that decision away from the program embedding it.
fn install_logger() {
    struct Terminal;

    impl log::Log for Terminal {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }

        fn log(&self, record: &log::Record<'_>) {
            if record.level() <= log::Level::Info {
                println!("{}", record.args());
            } else {
                eprintln!("{}", record.args());
            }
        }

        fn flush(&self) {}
    }

    let _ = log::set_logger(&Terminal).map(|()| log::set_max_level(log::LevelFilter::Info));
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

/// Lower-case hex, for the one file this program writes.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The directory this program keeps its files in.
fn data_directory() -> Result<std::path::PathBuf, String> {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "neither LOCALAPPDATA nor HOME is set".to_owned())?;
    Ok(std::path::Path::new(&base).join("rust-airplay"))
}

/// Path of the identity seed inside the user's local application data.
fn seed_path() -> Result<std::path::PathBuf, String> {
    Ok(data_directory()?.join("identity.seed"))
}

/// Loads the identity seed, creating and storing one on first run.
///
/// The library never touches the filesystem. A receiver that had to be paired again after
/// every restart is what this file prevents.
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

/// Path of the client registry.
fn clients_path() -> Result<std::path::PathBuf, String> {
    Ok(data_directory()?.join("clients"))
}

/// Reads the remembered client keys, one 64 character hex key per line.
///
/// A line that is not a key is skipped rather than failing the receiver: a file somebody
/// edited by hand must not stop a receiver from starting.
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
    use std::io::Write;
    writeln!(file, "{}", hex(client))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
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
                // The digits are optional: without them the receiver draws a one-time PIN
                // and prints it when a client asks for one.
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
            "--render" => {
                let name = next_value(&mut arguments, "--render")?;
                options.render = match name.as_str() {
                    "null" => RenderBackend::Null,
                    "gst" => RenderBackend::Gst,
                    other => {
                        return Err(format!("--render: {other} is not a backend (null or gst)"));
                    }
                };
            }
            "--audio-sink" => {
                options.audio_sink = Some(next_value(&mut arguments, "--audio-sink")?)
            }
            "--video-sink" => {
                options.video_sink = Some(next_value(&mut arguments, "--video-sink")?)
            }
            "--video-parser" => {
                options.video_parser = Some(next_value(&mut arguments, "--video-parser")?);
            }
            "--video-decoder" => {
                options.video_decoder = Some(next_value(&mut arguments, "--video-decoder")?);
            }
            "--warm-video" => options.warm_video = true,
            "--pace-video" => options.pace_video = true,

            "--audio-codec" => {
                let name = next_value(&mut arguments, "--audio-codec")?;
                options.audio_codec = match name.as_str() {
                    "alac" => AudioCodec::Alac,
                    "aac-eld" => AudioCodec::AacEld,
                    other => {
                        return Err(format!(
                            "--audio-codec: {other} is not a coding (alac or aac-eld)"
                        ));
                    }
                };
            }
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
