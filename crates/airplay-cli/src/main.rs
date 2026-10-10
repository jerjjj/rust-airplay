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
use airplay_renderer::{AudioCodec, AudioConfig};

/// The stream type a mirroring client asks for, whose entry carries the
/// `streamConnectionID` the mirror keys are derived from
/// (`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:954-983`).
const MIRROR_STREAM_TYPE: u16 = 110;

/// Frames a mirror session may hold for its renderer before it starts dropping them.
///
/// The queue exists so that waiting for a picture is not waiting inside the receive
/// loop. It is bounded because a client that sends faster than a decoder decodes must
/// not be able to make a receiver allocate without limit; a frame dropped when it is
/// full is reported, because a dropped frame is one the rest of the stream may depend
/// on. Thirty-two frames is about a second of a screen, which is longer than any
/// stall a decoder produces and far below anything a receiver would notice holding.
const QUEUED_FRAMES: usize = 32;

/// How many audio frames a writer thread will hold before frames start being dropped.
///
/// Audio packets are small and arrive in bursts, so this is deeper than the video
/// queue: at 44.1 kHz a frame is a few milliseconds, and a second of slack is what a
/// decoder stall needs. It is still bounded, because an unbounded queue turns "the
/// pipeline has stopped" into a growing delay rather than a visible fault.
const QUEUED_AUDIO_FRAMES: usize = 256;

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
  --render <backend>     What plays the media: null (default) or gst
  --audio-codec <codec>  alac or aac-eld, what the client's frames are
  --help                 Print this text

The receiver answers the control plane: /info, /fp-setup, /feedback, the pairing
routes and SETUP with its key derivation, and it receives the media a client
sends. `--render null` reads that media, decrypts and orders it, and plays
nothing, which is what a session is checked with; `--render gst` hands it to
GStreamer, and only a build with the `gst` feature has that backend.

Which audio coding a client sends is not something `SETUP` says, so it is given
here: `aac-eld` is what a current client sends, `alac` is what an AirPlay 1 one
does.";

/// Everything the receiver needs before it starts.
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
            // A current client asks for a PIN, and the PIN is printed here so the
            // user can type it.
            pin: PinMode::OneTime,
            password: None,
            device_id: None,
            seed: None,
            log: None,
            render: RenderBackend::Null,
            // AirPlay 2 clients send AAC-ELD, which is what an iPhone or iPad paired
            // with this receiver sends.
            audio_codec: AudioCodec::AacEld,
        }
    }
}

/// The renderer a session plays through, chosen from the options.
///
/// `--render gst` in a build without the feature is refused rather than quietly downgraded:
/// a receiver that plays nothing while its user believes it should be playing is worse than
/// one that says which build it is.
fn renderer(backend: RenderBackend) -> Result<Arc<dyn airplay_renderer::Renderer>, String> {
    match backend {
        RenderBackend::Null => Ok(Arc::new(airplay_renderer::NullRenderer::new())),
        RenderBackend::Gst => open_gst(),
    }
}

/// The GStreamer backend, when this build has it.
#[cfg(feature = "gst")]
fn open_gst() -> Result<Arc<dyn airplay_renderer::Renderer>, String> {
    let renderer = airplay_renderer_gst::GstRenderer::new()
        .map_err(|error| format!("--render gst: {error}"))?;
    println!(
        "render    gst    {} -> audio {}, video {}",
        renderer.program(),
        renderer.audio_sink(),
        renderer.video_sink()
    );
    Ok(Arc::new(renderer))
}

/// The GStreamer backend, which this build does not have.
#[cfg(not(feature = "gst"))]
fn open_gst() -> Result<Arc<dyn airplay_renderer::Renderer>, String> {
    Err("--render gst needs a build with the `gst` feature: cargo run -p airplay-cli --features gst"
        .to_owned())
}

