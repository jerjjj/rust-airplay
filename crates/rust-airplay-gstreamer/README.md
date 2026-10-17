# rust-airplay-gstreamer

A GStreamer renderer for [`rust-airplay`](https://crates.io/crates/rust-airplay).

It implements the `Renderer` trait: audio is delivered to a GStreamer pipeline in-process,
video is written to a `gst-launch-1.0` subprocess.

## Requirements

To build: the GStreamer development files, found with `pkg-config` (`gstreamer-1.0`,
`gstreamer-app-1.0`).

To run: a GStreamer runtime with these elements.

| Purpose | Elements |
|---|---|
| Video | `h264parse`, `avdec_h264` or `d3d11h264dec`, `videoconvert`, a sink |
| Audio | `avdec_aac` or `avdec_alac`, `audioconvert`, `audioresample`, a sink |

On Windows, MSYS2 UCRT64 provides both; put `C:\msys64\ucrt64\bin` on `PATH`.

## Usage

```rust,no_run
use std::sync::Arc;
use rust_airplay::Renderer;

# fn example() {
let renderer: Arc<dyn Renderer> = Arc::new(
    rust_airplay_gstreamer::GstRenderer::new().expect("a GStreamer runtime"),
);
# }
```

Environment variables:

| Variable | Effect |
|---|---|
| `AIRPLAY_GST_LAUNCH` | the `gst-launch-1.0` to run |
| `AIRPLAY_GST_AUDIO_SINK` | audio sink name, `wasapisink` on Windows |
| `AIRPLAY_GST_VIDEO_SINK` | video sink name, default `autovideosink` |
| `AIRPLAY_GST_VIDEO_WARM` | `1` starts the video pipeline with the receiver |
| `AIRPLAY_GST_KEEP_STDERR` | keep the subprocess's error output |

## Tests

```sh
cargo test
cargo test -- --ignored
```

The ignored tests need a GStreamer runtime. They decode frames captured from a session.

## Licence

GPL-3.0-or-later.

Chinese: [README.zh-CN.md](README.zh-CN.md).
