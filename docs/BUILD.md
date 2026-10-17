# Build guide

Chinese: [BUILD.zh-CN.md](BUILD.zh-CN.md).

## Toolchain

The workspace needs Rust 1.85 or newer, because it uses edition 2024. The version is pinned
in `Cargo.toml`, so an older toolchain refuses to build with a message saying what it wants
instead of failing somewhere inside a macro.

A C linker is needed for Rust itself, not for this code. On Windows that means the MSVC
build tools, which `rustup` installs with the default toolchain. On macOS and Linux the
system compiler is enough.

## Building the workspace

```sh
cargo build --workspace
```

This builds the library and the command-line receiver and links no C library.
`cargo tree --workspace` shows no `-sys` crate, and that is a property worth keeping: a
machine with no GStreamer, no sound card and no display can still build, test and lint the
whole workspace.

## Building the GStreamer backend

```sh
cargo build -p rust-airplay-cli --features gst
```

The backend is a crate of its own and is excluded from the workspace, so the default gates
never need system libraries. It does need the GStreamer development files, which
`pkg-config` is expected to find:

| Module | Package |
| --- | --- |
| `gstreamer-1.0` | GStreamer core development files |
| `gstreamer-app-1.0` | GStreamer plugins-base development files |
| `glib-2.0`, `gobject-2.0`, `gio-2.0` | GLib development files |

On Windows, MSYS2 UCRT64 provides all of them. Put its `bin` directory on `PATH` so
`pkg-config.exe` and the GStreamer tools are found:

```powershell
$env:PATH = 'C:\msys64\ucrt64\bin;' + $env:PATH
pkg-config --modversion gstreamer-1.0
cargo build -p rust-airplay-cli --features gst
```

On Debian and Ubuntu:

```sh
sudo apt-get install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
```

On macOS, install the GStreamer development files from the official installer or from
Homebrew, and export the pkg-config path. The backend has not been linked against a macOS
GStreamer here, so treat the first build on that platform as unexplored.

## What GStreamer needs at run time

The pipeline names its elements, so what has to be installed is visible in the element names
a missing-plugin error reports. In general:

| Purpose | Elements |
| --- | --- |
| Video | `h264parse`, `avdec_h264` or `d3d11h264dec`, `videoconvert`, a sink |
| Audio | `avdec_aac` or `avdec_alac`, `audioconvert`, `audioresample`, a sink |

Two dependencies people expect and this project does not need. There is no Bonjour or Avahi
requirement, because discovery is implemented in the crate with `mdns-sd` and no system
daemon is involved. There is also no separate FFmpeg installation, because `avdec_h264` and
`avdec_aac` arrive with GStreamer's libav plugin package.

## Environment

| Variable | Effect |
| --- | --- |
| `AIRPLAY_GST_LAUNCH` | the `gst-launch-1.0` executable to run |
| `AIRPLAY_GST_AUDIO_SINK` | audio sink, `wasapisink` on Windows by default |
| `AIRPLAY_GST_VIDEO_SINK` | video sink, `autovideosink` by default |
| `AIRPLAY_GST_VIDEO_WARM` | set to `1` to start the video pipeline with the receiver |
| `AIRPLAY_DUMP_VIDEO` | append the first video frames to this file, as Annex-B |
| `AIRPLAY_DUMP_AUDIO` | append the audio payloads to this file, length-prefixed |

The dump files are for diagnosis. The video one is an Annex-B stream that any decoder can
read, which separates a malformed stream from a stream that is merely hard to follow. The
audio one stores each payload behind a four-byte little-endian length, which is enough to
recover frame boundaries for a decoder the receiver did not write.

## Tests

The gates that must pass before a change is finished:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Two suites need things the workspace does not provide, so they are not part of that list.

The differential test compares this implementation with UxPlay's own C code. It needs a
checkout of UxPlay at commit `57ea83411d5f7e0b38c5841987439340543f025c` in `upstream/UxPlay`,
MSYS2 to compile it, and then:

```sh
pwsh -File tools/oracle/run.ps1
```

The GStreamer suite drives a real runtime, including decoding frames captured from a
session:

```sh
cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --ignored
```

## Troubleshooting

| Symptom | Cause and fix |
| --- | --- |
| `os error 10013` or `10048` on the control port | the port is reserved or already in use. Windows reserves ranges for Hyper-V; `netsh int ipv4 show excludedportrange protocol=tcp` lists them. Use another port, or `--port 0` |
| `--render gst` says the feature is missing | the binary was built without `--features gst` |
| A client does not see the receiver | mDNS multicast is blocked, or the two are on different networks. Both have to be on the same link |
| `LNK1104: cannot open file ...rust_airplay.exe` while rebuilding | a receiver from an earlier run is still running and holds the file. Stop it, or run a copy of the binary somewhere else |
| The picture stalls and the log reports dropped frames | the sink cannot keep up. On Windows try `AIRPLAY_GST_VIDEO_SINK=d3d11videosink`, and set `AIRPLAY_GST_VIDEO_WARM=1` so the window is created before the first client |
| Audio plays but video does not, or the reverse | the element for one of them is missing; the error names it. Install the plugin package that provides it |
