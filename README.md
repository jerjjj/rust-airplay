# rust-airplay

rust-airplay turns an ordinary computer into an AirPlay receiver. An iPhone, iPad or Mac
finds it in the AirPlay menu, mirrors its screen or streams audio to it, and the receiver
decrypts the stream and hands the frames to a renderer. Audio and screen mirroring both
work; the receiver serves the AirPlay control protocol over TCP, the media over UDP and a
second TCP connection, and announces itself with mDNS.

The repository has three crates. `rust-airplay` is the receiver as a library, `rust-airplay`
also ships a command-line program in `rust-airplay-cli`, and `rust-airplay-gstreamer`
implements rendering with GStreamer. The library has no C dependencies, and a program that
only wants to inspect the protocol can run it with a renderer that plays nothing.

## Features

- Screen mirroring and audio from iOS and macOS clients.
- Pairing with a PIN, remembered across restarts.
- Channel layout: AirPlay advertises itself as `_raop._tcp` and `_airplay._tcp` over mDNS,
  then serves `/info`, `/pair-verify`, `/pair-setup`, `/fp-setup`, `SETUP`, `RECORD`,
  `SET_PARAMETER`, `/feedback` and `TEARDOWN`.
- Audio reordering with retransmission requests when a packet is lost.
- A small renderer trait, plus a GStreamer implementation and a null one.

## Requirements

Rust 1.85 or newer.

To play anything, GStreamer is needed as well. The library and the command-line receiver
build and run without it when you pass `--render null`; only the `gst` feature pulls it in.

| What | Needs |
| --- | --- |
| Library | Rust 1.85 |
| Command-line receiver, `--render null` | Rust 1.85 |
| Command-line receiver, `--render gst` | GStreamer development files to build, GStreamer runtime and plugins to run |

`docs/BUILD.md` covers the packages per platform.

## Getting started

Build the receiver with the GStreamer backend:

```sh
cargo build -p rust-airplay-cli --features gst
```

Start it. A name is what clients will see in their menu; drop `--port` to use 7000.

```sh
cargo run -p rust-airplay-cli --features gst -- --name "Living room" --port 7000 --render gst
```

The program prints the ports it bound, the identity it derived, and a PIN. On an iPhone or
iPad, open the AirPlay menu, pick the receiver, and type the PIN. Screen mirroring starts as
soon as the client connects; audio follows whenever the client plays something.

To check a session without a sound card or a display, run the same receiver with
`--render null`. It decrypts and counts everything it receives and plays none of it, which
is enough to see whether a client connects, whether keys derive, and whether frames arrive.

```sh
cargo run -p rust-airplay-cli -- --port 7000 --render null
```

`--help` lists every option.

## Using the library

```toml
[dependencies]
rust-airplay = "0.1"
```

```rust,no_run
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use rust_airplay::server::PinMode;
use rust_airplay::{AudioCodec, Config, NullRenderer};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = Config {
    name: "Living room".to_owned(),
    port: 7000,
    seed: [0x42; 32],
    device_id: None,
    pin: PinMode::OneTime,
    password: None,
    audio_codec: AudioCodec::AacEld,
    renderer: Arc::new(NullRenderer::new()),
    clients: Arc::new(Mutex::new(HashSet::new())),
    on_client: None,
    trace_log: None,
    dump_video: None,
    dump_audio: None,
};

rust_airplay::run(config).await?;
# Ok(())
# }
```

`run` serves clients until the process is asked to stop, and returns an error only when the
receiver cannot start at all. Progress and problems are reported through the `log` crate,
so the program embedding it decides where messages go. Storage is the caller's business
too: `seed` is what makes a receiver recognisable across restarts and `clients` is what
makes a paired client skip the PIN, and both are values the caller passes in and persists.

To play the media, implement `Renderer` or use the GStreamer backend:

```toml
[dependencies]
rust-airplay = "0.1"
rust-airplay-gstreamer = "0.1"
```

## Usage notes

A few things are worth knowing before the first run.

**Ports.** The control service uses one port, 7000 by default. Windows sometimes reserves
ranges of ports for Hyper-V or WSL, and binding fails with `os error 10013` when it does.
`netsh int ipv4 show excludedportrange protocol=tcp` lists the reserved ranges. Passing
`--port 0` picks an unused one, but the advertised record then carries an ephemeral port
that changes on every start.

**Files.** The command-line receiver keeps two files under the local application data
directory, `rust-airplay/identity.seed` and `rust-airplay/clients`. Deleting the first gives
the receiver a new identity, so every client has to pair again. Deleting the second keeps
the identity and forgets the clients. The library itself writes neither.

**GStreamer.** The backend starts video as a `gst-launch-1.0` subprocess and drives audio
in-process. If the receiver reports that a stream cannot be opened, the message names the
GStreamer element that is missing. `AIRPLAY_GST_VIDEO_SINK` chooses a different video sink;
on Windows `d3d11videosink` starts faster than the default `autovideosink`.
`AIRPLAY_GST_VIDEO_WARM=1` creates the video pipeline at startup so the first client does
not wait for the window.

**Audio coding.** `SETUP` does not say which coding an audio stream carries, so the receiver
is told with `--audio-codec`. `aac-eld` is what current iOS clients send, `alac` is what an
AirPlay 1 client sends.

## Documentation

- [API reference](https://docs.rs/rust-airplay) generated from the source
- [docs/API.md](docs/API.md) describes the configuration, the renderer trait and the command line
- [docs/BUILD.md](docs/BUILD.md) covers dependencies, GStreamer per platform, and troubleshooting
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) describes the crates and the path a session takes
- [AGENT.md](AGENT.md) is for automated contributors

Chinese translations sit beside each document with a `.zh-CN.md` suffix, starting at
[README.zh-CN.md](README.zh-CN.md).

## Licence

GPL-3.0-or-later. The full text is in [LICENSE](LICENSE), and
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) records the notices each translated file
carries.
