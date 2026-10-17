# The GStreamer backend

This crate renders a session's audio and video through GStreamer. It is **not** a member of
the default workspace: its gates need the GStreamer development libraries.

```powershell
cargo test --workspace --all-targets                                 # the default gates
cargo test -p airplay-renderer-gst -- --ignored --test-threads=1     # the ones that need a runtime
```

## What it needs

To **build**: the GStreamer development files, found with `pkg-config` (`gstreamer-1.0`,
`gstreamer-app-1.0`). MSYS2/UCRT64's install provides them, and `pkg-config.exe` there is on
the system `PATH` on this machine.

To **run**: a GStreamer runtime with `gst-launch-1.0` on `PATH` (video), and the plugins for
what a client sends — a decoder for the audio coding (`alac` or AAC-ELD), one for H.264, and
a sink:

```powershell
$env:PATH = 'C:\msys64\ucrt64\bin;' + $env:PATH
gst-inspect-1.0 wasapisink | Select-Object -First 3
```

`AIRPLAY_GST_LAUNCH` names another `gst-launch-1.0`; `AIRPLAY_GST_AUDIO_SINK` and
`AIRPLAY_GST_VIDEO_SINK` name another sink. `fakesink` accepts everything and needs no
device, which is how the tests check a pipeline on a machine with no sound card.

## Why it is shaped this way

[ADR-0003](../../docs/ADR-0003-media-backend.md) chose GStreamer first and said the backend
must not stop the default workspace's gates from passing on a machine without it. That
leaves three ways to reach GStreamer on Windows:

1. **Link it with the official bindings.** Needs a GStreamer SDK built for the same
   toolchain. The usual Windows GStreamer is MSYS2's, whose import libraries the MSVC
   linker does not accept, so a contributor would need a second Rust toolchain.
2. **Hand-written FFI, loaded at run time.** Works, keeps the build clean, and costs an
   `unsafe` surface that has to be maintained against GStreamer's ABI by hand.
3. **Run `gst-launch-1.0` as a subprocess** and write frames to its standard input. No
   `unsafe`, no toolchain coupling, no ABI to track — the pipeline is GStreamer's own.

**Video takes the third. Audio takes the first, and has to.** This was settled by
measurement, not preference (`tools/eld_probe.md` records it):

- H.264 and H.265 carry start codes, so a decoder finds picture boundaries in a byte
  stream. Video is a subprocess, and works.
- A raw AAC-ELD frame has **no sync word and no sample rate**. A byte stream says nothing
  about where a frame ends, and every framing tried as a substitute fails: `aacparse` has
  nothing to split on, ADTS's two-bit profile cannot express object type 39, and LOAS —
  which can — is refused by `aacparse`.

So audio is delivered as `appsrc`, one buffer per frame, which is what upstream does and why
upstream plays sound. It also means the caps matter in a way they never did for video: they
must carry `codec_data`, the `AudioSpecificConfig` a raw frame cannot carry
(`f8e85000` for ELD at 44.1 kHz stereo). Without it every decoder refuses the stream with
"output caps are unfixed", and nothing says so until forty frames in.

What the split costs: this crate needs GStreamer's development files to build, so it is
built by the `--features airplay-cli/gst` gate rather than the default one. The default
workspace — `airplay-core`, `airplay-renderer`, `airplay-cli` without the feature — still
links nothing.

## What the tests cover

The tests that need no runtime check the descriptions this crate builds and the errors it
reports. The `#[ignore]`d ones drive a real runtime and check the part that is not a thin
wrapper:

- a pipeline the runtime accepts, for every coding the renderer boundary names;
- bytes written to a pipe are what the pipeline reads (`wavparse` proves they arrived
  intact);
- a parameter set behind a start code is taken by the video pipeline;
- a sink a machine does not have is reported when a stream is opened, not as silence;
- the volume a session sets is read back;
- **forty frames of a real AAC-ELD session are decoded into PCM**
  (`tests/audio.rs`), which is the check that would have caught this backend playing
  nothing: `appsrc` accepts a buffer long before a decoder refuses it, so a test that
  stopped at "push returned Ok" passes in silence.

They are `#[ignore]`d because they need a GStreamer runtime, which not every machine has.

## What the tests keep

`tests/fixtures/mirror-first-frames.h264` is the first thirty frames of a real
session: an iPad mirroring a 1554x1080 screen, as this backend handed them to a
decoder. `tests/fixtures/mirror-first-audio.eld` is forty audio payloads from a session of
the same kind, each behind its length. Both are committed because they are the only
evidence that survives a session — a device is not available to a test, and "the picture
looks wrong" cannot be asserted, while "the pipeline decoded every frame" and "the decoder
produced PCM" can.

Two tests read the video fixture, and they are the ones to run first when a picture is
wrong:

```powershell
$env:PATH = 'C:\msys64\ucrt64\bin;' + $env:PATH
cargo test --test decoding -- --ignored --test-threads=1 --nocapture
```

They decode the fixture through this backend's own pipeline and through the one a
player would build, and compare the counts. Both report 64 frames of 65, the last
still inside the decoder when the stream ends. If the two ever disagree, the
difference is which decoder each chose.