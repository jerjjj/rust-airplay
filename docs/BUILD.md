# Build guide

Chinese: [BUILD.zh-CN.md](BUILD.zh-CN.md).

## Requirements

Rust 1.85 or newer (edition 2024), and a C linker for the Rust toolchain itself: MSVC on
Windows, the system compiler on macOS and Linux.

The default workspace links no C library. GStreamer is only needed for the playback backend.

## Build

```sh
cargo build --workspace
cargo build -p rust-airplay-cli --features gst
```

## GStreamer

The backend crate is excluded from the workspace, so the gates above never need system
libraries.

### Development files

Found with `pkg-config`:

| Module | Package |
|---|---|
| `gstreamer-1.0` | GStreamer core development files |
| `gstreamer-app-1.0` | GStreamer plugins-base development files |
| `glib-2.0`, `gobject-2.0`, `gio-2.0` | GLib development files |

Windows, MSYS2 UCRT64:

```powershell
$env:PATH = 'C:\msys64\ucrt64\bin;' + $env:PATH
pkg-config --modversion gstreamer-1.0
cargo build -p rust-airplay-cli --features gst
```

Debian and Ubuntu:

```sh
sudo apt-get install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
```

macOS: install the GStreamer development files and export the path to `pkg-config`.

### Runtime elements

| Purpose | Elements |
|---|---|
| Video | `h264parse`, `avdec_h264` or `d3d11h264dec`, `videoconvert`, a sink |
| Audio | `avdec_aac` or `avdec_alac`, `audioconvert`, `audioresample`, a sink |

No Bonjour or Avahi is needed: discovery is implemented in Rust. No separate FFmpeg install
is needed either: the decoders come from GStreamer's libav plugin pack.

## Run

```sh
cargo run -p rust-airplay-cli --features gst -- --name "my receiver" --port 7000 --render gst
```

## Environment variables

| Variable | Effect |
|---|---|
| `AIRPLAY_GST_LAUNCH` | the `gst-launch-1.0` to run |
| `AIRPLAY_GST_AUDIO_SINK` | audio sink name |
| `AIRPLAY_GST_VIDEO_SINK` | video sink name |
| `AIRPLAY_GST_VIDEO_WARM` | `1` starts the video pipeline with the receiver |
| `AIRPLAY_DUMP_VIDEO` | append the first video frames to this file, as Annex-B |
| `AIRPLAY_DUMP_AUDIO` | append the audio payloads to this file, length-prefixed |

## Tests

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Two suites need more than the workspace:

```sh
pwsh -File tools/oracle/run.ps1
cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --ignored
```

The first builds a C program from UxPlay (`tools/oracle/README.md` explains it) and compares
this implementation with it. It needs a local UxPlay checkout at commit
`57ea83411d5f7e0b38c5841987439340543f025c` in `upstream/UxPlay`, and MSYS2. The second needs
a GStreamer runtime.

## Troubleshooting

| Symptom | Cause |
|---|---|
| `os error 10013` or `10048` on the control port | port reserved or in use; Windows reserves ranges for Hyper-V. Choose another, or `--port 0` |
| `--render gst` reports a missing feature | the binary was built without `--features gst` |
| No client finds the receiver | check that mDNS multicast is not blocked, and that both are on the same network |
| `LNK1104: cannot open file ...rust_airplay.exe` while rebuilding | a receiver from a previous run is still running; stop it, or run a copy of the binary |
| The picture stalls and frames are dropped | the sink cannot keep up; try `AIRPLAY_GST_VIDEO_SINK=d3d11videosink` on Windows |
