# rust-airplay

An AirPlay 1/2 receiver, written in safe Rust.

This crate is the whole receiver: it announces a service over mDNS, answers the control
protocol, pairs clients, derives the keys a media session needs, decrypts and orders what
arrives, and hands frames to whatever renderer you supply. It contains no command line, no
file access and no printing, so a program embedding it keeps those decisions.

A ready-made program is the [`rust-airplay-cli`](https://crates.io/crates/rust-airplay-cli)
package, and a GStreamer backend is the
[`rust-airplay-gstreamer`](https://crates.io/crates/rust-airplay-gstreamer) package.

## Status

What has been verified, and on what:

| Area | Verified on |
|---|---|
| Screen mirroring, audio playback | Windows, a real iPad, MSYS2 UCRT64 GStreamer 1.28 |
| Control plane, pairing, discovery | Windows, a real iPad |
| Everything else | the test suite only |

macOS and Linux build and pass their tests, but no device has been run against them. Say so
plainly in a program's documentation rather than promising more.

## Adding it

```toml
[dependencies]
rust-airplay = "0.1"
```

MSRV is Rust 1.85. The default build links no C library.

## Using it

A receiver needs four things from its embedder: an identity seed, a set of clients it
already trusts, a renderer, and somewhere to put the log records.

```rust,no_run
use std::sync::{Arc, Mutex};
use std::collections::HashSet;
use rust_airplay::{Config, NullRenderer};
use rust_airplay::server::PinMode;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = Config {
    name: "my receiver".to_owned(),
    port: 7000,
    seed: [0x42; 32],
    device_id: None,
    pin: PinMode::OneTime,
    password: None,
    audio_codec: rust_airplay::AudioCodec::AacEld,
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

[`run`] returns when the process is asked to stop, and reports a failure only for something
that prevents the receiver from starting. A session that fails later is reported through
`log` and does not stop the receiver.

## Playing media

Frames are delivered to [`Renderer`]. [`NullRenderer`] accepts everything and plays
nothing, which is how a session is checked on a machine with no sound card and no display.
For real playback, implement [`Renderer`], or use the GStreamer backend:

```rust,no_run
# #[cfg(feature = "unused")]
# fn example() {
use rust_airplay::Renderer;
let renderer: std::sync::Arc<dyn Renderer> =
    std::sync::Arc::new(rust_airplay_gstreamer::GstRenderer::new().unwrap());
# }
```

The GStreamer backend needs the GStreamer development files to build and a GStreamer runtime
with the usual plugins to run. See its own README.

## Logging

The crate emits records through the `log` facade and never prints. Install a logger in the
embedding program; without one, nothing is reported.

## Licence

GPL-3.0-or-later. This is not a preference: parts of the crate are translations of GPL and
LGPL sources from UxPlay, so a program that links it is bound by the same licence. The
attribution for each translated file is in `THIRD_PARTY_NOTICES.md` in the repository.

中文说明见 [README.zh-CN.md](README.zh-CN.md)。
