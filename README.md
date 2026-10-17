# rust-airplay

An AirPlay 1/2 receiver for Windows, macOS and Linux, written in safe Rust.

`rust-airplay` accepts a screen-mirroring session and an audio stream from an iPhone, iPad
or Mac, decrypts them and plays them through a renderer you choose. It is a port of
[UxPlay](https://github.com/FDH2/UxPlay) that keeps the same wire behaviour and adds a
library split UxPlay does not have: the protocol and the session are a library, and the
command line is a thin program on top of it.

Chinese: [README.zh-CN.md](README.zh-CN.md).

## What it does

- Announces itself over mDNS as an AirPlay receiver, with the records a current client
  expects, so it appears in the AirPlay menu without extra configuration.
- Answers the control protocol: `/info`, `/fp-setup`, `/pair-setup`, `/pair-verify`,
  `/feedback`, `SETUP`, `RECORD`, `SET_PARAMETER` and `TEARDOWN`.
- Pairs clients with a one-time or fixed PIN, and remembers the keys of clients it has
  verified.
- Sets up the audio, control, timing and mirror streams, derives the keys a session needs,
  and decrypts what arrives.
- Reassembles H.264 mirror packets into frames, prepending the stream's parameter sets
  where a client sends them separately.
- Reorders audio, asks for retransmissions when a packet is missing, and answers the clock
  exchange.
- Hands frames to a `Renderer`. A GStreamer backend is included; a `NullRenderer` reads
  everything and plays nothing, which is how a session is checked without hardware.

## What it does not do

- It does not encode. A receiver decrypts and plays; the client encodes.
- It does not record the screen. AirPlay mirroring is not a screen-capture API.
- It does not store anything on its own. An identity seed and a client list are values the
  caller persists, which keeps the library free of file access.
- It does not synchronise audio and video to a common clock yet. Both playback paths run at
  the rate they receive frames; A/V sync is the next piece of work.

## Verified on

| Area | Verified on |
|---|---|
| Screen mirroring and audio playback | Windows 11, a real iPad, MSYS2 UCRT64 GStreamer 1.28 |
| Control plane, pairing, discovery | Windows 11, a real iPad |
| Everything else | the test suite, plus a differential test against UxPlay's own C code |

macOS and Linux compile and pass their tests. No device has been run against either, and
the GStreamer element names and sinks differ per platform, so treat them as unverified.

## Requirements

Building anything in this workspace needs Rust 1.85 or newer. The default build links no C
library, so nothing else is required.

Playing media needs GStreamer, which is an optional feature:

| What | Needs |
|---|---|
| `rust-airplay` (library, default) | Rust 1.85 |
| `rust-airplay-cli --render null` | Rust 1.85 |
| `rust-airplay-cli --render gst` | GStreamer development files to build, GStreamer runtime and plugins to run |
| `rust-airplay-gstreamer` | the same as the line above |

Details, per platform, in [docs/BUILD.md](docs/BUILD.md).

## Build

```sh
cargo build --workspace
cargo build -p rust-airplay-cli --features gst
```

## Run

```sh
cargo run -p rust-airplay-cli -- --name "my receiver" --port 7000 --render gst
```

Then open the AirPlay menu on an iPhone or iPad, choose the receiver, and enter the PIN the
program prints. `--render null` runs the same receiver without playing anything, which is
the way to check the protocol on a machine with no sound card.

All options are in the usage text (`--help`) and in the command line section of
[docs/API.md](docs/API.md).

## Use as a library

```toml
[dependencies]
rust-airplay = "0.1"
```

```rust,no_run
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use rust_airplay::{AudioCodec, Config, NullRenderer};
use rust_airplay::server::PinMode;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = Config {
    name: "my receiver".to_owned(),
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

The full surface is in [docs/API.md](docs/API.md), and the generated documentation is on
[docs.rs](https://docs.rs/rust-airplay).

## Documentation

- [docs/BUILD.md](docs/BUILD.md) - dependencies, per-platform notes, troubleshooting
- [docs/API.md](docs/API.md) - the library API and the command line
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) - how the pieces fit, and why
- [AGENT.md](AGENT.md) - guidance for automated contributors

Every document except `AGENT.md` also exists in Chinese.

## Tests

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Two suites go further and are not part of the default run:

- `cargo test -p rust-airplay --test upstream_oracle -- --ignored` compares this
  implementation with UxPlay's own C code. It needs a local UxPlay checkout and a C
  toolchain; see [docs/BUILD.md](docs/BUILD.md).
- `cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --ignored` drives
  a real GStreamer runtime, including decoding frames captured from a real session.

## Licence

GPL-3.0-or-later. Parts of this crate are translations of UxPlay's GPL and LGPL sources, so
the licence is inherited rather than chosen. Attribution for each translated file is in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
