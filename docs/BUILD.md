# Building and running

Chinese: [BUILD.zh-CN.md](BUILD.zh-CN.md).

## Toolchain

Rust 1.85 or newer, edition 2024. The workspace pins `rust-version` in `Cargo.toml`; `cargo
build` reports it rather than failing obscurely on an older toolchain.

A C linker is needed for the Rust toolchain itself, not for this code: on Windows the MSVC
one that rustup installs with the default host triple, on macOS and Linux the system
compiler.

## The default build

```sh
cargo build --workspace
```

This links no C library. `cargo tree --workspace` shows no `-sys` crate, which is a
deliberate property: a machine without GStreamer, without a sound card and without a
display still builds, tests and lints the whole workspace.

## The GStreamer backend

`rust-airplay-gstreamer` is a crate of its own, excluded from the workspace so that the
gates above never need system libraries.

### Required at build time

The GStreamer development files, found through `pkg-config`:

| Module | Provided by |
|---|---|
| `gstreamer-1.0` | the GStreamer core development package |
| `gstreamer-app-1.0` | the GStreamer plugins-base development package (or `gstreamer-app-1.0` directly) |
| `glib-2.0`, `gobject-2.0`, `gio-2.0` | GLib development files, which the GStreamer packages depend on |

### Windows, MSYS2 UCRT64

Install the MSYS2 packages for GStreamer and its development files, then make sure
`pkg-config` can be found:

```powershell
$env:PATH = 'C:\msys64\ucrt64\bin;' + $env:PATH
pkg-config --modversion gstreamer-1.0
cargo build -p rust-airplay-cli --features gst
```

The import libraries MSYS2 ships are MinGW ones. They link with the MSVC toolchain here in
practice, but that combination is not a supported configuration of either project; if the
link step fails on this machine, build the library and use the `gst-launch` subprocess path
in whatever form your GStreamer installation supports.

### Debian and Ubuntu

```sh
sudo apt-get install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
cargo build -p rust-airplay-cli --features gst
```

### macOS

Untested. The GStreamer development files installed through the official installer or
Homebrew should be found by `pkg-config` once its path is exported. Please treat the
resulting binary as unverified.

## Required at run time

A GStreamer runtime with these elements available:

| Purpose | Elements |
|---|---|
| Mirroring, video | `h264parse`, `avdec_h264` (or `d3d11h264dec` on Windows), `videoconvert`, a sink (`autovideosink`, `d3d11videosink`) |
| Audio | `avdec_aac` for AAC-ELD, `avdec_alac` for ALAC, `audioconvert`, `audioresample`, a sink (`wasapisink` on Windows, `autoaudiosink` elsewhere) |

Two dependencies a reader often expects and this project does not have:

- **No Bonjour or Avahi.** Discovery is implemented in Rust by `mdns-sd`; no system DNS-SD
  daemon is involved.
- **No separate FFmpeg install.** `avdec_h264` and `avdec_aac` come from GStreamer's libav
  plugin pack.

A missing element is reported when a stream is opened, not as silence.

## Environment variables

| Variable | Effect |
|---|---|
| `AIRPLAY_GST_LAUNCH` | the `gst-launch-1.0` to run |
| `AIRPLAY_GST_AUDIO_SINK` | audio sink name |
| `AIRPLAY_GST_VIDEO_SINK` | video sink name |
| `AIRPLAY_GST_VIDEO_WARM` | `1` starts the video pipeline with the receiver instead of with the first client |
| `AIRPLAY_DUMP_VIDEO` | append the first video frames, as Annex-B, to this file |
| `AIRPLAY_DUMP_AUDIO` | append the audio payloads, length-prefixed, to this file |

## The differential test

`cargo test -p rust-airplay --test upstream_oracle -- --ignored` compares this
implementation against UxPlay's own C code: keystream positions, decryptor carry behaviour,
pairing vectors and the counter arithmetic.

It needs:

1. a UxPlay checkout at the commit recorded in `research/evidence/upstream-commit.txt`,
   placed at `upstream/UxPlay` (that directory is ignored by git);
2. the oracle built with `pwsh -File tools/oracle/build.ps1`, which expects MSYS2/UCRT64;
3. `AIRPLAY_UPSTREAM_ORACLE` pointing at the built executable, or `pwsh -File
   tools/oracle/run.ps1`, which sets it.

This suite does not run in CI: the upstream checkout is not vendored and the oracle build
script drives a Windows toolchain.

## Common problems

| Symptom | Cause |
|---|---|
| `os error 10013` or `10048` on the control port | the port is reserved or already in use; Windows reserves ranges for Hyper-V. Choose another, or `--port 0` |
| `--render gst` reports the feature is missing | the binary was built without `--features gst` |
| The receiver starts and no client finds it | check that mDNS multicast is not blocked, and that the receiver and the client are on the same network |
| `LNK1104: cannot open file ...rust_airplay.exe` while rebuilding | a receiver from a previous run is still running; stop it, or copy the binary elsewhere and run the copy |
| The picture stalls and frames are dropped | the sink cannot keep up; see the note on sinks in `crates/rust-airplay-gstreamer/README.md` |
