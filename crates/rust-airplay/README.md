# rust-airplay

An AirPlay 1/2 receiver as a library.

It advertises a receiver over mDNS, answers the AirPlay control protocol, pairs clients,
receives screen mirroring and audio, decrypts them, and hands frames to a renderer. It
contains no command line, no file access and no printing, so the program embedding it keeps
those decisions.

## Usage

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

`run` serves clients until the process is asked to stop.

## Playing media

Frames are delivered to `Renderer`. `NullRenderer` accepts everything and plays nothing,
which is useful for inspecting a session. For playback, implement `Renderer` or use the
[`rust-airplay-gstreamer`](https://crates.io/crates/rust-airplay-gstreamer) crate.

## Requirements

Rust 1.85 or newer. No C library is linked.

## Documentation

- [API reference](https://docs.rs/rust-airplay)
- [README](https://github.com/jerjjj/rust-airplay) - the command-line receiver and the build guide

## Licence

GPL-3.0-or-later.

中文：[README.zh-CN.md](README.zh-CN.md)。
