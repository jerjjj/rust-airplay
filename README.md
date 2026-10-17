# rust-airplay

An AirPlay 1/2 receiver for Windows, macOS and Linux, written in Rust.

It accepts a screen-mirroring session and an audio stream from an iPhone, iPad or Mac, then
decrypts and plays them through a renderer. The repository contains the receiver as a
library, a command-line program that runs it, and a GStreamer renderer.

## Features

- Announces itself over mDNS and answers the AirPlay control protocol, so it appears in the
  AirPlay menu of an iPhone, iPad or Mac.
- Pairs clients with a PIN and remembers them, so a paired client reconnects without one.
- Receives screen mirroring and audio, decrypts them, and hands frames to a renderer.
- Plays through GStreamer, or through nothing at all when you only want to inspect a
  session.

## Requirements

Rust 1.85 or newer. Playing media needs GStreamer:

| Build | Needs |
|---|---|
| Library, and `--render null` | Rust 1.85 |
| `--render gst` | GStreamer development files to build, GStreamer runtime to run |

Per-platform instructions are in [docs/BUILD.md](docs/BUILD.md).

## Build and run

```sh
cargo build -p rust-airplay-cli --features gst
cargo run -p rust-airplay-cli --features gst -- --name "my receiver" --port 7000 --render gst
```

Open the AirPlay menu on an iPhone or iPad, choose the receiver, and enter the PIN the
program prints.

Use `--render null` to run the same receiver without playing anything:

```sh
cargo run -p rust-airplay-cli -- --port 7000 --render null
```

All options are listed by `--help`.

## Use as a library

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

To play the media, implement `Renderer` or use
[`rust-airplay-gstreamer`](crates/rust-airplay-gstreamer).

## Documentation

- [API reference](https://docs.rs/rust-airplay)
- [docs/API.md](docs/API.md) - configuration, renderer traits, command line
- [docs/BUILD.md](docs/BUILD.md) - dependencies and troubleshooting
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) - crate layout and the path a session takes
- [AGENT.md](AGENT.md) - notes for automated contributors

Chinese: [README.zh-CN.md](README.zh-CN.md). Every document except `AGENT.md` has a Chinese
version beside it.

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
