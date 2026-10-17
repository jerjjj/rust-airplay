# rust-airplay-gstreamer

A GStreamer renderer for [`rust-airplay`](https://crates.io/crates/rust-airplay).

The crate implements the `Renderer` trait in two halves. Video goes to a `gst-launch-1.0`
subprocess over its standard input, because H.264 carries start codes and a byte stream
therefore keeps its own framing. Audio is driven in-process through `appsrc`, one buffer per
frame, because a raw AAC-ELD frame has neither a sync word nor a sample rate, so nothing in a
byte stream says where a frame ends.

## Requirements

Building needs the GStreamer development files, which are found with `pkg-config`:

- `gstreamer-1.0`
- `gstreamer-app-1.0`

Running needs a GStreamer runtime with the elements a session names:

| Purpose | Elements |
| --- | --- |
| Video | `h264parse`, `avdec_h264` or `d3d11h264dec`, `videoconvert`, a sink |
| Audio | `avdec_aac` or `avdec_alac`, `audioconvert`, `audioresample`, a sink |

On Windows, MSYS2 UCRT64 provides both; put `C:\msys64\ucrt64\bin` on `PATH`. On Debian and
Ubuntu, `libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev` are enough to build, and
the plugin packages for the elements above to run.

## Using it

```rust,no_run
use std::sync::Arc;
use rust_airplay::Renderer;

# fn example() {
let renderer: Arc<dyn Renderer> = Arc::new(
    rust_airplay_gstreamer::GstRenderer::new().expect("a GStreamer runtime"),
);
# }
```

The renderer is usually built once and shared, since it also holds the warm video pipeline
when that is enabled.

## Environment

| Variable | Effect |
| --- | --- |
| `AIRPLAY_GST_LAUNCH` | the `gst-launch-1.0` executable to run |
| `AIRPLAY_GST_AUDIO_SINK` | audio sink; `wasapisink` on Windows |
| `AIRPLAY_GST_VIDEO_SINK` | video sink; `autovideosink` by default |
| `AIRPLAY_GST_VIDEO_WARM` | set to `1` to start the video pipeline with the receiver |
| `AIRPLAY_GST_KEEP_STDERR` | keep the subprocess's error output instead of redirecting it to a file |

A window sink takes about a second to come up before it has been given a frame, so the first
client waits for it. `AIRPLAY_GST_VIDEO_WARM` pays that cost at startup instead, and the
pipeline is then reused between sessions. On Windows, `d3d11videosink` starts faster than the
default and can take the frames `d3d11h264dec` produces without a copy.

## Tests

```sh
cargo test
cargo test -- --ignored
```

The ignored tests need a GStreamer runtime. They decode frames captured from a real session,
which is what makes them useful: a pipeline can accept every buffer and still decode none,
and only the decoded output settles the question.