/// Opens an audio stream on the renderer, reporting why when it cannot.
///
/// A session without audio is not a reason to drop the client: the protocol half still
/// works, and what is wrong is the machine's media stack rather than the session.
///
/// The stream is owned rather than borrowed from the renderer, because it is played on a
/// thread of its own: a stream tied to the renderer's lifetime could not be moved into one,
/// and a stream that blocks the receive loop is what made a session with sound in it freeze.
fn open_audio(
    renderer: &Arc<dyn airplay_renderer::Renderer>,
    codec: AudioCodec,
) -> Option<Box<dyn airplay_renderer::AudioStream>> {
    let config = AudioConfig {
        codec,
        ..AudioConfig::default()
    };
    match renderer.open_audio(config) {
        Ok(stream) => {
            println!(
                "media     audio stream open, {} at {}",
                codec.name(),
                config.sample_rate
            );
            Some(stream)
        }
        Err(error) => {
            eprintln!("rust-airplay: cannot play audio: {error}");
            None
        }
    }
}

/// Where to write the first video frames, when a session is being diagnosed.
///
/// `AIRPLAY_DUMP_VIDEO=<path>` names a file the frames are appended to as
/// Annex-B, which is what makes "the picture is incomplete" a question about
/// bytes rather than about a window: a second decoder reading that file says
/// whether the stream is well formed, and where it stops being.
fn dump_video_path() -> Option<std::path::PathBuf> {
    std::env::var_os("AIRPLAY_DUMP_VIDEO").map(std::path::PathBuf::from)
}

/// Where to write the audio bytes a client sends, when that has been asked for.
///
/// The bytes are written with a four-byte little-endian length in front of each payload
/// and nothing else added: what a device sends is a sequence of frames of one coding, and
/// the coding is the question. A file with the lengths in it can be cut back into frames,
/// handed to a decoder this port did not write, and measured — packet sizes are what say
/// whether a stream is constant bit rate, and a decoder that is given raw frames has no
/// sample rate or channel count to work from, so the sizes are the thing that makes either
/// possible.
fn dump_audio_path() -> Option<std::path::PathBuf> {
    std::env::var_os("AIRPLAY_DUMP_AUDIO").map(std::path::PathBuf::from)
}

/// A future that is ready when the receiver is stopping.
///
/// A receiver ends with Ctrl+C, and its receive loops need to be told: a loop that outlived
/// the control plane would hold a sound card or a window open with nothing to play into it.
/// One that cannot install signal handling waits forever instead, which is the same receiver
/// it was before this existed.
type Stopping = tokio::sync::watch::Receiver<bool>;

/// A signal that says the receiver is stopping, and a way to watch it.
fn shutdown_signal() -> Stopping {
    let (sender, receiver) = tokio::sync::watch::channel(false);
    #[cfg(feature = "gst")]
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        println!("\nstopping");
        let _ = sender.send(true);
    });
    #[cfg(not(feature = "gst"))]
    let _ = sender;
    receiver
}

