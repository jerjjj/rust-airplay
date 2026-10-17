# rust-airplay

An AirPlay 1/2 receiver for Windows, macOS and Linux, written in Rust.

A receiver accepts a screen-mirroring session and an audio stream from an iPhone, iPad or
Mac, decrypts them, and plays them through a renderer of your choosing. This crate is the
receiver: the protocol, the media session, and the boundary a renderer plugs into. It is a
port of [UxPlay](https://github.com/FDH2/UxPlay) with the same wire behaviour.

## Overview

- **Answers AirPlay.** `_raop._tcp` and `_airplay._tcp` are advertised over mDNS, and the
  control protocol is served over TCP: `/info`, `/fp-setup`, `/pair-setup`, `/pair-verify`,
  `/feedback`, `SETUP`, `RECORD`, `SET_PARAMETER`, `TEARDOWN`.
- **Pairs clients.** A one-time or fixed PIN through SRP-6a, or a key a client already has
  through Curve25519. Verified keys are remembered, so a client that has paired once is
  recognised without a PIN.
- **Handles the media.** Audio is decrypted, reordered against a bounded window and asked
  for again when a packet is missing. Mirroring is decrypted against a keystream whose
  position follows a rule read off UxPlay's own code, and reassembled into frames.
- **Leaves rendering to you.** Frames cross a small trait boundary. A GStreamer backend is
  provided; `NullRenderer` accepts everything and plays nothing, which is how a session is
  checked on a machine with no sound card.
- **Keeps policy out.** The library reads no environment variable, chooses no path and
  prints nothing. Identity seeds, client lists and loggers are provided by the program that
  runs it.

## Status

| What | Verified on |
|---|---|
| Screen mirroring and audio playback | Windows 11, a real iPad, GStreamer 1.28 |
| Control plane, pairing, discovery | Windows 11, a real iPad |
| Everything else | the test suite, and a differential test against UxPlay's C code |

macOS and Linux build and pass their tests, but no device has been run against either. The
GStreamer elements a session needs differ per platform, so treat those two as unverified.

Not implemented yet: synchronising audio and video to a common clock. Both playback paths
run at the rate they receive frames.

## Requirements

Rust 1.85 or newer. The default build links no C library.

Playing media needs GStreamer; it is an optional feature, and the workspace builds, tests
and lints without it. See [docs/BUILD.md](docs/BUILD.md) for per-platform instructions.

## Getting started

Build the library and the command-line receiver:

```sh
cargo build --workspace
```

Build the GStreamer backend, which needs the GStreamer development files:

```sh
cargo build -p rust-airplay-cli --features gst
```

Run a receiver:

```sh
cargo run -p rust-airplay-cli --features gst -- --name "my receiver" --port 7000 --render gst
```

Open the AirPlay menu on an iPhone or iPad, choose the receiver, and enter the PIN the
program prints. Pass `--render null` instead of `--render gst` to run the same receiver
without playing anything.

## Using it as a library

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

`run` serves clients until the process is asked to stop. To play the media for real,
implement `Renderer`, or use [`rust-airplay-gstreamer`](crates/rust-airplay-gstreamer).

## Documentation

- [API reference](https://docs.rs/rust-airplay) on docs.rs
- [docs/API.md](docs/API.md) — the configuration, the renderer traits, the command line
- [docs/BUILD.md](docs/BUILD.md) — dependencies, per-platform notes, troubleshooting
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — how the pieces fit, and why
- [AGENT.md](AGENT.md) — guidance for automated contributors

Chinese translations of every document except `AGENT.md` sit beside them as
`<name>.zh-CN.md`, starting with [README.zh-CN.md](README.zh-CN.md).

## Development

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

The GStreamer backend is a crate of its own, so it has its own copy of those gates; see
[AGENT.md](AGENT.md).

Two suites run against real machinery and are not part of the default run:

```sh
cargo test -p rust-airplay --test upstream_oracle -- --ignored
cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --ignored
```

The first compares this implementation with UxPlay's own C code and needs a local UxPlay
checkout and a C toolchain. The second decodes frames captured from a real session.

## Minimum supported Rust version

1.85. The workspace pins it in `Cargo.toml`, and `cargo check` reports it rather than
failing in a confusing way on an older toolchain.

## Licence

GPL-3.0-or-later. Parts of this crate are translations of UxPlay's GPL and LGPL sources, so
the licence is inherited rather than chosen: a program that links this crate is bound by
it. Per-file attribution is in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Contributing

Bug reports and pull requests are welcome. [AGENT.md](AGENT.md) describes the gates, the
rules the code follows, and what has and has not been verified — please read it before
claiming a change works on a platform this project has never run on.
