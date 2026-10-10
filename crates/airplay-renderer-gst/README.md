# The GStreamer backend

This crate renders a session's audio and video through GStreamer. It is **not** a member of
the default workspace, and it is built and tested on its own:

```powershell
cd crates/airplay-renderer-gst
cargo test                                   # the tests that need nothing
cargo test -- --ignored --test-threads=1     # the ones that need a GStreamer runtime
```

## What it needs

A GStreamer runtime with `gst-launch-1.0` on `PATH`, and the plugins for what a client
sends: a decoder for the audio coding (`alac` or AAC-ELD) and one for H.264, plus a sink.
This machine's MSYS2/UCRT64 install (GStreamer 1.28.3) has all of them:

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
leaves three ways to reach GStreamer on Windows, and this crate takes the third:

1. **Link it with the official bindings.** Needs a GStreamer SDK built for the same
   toolchain. The usual Windows GStreamer is MSYS2's, whose import libraries the MSVC
   linker does not accept, so a contributor would need a second Rust toolchain. Rejected
   for a repository anyone should be able to clone and build.
2. **Hand-written FFI, loaded at run time.** Works, keeps the build clean, and costs an
   `unsafe` surface that has to be maintained against GStreamer's ABI by hand.
3. **Run `gst-launch-1.0` as a subprocess** and write frames to its standard input. No
   `unsafe`, no toolchain coupling, no ABI to track — the pipeline is GStreamer's own.

The third is what this crate does. What it costs is that the pipeline is a description
rather than a graph this code builds: a caller cannot reach into it to set a property, and
a sink has to be one `gst-launch` knows by name. For the operations a session needs —
open, push, flush, close — that is not a limitation, and `S4-3` is where the ones that need
a property (volume) have to be looked at again.

It also means the runtime is found at run time, which is the right failure mode: a machine
without GStreamer gets a receiver that says so, not one that will not start.

## What the tests cover

The tests that need no runtime check the descriptions this crate builds and the errors it
reports. The `#[ignore]`d ones drive a real runtime and check the part that is not a thin
wrapper:

- a pipeline the runtime accepts, for every coding the renderer boundary names;
- bytes written to a pipe are what the pipeline reads (`wavparse` proves they arrived
  intact);
- a parameter set behind a start code is taken by the video pipeline;
- a sink a machine does not have is reported when a stream is opened, not as silence;
- the volume a session sets is read back.

They are `#[ignore]`d for the same reason the crate is: the default workspace's gates run
where GStreamer is not installed.

## What the tests keep

`tests/fixtures/mirror-first-frames.h264` is the first thirty frames of a real
session: an iPad mirroring a 1554x1080 screen, as this backend handed them to a
decoder. It is committed because it is the only evidence that survives a session —
a device is not available to a test, and "the picture looks wrong" cannot be
asserted, while "the pipeline decoded every frame" can.

Two tests read it, and they are the ones to run first when a picture is wrong:

```powershell
$env:PATH = 'C:\msys64\ucrt64\bin;' + $env:PATH
cargo test --test decoding -- --ignored --test-threads=1 --nocapture
```

They decode the fixture through this backend's own pipeline and through the one a
player would build, and compare the counts. Both report 64 frames of 65, the last
still inside the decoder when the stream ends. If the two ever disagree, the
difference is which decoder each chose.