/// Waits for the signal, or forever for a receiver that has none.
async fn stopped(mut signal: Stopping) {
    let _ = signal.changed().await;
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
    // The renderer is chosen before anything is advertised, because a backend this build
    // or this machine cannot provide is something to say now rather than when a client is
    // already waiting for sound.
    let renderer = renderer(options.render)?;
    if options.render == RenderBackend::Null {
        println!("render    null   media is read and played nowhere");
    }
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
    let audio_renderer = Arc::clone(&renderer);
    let audio_codec = options.audio_codec;
    // The control plane runs until Ctrl+C, and a receive loop that outlived it would keep a
    // sound card or a window open with nothing to play into it.
    let stopping = shutdown_signal();
    let audio_stop = stopping.clone();
    let (media_key, media_ready) = tokio::sync::oneshot::channel::<([u8; 16], [u8; 16])>();
    let media_key = Arc::new(Mutex::new(Some(media_key)));
    // The audio key of the session, kept where every control connection can see it.
    //
    // A device sends its audio key in one `SETUP` and its `type: 110` stream entry in a
    // later one without repeating the key — and those two need not be served by the same
    // control connection, because the receiver builds a handler set per connection
    // (`Server::run`'s `new_handlers`). A remembered key that lives inside one of those
    // handler sets is therefore not a key the session has: it is a key that connection
    // saw, and the connection that needs it is the one that never had it. That is why
    // this is shared rather than local.
    let session_audio_key: Arc<Mutex<Option<[u8; 16]>>> = Arc::new(Mutex::new(None));
    // Where the clock exchange goes: the timing port a client named in its `SETUP`,
    // at the address its control connection came from. The receiver starts that
    // exchange, so a session that never learns this never ticks
    // (`raop_ntp.c:614-618`).
    let (timing_target, timing_ready) = tokio::sync::oneshot::channel::<std::net::SocketAddr>();
    let timing_target = Arc::new(Mutex::new(Some(timing_target)));
    // The mirror stream is keyed by the connection identifier a client sends in its
    // `type: 110` stream entry, which arrives in a *different* `SETUP` from the audio
    // key the stream is derived from — and a device may open the mirror connection
    // before either. So the keys live in one place that is written whenever a `SETUP`
    // derives them and read whenever a connection needs them, rather than in a channel
    // that can only hand them over once: a one-shot delivery loses them entirely if
    // the first `SETUP` of a session is the one that consumes it, which is exactly what
    // a device that splits its session across two requests does.
    let (mirror_keys, mirror_ready) =
        tokio::sync::watch::channel::<Option<airplay_core::media::mirror::StreamKeys>>(None);
    let mirror_keys = Arc::new(Mutex::new(mirror_keys));
    let audio_sockets = Arc::clone(&media);
    let audio_stop = audio_stop.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = stopped(audio_stop.clone()) => {}
            ready = media_ready => {
                let Ok((key, iv)) = ready else {
                    return;
                };
                let stream = open_audio(&audio_renderer, audio_codec);
                let mut session = airplay_core::media::ports::session(key, iv);
                // Audio is pushed by a thread of its own, for the same reason video is:
                // writing to a pipeline blocks when the pipeline is full, and a write that
                // blocks inside the receive loop stops everything that loop is watching —
                // the screen, the control socket and the clock exchange. A session with
                // sound in it froze while a silent one ran, and this is that.
                //
                // A queue that is full drops the frame rather than waiting. Waiting is the
                // fault being avoided, and the receive loop has to keep draining the sockets
                // whatever the pipeline is doing; dropping is counted and reported once, so
                // a pipeline that never recovers is seen rather than buffered away.
                let (feed, queued) = std::sync::mpsc::sync_channel::<airplay_renderer::AudioFrame>(
                    QUEUED_AUDIO_FRAMES,
                );
                let played = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let played_writer = std::sync::Arc::clone(&played);
                // The stream is taken out of the `Option` rather than borrowed from it: the
                // writer thread owns it for the life of the session, and a borrowed stream
                // cannot be moved into a thread.
                let playing = stream.map(|playing| {
                    std::thread::spawn(move || {
                        use std::sync::atomic::Ordering;
                        while let Ok(frame) = queued.recv() {
                            // What real-time playback needs and this backend does not do
                            // yet: the frame carries its timestamp and the sink is told to
                            // play as fast as it is given, so audio runs at whatever rate
                            // the pipeline manages. Named here rather than left implicit.
                            if let Err(error) = playing.push(frame) {
                                eprintln!("rust-airplay: the renderer refused a frame: {error}");
                                break;
                            }
                            played_writer.fetch_add(1, Ordering::Relaxed);
                        }
                        // A last frame already handed over is played out rather than cut
                        // off, which is what an end of stream means to a backend.
                        if let Err(error) = playing.close() {
                            eprintln!("rust-airplay: the audio stream ended badly: {error}");
                        }
                    })
                });
                // The clock exchange runs beside the audio, on its own socket: the
                // receiver is the one that starts it, and upstream keeps it going
                // for as long as the session does.
                let timing_sockets = Arc::clone(&audio_sockets);
                let timing_stop = audio_stop.clone();
                tokio::spawn(async move {
                    let Ok(target) = timing_ready.await else {
                        return;
                    };
                    println!("media     clock exchange with {target}");
                    let mut sent: u64 = 0;
                    let mut stop = timing_stop;
                    loop {
                        // `build_request` takes Unix nanoseconds and puts the NTP form
                        // of them where upstream puts the send time
                        // (`raop_ntp.c:615-616`).
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|since| since.as_nanos() as u64)
                            .unwrap_or(0);
                        let Ok(request) = airplay_core::media::timing::build_request(now) else {
                            eprintln!("rust-airplay: cannot build a timing request");
                            return;
                        };
                        match timing_sockets.send_timing_request(target, &request).await {
                            Ok(_) => sent += 1,
                            Err(error) => {
                                eprintln!("rust-airplay: cannot send a timing request: {error}");
                                return;
                            }
                        }
                        tokio::select! {
                            _ = stop.changed() => return,
                            _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {}
                        }
                        if sent % 10 == 0 {
                            println!("media     {sent} timing requests sent");
                        }
                    }
                });
                let frames = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let dropped = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                // What a device actually sends, for a decoder that is not this one to
                // identify: a stream that is received, queued and played without a sound is
                // one whose coding is the thing in doubt.
                let dump_frames = dump_audio_path().map(|path| {
                    println!("media     writing the audio bytes to {}", path.display());
                    let file = std::fs::File::create(&path)
                        .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
                    std::sync::Arc::new(std::sync::Mutex::new(file))
                });
                // What arrived, so "the client is not sending" and "we are throwing it
                // away" cannot be confused in a log. A session that never reaches the
                // frame counter still says which sockets were written to.
                let received = std::sync::Arc::new(airplay_core::media::ports::Received::default());
                let watching = std::sync::Arc::clone(&received);
                tokio::spawn(async move {
                    let mut last = (0u64, 0u64, 0u64, 0u64);
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        let now = (
                            watching.data(),
                            watching.control(),
                            watching.timing(),
                            watching.unusable(),
                        );
                        if now != last {
                            println!(
                                "media     received: data {}, control {}, timing {}, unusable {}",
                                now.0, now.1, now.2, now.3
                            );
                            if let Some(reason) = watching.last_error() {
                                println!("media     first datagram that did not fit: {reason}");
                            }
                            last = now;
                        }
                    }
                });
                let outcome = audio_sockets
                    .serve(&mut session, &received, |frame| {
                        use std::sync::atomic::Ordering;
                        let count = frames.fetch_add(1, Ordering::Relaxed) + 1;
                        if count == 1 {
                            println!(
                                "media     first audio frame: {} bytes, seq {}, ts {}",
                                frame.payload.len(),
                                frame.sequence,
                                frame.timestamp
                            );
                        } else if count % 500 == 0 {
                            println!("media     {count} audio frames received");
                        }
                        if let Some(file) = dump_frames.as_ref().and_then(|dump| dump.lock().ok()) {
                            use std::io::Write;
                            let length = frame.payload.len() as u32;
                            let _ = (&*file).write_all(&length.to_le_bytes());
                            let _ = (&*file).write_all(&frame.payload);
                        }
                        let queued = airplay_renderer::AudioFrame {
                            data: frame.payload,
                            timestamp: u64::from(frame.timestamp),
                        };
                        if feed.try_send(queued).is_err() {
                            let missed = dropped.fetch_add(1, Ordering::Relaxed) + 1;
                            if missed == 1 {
                                eprintln!(
                                    "rust-airplay: the audio pipeline is not keeping up; \
                                     frames are being dropped rather than stalling the session"
                                );
                            }
                        }
                    })
                    .await;
                drop(feed);
                if let Some(playing) = playing {
                    let _ = playing.join();
                }
                println!(
                    "media     {} audio frames played of {}, {} dropped",
                    played.load(std::sync::atomic::Ordering::Relaxed),
                    frames.load(std::sync::atomic::Ordering::Relaxed),
                    dropped.load(std::sync::atomic::Ordering::Relaxed)
                );
                if let Err(error) = outcome {
                    eprintln!("rust-airplay: the media session stopped: {error}");
                }
            }
        }
    });

    // A mirroring client connects over TCP after its `SETUP`, and until it does
    // there is nothing to read from. Each connection gets a session of its own: a
    // client that reconnects starts its stream over.
    let mirror_sockets = Arc::clone(&media);
    let mirror_renderer = Arc::clone(&renderer);
    let mirror_stop = stopping.clone();
    tokio::spawn(async move {
        // The listener accepts from the moment the receiver is up, which is not the
        // moment it has keys: a device may open the mirror connection before the
        // `SETUP` that carries the `type: 110` stream entry has been answered, and a
        // connection nobody accepts is one the device abandons after a timeout. So
        // the first connection waits for the keys beside the accept rather than
        // behind it.
        println!(
            "media     mirror listener ready on port {}",
            mirror_sockets.mirror_port().unwrap_or_default()
        );
        // The keys a connection needs are whatever the last `SETUP` derived, so each
        // connection reads them here rather than being handed them once. A connection
        // that arrives first waits for them beside the accept, and a later `SETUP` that
        // re-keys the stream is picked up without the listener having to restart.
        let mut keys = mirror_ready;
        let mut pending = None;
        loop {
            let mut stream = match pending.take() {
                Some(stream) => stream,
                None => tokio::select! {
                    _ = stopped(mirror_stop.clone()) => return,
                    accepted = mirror_sockets.accept_mirror() => match accepted {
                        Ok(stream) => stream,
                        Err(error) => {
                            eprintln!("rust-airplay: the mirror listener stopped: {error}");
                            return;
                        }
                    },
                },
            };
            // A mirror stream cannot be read without its keys, so a connection that got
            // here first waits for the `SETUP` that derives them. The wait is beside the
            // accept rather than in front of it, so a second connection is still taken.
            let current = loop {
                if let Some(current) = *keys.borrow() {
                    break current;
                }
                tokio::select! {
                    _ = stopped(mirror_stop.clone()) => return,
                    changed = keys.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                    accepted = mirror_sockets.accept_mirror(), if pending.is_none() => match accepted {
                        Ok(stream) => {
                            println!("media     another mirror connection is waiting for its keys");
                            pending = Some(stream);
                        }
                        Err(error) => {
                            eprintln!("rust-airplay: the mirror listener stopped: {error}");
                            return;
                        }
                    },
                }
            };
            println!(
                "*** mirror stream keyed, reading the connection ({} waiting)",
                usize::from(pending.is_some())
            );
            let keys = current;
            let mut session = match airplay_core::media::mirror::MirrorSession::new(&keys) {
                Ok(session) => session,
                Err(error) => {
                    eprintln!("rust-airplay: cannot start the mirror session: {error}");
                    return;
                }
            };
            // A mirror stream carries its own codec data, so what it needs from the
            // renderer is a video stream and nothing else; the size is whatever the client
            // sends, which is not something `SETUP` says.
            let video = match mirror_renderer.open_video(airplay_renderer::VideoConfig {
                width: None,
                height: None,
            }) {
                Ok(stream) => Some(stream),
                Err(error) => {
                    eprintln!("rust-airplay: cannot show video: {error}");
                    None
                }
            };
            let mut frames: u64 = 0;
            let mut sent: u64 = 0;
            let mut refused = 0u64;
            // When the frame being pushed belongs, from the client's own timestamps.
            // A mirror stream is one key frame followed by frames that describe only
            // what changed, so decoding faster than real time loses frames the rest of
            // the stream depends on and leaves the picture partly wrong for the rest of
            // the session. The rate is the client's to state, and it is applied in the
            // thread below rather than here: writing to a pipeline and waiting for the
            // next frame are both blocking, and a receive loop is not a place a blocking
            // call can be — one that waits stops reading its socket, which stops the
            // client too, and the picture freezes with it.
            // How many frames are waiting for the renderer, kept beside the queue
            // because a channel does not say: it is the number that decides whether a
            // picture is being kept up with or dropped.
            let waiting = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let waiting_writer = std::sync::Arc::clone(&waiting);
            let (feed, queued) =
                std::sync::mpsc::sync_channel::<(Vec<u8>, u64, bool)>(QUEUED_FRAMES);
            let playing = video.map(|video| {
                std::thread::spawn(move || {
                    let mut shown: u64 = 0;
                    while let Ok((data, timestamp, idr)) = queued.recv() {
                        waiting_writer.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        // What the pacing is actually told, for the first frames: the
                        // interval a device sends is not a frame rate this port can
                        // assume, and a wrong one is a wait that stalls a picture
                        // instead of pacing it. Printed rather than silently clamped,
                        // because the number is the whole question.
                        if shown < 3 {
                            println!("media     writer frame {}: {} bytes", shown + 1, data.len());
                        }
                        // No waiting between frames, and the interval that used to
                        // drive one is only printed now. Waiting was this port's own
                        // idea and it was wrong twice over: the pipeline it was
                        // protecting measures fifty-one frames a second on the machine
                        // it runs on, so it never needed protecting, and a wait of one
                        // frame interval sets the rate to exactly what the client sends
                        // — so the writer plus its own wait could only ever be slower
                        // than the stream. The queue filled, frames were dropped, and
                        // every later frame referred to a picture that was never
                        // finished.
                        //
                        // What paces a stream is what reads it: writing to a pipeline
                        // blocks when the pipeline is full, which is backpressure doing
                        // the job without a guess about the client's clock.
                        if let Err(error) = video.push(airplay_renderer::VideoFrame {
                            data,
                            timestamp,
                            keyframe: idr,
                        }) {
                            eprintln!("rust-airplay: the renderer refused a frame: {error}");
                            break;
                        }
                        shown += 1;
                    }
                    // A last frame already handed over is played out rather than cut
                    // off, which is what an end of stream means to a backend.
                    match video.close() {
                        Ok(()) => shown,
                        Err(error) => {
                            eprintln!("rust-airplay: the video stream ended badly: {error}");
                            shown
                        }
                    }
                })
            });
            // Every packet a mirror connection sends, counted before it is
            // interpreted, so that a stream which stops is a stream that stopped
            // arriving rather than one this receiver cannot say anything about.
            let received = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            let received_bytes = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            let watching = std::sync::Arc::clone(&received);
            let watching_bytes = std::sync::Arc::clone(&received_bytes);
            let watching_frames = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            let watching_frames_task = std::sync::Arc::clone(&watching_frames);
            // The census the session keeps is the one the reporter reads: it is shared
            // rather than copied, so there is one answer to what became of the packets.
            let watching_census = session.census();
            // The recent packets live beside the census and are read the same way.
            let recent_source = std::sync::Arc::clone(&watching_census);
            let watching_recent = move || recent_source.recent();
            tokio::spawn(async move {
                let mut last = (0u64, 0u64, 0u64);
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    let now = (
                        watching.load(std::sync::atomic::Ordering::Relaxed),
                        watching_bytes.load(std::sync::atomic::Ordering::Relaxed),
                        watching_frames_task.load(std::sync::atomic::Ordering::Relaxed),
                    );
                    if now != last {
                        println!(
                            "media     mirror: {} packets, {} bytes in, {} frames out",
                            now.0, now.1, now.2
                        );
                        // What became of the packets that were not frames. A stream
                        // that is arriving but producing nothing says which of the
                        // kinds it turned into, which is the whole question when a
                        // picture stops.
                        let census = watching_census.read();
                        println!(
                            "media     mirror kinds: video {}, codec {}, reports {}, \
                             unknown {} {:?}, not units {}",
                            census.video,
                            census.codec,
                            census.reports,
                            census.unknown,
                            census.last_unknown,
                            census.not_units
                        );
                        // The first payload a reader could not take, with the bytes it
                        // was given: a payload that is not units is either a decryption
                        // gone wrong or a framing this receiver does not have, and the
                        // bytes are what says which.
                        if let Some(bad) = &census.first_bad {
                            println!(
                                "media     first unreadable payload: {} bytes declared, {} read: \
                                 {} (bytes: {})",
                                bad.declared, bad.length, bad.error, bad.preview
                            );
                            // The packets either side of it. A keystream that has gone
                            // out of step is only visible across packets, and the one
                            // that broke it is the one just before the first payload
                            // that could not be read.
                            println!(
                                "media     packets around it (size, carry->carry, blocks->blocks, first bytes, read):"
                            );
                            for packet in watching_recent() {
                                println!(
                                    "media       {:>7} B  carry {}->{}  blocks {}->{}  \
                                     {:02x}{:02x}{:02x}{:02x}  {}",
                                    packet.declared,
                                    packet.carried_before,
                                    packet.carried_after,
                                    packet.blocks_before,
                                    packet.blocks_after,
                                    packet.first[0],
                                    packet.first[1],
                                    packet.first[2],
                                    packet.first[3],
                                    if packet.read { "frame" } else { "NOT READ" }
                                );
                            }
                        }
                        last = now;
                    }
                }
            });
            let outcome = stream
                .serve(
                    &mut session,
                    |_header, bytes| {
                        received.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        received_bytes
                            .fetch_add(bytes as u64, std::sync::atomic::Ordering::Relaxed);
                        // Nothing to do here: the census is shared, so the reporter
                        // reads it as it stands. A copy per packet would be a copy of
                        // something the reporter can already see.
                    },
                    |frame| {
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
                    // The bytes handed to a decoder are what a picture is diagnosed
                    // from, and nothing else can say whether a stream is malformed or
                    // merely hard to follow: `AIRPLAY_DUMP_VIDEO=<path>` writes the
                    // first frames as an Annex-B stream a second decoder can read.
                    if let Some(path) = dump_video_path() {
                        if frames <= 30 {
                            use std::io::Write;
                            if let Ok(mut file) = std::fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&path)
                            {
                                let _ = file.write_all(&frame.data);
                            }
                        }
                    }
                    // A queue that is full means the picture is behind what a client
                    // sent, which is worth saying once: a frame dropped here is one the
                    // rest of the stream may depend on.
                    match feed.try_send((frame.data, frame.timestamp, frame.idr)) {
                        Ok(()) => {
                            sent += 1;
                            waiting.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if sent % 200 == 0 {
                                // The depth is the number that says whether the
                                // renderer keeps up: a queue that hovers near empty is
                                // a pipeline reading at the rate a client sends, and
                                // one that sits at the limit is a picture about to be
                                // dropped.
                                println!(
                                    "media     {sent} video frames queued, {} waiting",
                                    waiting.load(std::sync::atomic::Ordering::Relaxed)
                                );
                            }
                        }
                        Err(std::sync::mpsc::TrySendError::Full(_)) => {
                            refused += 1;
                            if refused == 1 {
                                eprintln!(
                                    "rust-airplay: the renderer is {QUEUED_FRAMES} frames behind; \
                                     frames are being dropped and the picture will be incomplete"
                                );
                            }
                        }
                        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {}
                    }
                })
                .await;
            // Dropping the queue is what ends the writer, and waiting for it is what
            // makes the count below the frames that were actually handed over.
            drop(feed);
            if let Some(playing) = playing {
                match playing.join() {
                    Ok(shown) => println!("media     {shown} video frames shown of {frames}"),
                    Err(_) => eprintln!("rust-airplay: the video thread ended unexpectedly"),
                }
            }
            match outcome {
                // The client closing the connection is how a mirror session ends.
                Ok(()) => println!("media     the mirror stream ended after {frames} frames"),
                // A connection that comes apart ends that connection, not the session:
                // a client reconnects after a mirror stream goes wrong, and a receiver
                // that dropped its listener because of one bad packet would never see
                // it again. The loop goes back to accepting.
                Err(error) => eprintln!(
                    "rust-airplay: the mirror stream stopped after {frames} frames: {error}\n\
                     rust-airplay: waiting for the client to connect again"
                ),
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
                let timing_target = Arc::clone(&timing_target);
                // The audio key has to outlive the request that carried it *and* the
                // connection that served it, so it is the session's rather than this
                // handler set's.
                let session_audio_key = Arc::clone(&session_audio_key);
                handlers.set_setup_callback(move |request| {
                    // Which streams a client asked for is what decides whether the
                    // mirror listener will ever see a connection, so it is part of
                    // what a session log has to say.
                    let asked: Vec<String> = request
                        .streams
                        .iter()
                        .map(|stream| {
                            // The coding and the frame size, when the client named them:
                            // `ct` is the one field that says which decoder a stream needs,
                            // and a receiver that does not print it is a receiver that
                            // cannot be told its assumption is wrong.
                            let coding = match stream.compression_type {
                                Some(2) => "ct 2 (ALAC)".to_owned(),
                                Some(8) => "ct 8 (AAC-ELD)".to_owned(),
                                Some(other) => format!("ct {other}"),
                                None => "ct absent".to_owned(),
                            };
                            let spf = match stream.samples_per_frame {
                                Some(spf) => format!(", spf {spf}"),
                                None => String::new(),
                            };
                            match stream.stream_connection_id {
                                Some(id) => {
                                    format!("type {} (id {id}, {coding}{spf})", stream.kind)
                                }
                                None => format!("type {} ({coding}{spf})", stream.kind),
                            }
                        })
                        .collect();
                    println!("media     SETUP keys [{}]", request.keys.join(", "));
                    println!(
                        "media     SETUP asked for [{}], audio key {}, iv {}",
                        asked.join(", "),
                        if request.audio_key.is_some() {
                            "yes"
                        } else {
                            "no"
                        },
                        if request.audio_iv.is_some() {
                            "yes"
                        } else {
                            "no"
                        }
                    );
                    // The clock exchange needs the address the client's control
                    // connection came from, at the port its `SETUP` named. A client
                    // that names no port is the AirPlay 2 "Remote Control" protocol
                    // upstream does not serve (`raop_handlers.h:902-907`), and one
                    // whose request had no transport at all cannot be reached.
                    match (request.client_address, request.client_timing_port) {
                        (Some(address), Some(port)) => {
                            if let Some(sender) = lock(&timing_target).take() {
                                let target = std::net::SocketAddr::new(address.ip(), port);
                                let _ = sender.send(target);
                            }
                        }
                        (address, port) => println!(
                            "media     no clock exchange: address {}, timingPort {}",
                            address.map_or("unknown".to_owned(), |a| a.ip().to_string()),
                            port.map_or("absent".to_owned(), |p| p.to_string())
                        ),
                    }
                    // The session key starts the receive loop, once, however many
                    // connections a client opens. It is also kept, because a device
                    // sends its `type: 110` stream entry in a later request without
                    // repeating the key that stream is encrypted under.
                    if let (Some(key), Some(iv)) = (request.audio_key, request.audio_iv) {
                        *lock(&session_audio_key) = Some(key);
                        if let Some(sender) = lock(&media_key).take() {
                            let _ = sender.send((key, iv));
                            println!("*** media session keyed, audio can arrive ***");
                        }
                    }
                    // The mirror stream's key is the audio key hashed with the
                    // connection identifier the client chose, and a device sends
                    // those in *different* requests: the key in the first `SETUP`
                    // of a session, the `type: 110` stream entry in a later one
                    // without the key repeated (`raop_handlers.h:955-967` derives
                    // it from the audio key the connection already holds). This
                    // used to be read from the same request, which meant a device
                    // that splits them never got a mirror stream at all: the
                    // listener below waits for keys that were never derived, so a
                    // client that connects is not even accepted.
                    let mirror_stream = request
                        .streams
                        .iter()
                        .find(|stream| stream.kind == MIRROR_STREAM_TYPE);
                    match mirror_stream.and_then(|stream| stream.stream_connection_id) {
                        Some(id) => {
                            let key = request.audio_key.or(*lock(&session_audio_key));
                            let Some(key) = key else {
                                eprintln!(
                                    "rust-airplay: a mirror stream was asked for before any \
                                     audio key arrived, so its bytes cannot be decrypted"
                                );
                                return;
                            };
                            // The keys are published where the listener reads them, so a
                            // connection that is already waiting is released and one that
                            // arrives later finds them.
                            let keys = airplay_core::media::mirror::stream_keys(id, &key);
                            println!("*** mirror stream keyed (id {id}), video can arrive ***");
                            lock(&mirror_keys).send_replace(Some(keys));
                        }
                        // A `type: 110` entry with no identifier is a session this
                        // receiver cannot key, and the listener would otherwise wait for
                        // keys nothing will ever derive. Saying so is the difference
                        // between a stream that never arrived and one that arrived and
                        // could not be read — the two looked identical in the last two
                        // sessions, which is what made them slow to tell apart.
                        None if mirror_stream.is_some() => eprintln!(
                            "rust-airplay: the client asked for a mirror stream without \
                             naming its connection id, so its keys cannot be derived"
                        ),
                        None => {}
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
