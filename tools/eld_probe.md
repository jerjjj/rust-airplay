# The ELD probe: does a dump's audio decode when it is fed the way upstream feeds it?

`tools/eld_probe.c` answers one question and nothing else: are the audio frames this port
receives valid AAC-ELD, or is the fault in the data? It reads a dump written by
`AIRPLAY_DUMP_AUDIO`, pushes every payload into GStreamer as **its own buffer**, with the
caps and the `AudioSpecificConfig` upstream uses, and reports how many buffers came back.

The answer it gave on a real dump: **300 buffers in, 300 out, 1 152 000 bytes of PCM.**
The frames are right. What was wrong was the pipeline that delivered them.

## What that settled

Three things were suspected in turn and each is now answered:

| Suspicion | Answer |
|---|---|
| the audio is decrypted wrongly | no: the first byte of every frame is `8e`/`8d`, which is on upstream's own list of valid openings (`raop_buffer.c:139-145`) |
| the client sends ALAC, not ELD | no: it says `type 96 (ct 8 (AAC-ELD), spf 480)` in `SETUP`, and `ct` is read now |
| the frames are not decodable | **no**: this probe decodes them, 300 out of 300 |

What was actually wrong, in two parts:

1. **The caps had no `codec_data`.** A raw AAC frame carries no sample rate of its own, so a
   decoder given one cannot fix its output format and refuses with "output caps are unfixed".
   Upstream's caps (`renderers/audio_renderer.c:68`) carry
   `codec_data=(buffer)f8e85000` — the `AudioSpecificConfig` for ELD, 44100 Hz, stereo,
   480 samples a frame. This port had guessed `f82c00`, which is not even ELD: its object
   type decodes to 33, and an earlier attempt that used it failed for that reason.

2. **A byte stream cannot carry frame boundaries.** `appsrc` gives the decoder one buffer per
   audio payload, and a buffer is a frame. `fdsrc` gives it chunks of a byte stream, and raw
   AAC has no sync word for a parser to split on. Every framing that could stand in for the
   buffer boundary was tried and does not work here:
   * `aacparse` on raw AAC: no sync word, nothing to split on;
   * ADTS: its two-bit profile field cannot express AAC-ELD (object type 39);
   * LOAS, which can express ELD: `aacparse` refuses the result.

So the port's audio path — a subprocess fed bytes on stdin — is the thing that cannot work
as designed. Audio needs `appsrc`, which means linking GStreamer rather than running
`gst-launch`, which is what upstream does and why upstream plays sound.

## Building and running it

Needs the MSYS2 UCRT64 GStreamer development libraries, which this machine has
(`pkg-config --modversion gstreamer-1.0` reports 1.28.3).

```powershell
$env:PATH = "C:\msys64\ucrt64\bin;$env:PATH"
$flags = & C:\msys64\ucrt64\bin\pkg-config.exe --cflags --libs gstreamer-1.0 gstreamer-app-1.0
& C:\msys64\ucrt64\bin\gcc.exe -O1 -o target/eld_probe.exe tools/eld_probe.c ($flags -split '\s+')
.\target\eld_probe.exe target\client-audio.bin
```

Exit code 0 means the frames decoded; 2 means nothing decoded and the data is the problem.
