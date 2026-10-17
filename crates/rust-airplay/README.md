# rust-airplay

An AirPlay 1/2 receiver as a library.

`rust-airplay` advertises a receiver over mDNS, answers the AirPlay control protocol, pairs
clients, receives screen mirroring and audio, decrypts them, and hands frames to a renderer.
It opens no files, reads no environment variables and prints nothing, so the program that
embeds it decides where the logs go and where the identity is kept.

## Adding it

```toml
[dependencies]
rust-airplay = "0.1"
```

Rust 1.85 or newer. The crate links no C library.

## Starting a receiver

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

`run` serves clients until the process is asked to stop. Two fields are worth explaining
because they are where a receiver's behaviour across restarts comes from. `seed` is the
secret the identity derives from, so passing the same seed keeps the receiver recognisable.
`clients` is the set of client keys already verified, and `on_client` is called when a new
one joins, which is the moment to write it down.

## Playing the media

Frames arrive at `Renderer`, which a calling program implements. `NullRenderer` accepts
everything and plays nothing, which is useful for testing and for inspecting a session on a
machine with no sound card.

The GStreamer implementation is a separate crate:

```toml
[dependencies]
rust-airplay = "0.1"
rust-airplay-gstreamer = "0.1"
```

## Logging

Records go through the `log` crate at `info`, `warn` and `debug`. Nothing is printed without
a logger installed. The PIN a client must type is among the `info` records, so a program that
wants to show it prominently should install a logger that does.

## Licence

GPL-3.0-or-later.

中文：[README.zh-CN.md](README.zh-CN.md)。
