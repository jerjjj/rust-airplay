# Architecture

Chinese: [ARCHITECTURE.zh-CN.md](ARCHITECTURE.zh-CN.md).

This document explains how the parts fit and why they are split the way they are. It
replaces the decision records the project kept earlier; the decisions are restated here
with the reasoning that led to them.

## The shape

```
rust-airplay-cli          a program: arguments, files, printing
        |
        v
rust-airplay              library
  +-- receiver            the session: ports, discovery, keys, streams
  +-- server              the control protocol over TCP
  +-- media               audio, mirror, timing, the datagram sockets
  +-- discovery           the mDNS advertisement
  +-- pairing, srp, crypto, fairplay, plist, wire
  +-- render              the boundary a renderer plugs into
        ^
        |
rust-airplay-gstreamer    a renderer: audio in-process, video as a subprocess
```

An arrow means "depends on". The library depends on no other crate in this repository. The
backend depends on the library and on GStreamer. The program depends on the library and,
optionally, on the backend.

## Why the boundary sits there

A renderer is a decision the protocol half should not make: whether to play through
GStreamer, through something else, or through nothing at all. The library therefore carries
a small set of traits - `Renderer`, `AudioStream`, `VideoStream` - and hands frames across
it. Two implementations exist: `NullRenderer`, which accepts everything and plays nothing,
and the GStreamer backend.

The traits are deliberately thin. Frames cross as owned buffers with a timestamp, because a
session cannot lend memory to a backend it does not control, and because a backend that
blocked the receive loop would stall everything the loop watches. Every write is expected
to block when the backend is full; the receive loop never performs one. That is what made a
session with sound freeze before, and it is a property of the design now, not of one
backend.

## Why the command line is thin

Everything that can be a value is one: an identity seed, a client set, a trace file, a
renderer. The library reads no environment variable, chooses no path and prints nothing. A
program embedding it keeps those decisions, and the library stays testable - a test can run
a whole session with an identity of `[0x42; 32]` and a renderer that counts frames.

The one thing the library does own is the stop signal: `run` watches for the process being
interrupted, because a receive loop that outlived it would hold a sound card open with
nothing to play. A caller that wants a different trigger drops the future instead.

## The protocol path

A client finds the receiver over mDNS, opens a TCP connection, and speaks RTSP over it.

| Step | What happens |
|---|---|
| `/info` | the receiver answers with the same text records it advertises |
| `/pair-verify` | Curve25519, so an already-paired client is recognised without a PIN |
| `/pair-setup` | SRP-6a with a PIN, for a client that is not known yet |
| `/fp-setup` | the FairPlay handshake a mirroring client expects |
| `SETUP` | the client names its streams; the receiver answers with the ports it bound |
| `RECORD` | playback starts |
| `SET_PARAMETER` | volume, progress, metadata |

`SETUP` is where most of the interesting work hides. A client sends its audio key there,
and sends the mirror stream's connection identifier in a later `SETUP` without repeating
the key. Those two need not arrive on the same connection, because the receiver builds a
handler set per connection. The audio key is therefore session state rather than connection
state, and it is shared between handler sets.

## The media path

Three datagram sockets carry the media: timing, control and data. A fourth, a TCP listener,
carries the mirrored screen.

**Audio** arrives over RTP, is decrypted with AES-CBC where the reset is per packet, and is
put into a bounded reordering window. A packet that never arrives is asked for again on the
control socket, and the request is what makes a gap recoverable. Frames leave the window in
order; a gap that cannot be filled is counted and skipped rather than stalling the stream.

**Mirror** arrives over TCP as packets with a 128 byte header. The keystream is derived from
the stream's key and IV, and the rule for where a packet's bytes come from is not the
obvious one: a packet reads from where the previous packet ended, whole blocks at a time, so
a packet that does not fill its last block leaves the rest of that block for the next one -
and those bytes are nowhere in the stream, because they are the rest of a block that has
already been read past. The counter then advances by the blocks the packet's fresh bytes
filled. This is measured against UxPlay's own code rather than derived, and the test that
does it is the reason the rule is stated this precisely.

**Frames** come out of that path as Annex-B access units. Parameter sets a client sends
separately are prepended to the frame they belong to.

## Why GStreamer, and why in two shapes

Playing media needs decoders this project does not implement. GStreamer was chosen because
UxPlay uses it, which makes behaviour comparable and its failures recognisable.

The two media are delivered differently, and the difference is not a preference:

- **Video is a subprocess.** H.264 and H.265 carry start codes, so a decoder finds picture
  boundaries in a byte stream. `gst-launch-1.0` is started, and frames are written to its
  standard input.
- **Audio is linked.** A raw AAC-ELD frame has no sync word and no sample rate, so a byte
  stream says nothing about where a frame ends. The only way to deliver it is one buffer per
  frame, which means `appsrc`, which means linking `libgstreamer`.

That costs the crate a build-time dependency on the GStreamer development files, which is
why it is a separate crate and why the default workspace links no C library at all.

## What a session costs, and where the time goes

Measured on the machine this was developed on, with the committed real-session fixtures:

| What | Cost |
|---|---|
| Control plane, one request | under a millisecond |
| Receiver handshake, first request to first frame | about 340 ms |
| Video sink coming up, before any frame arrives | about 700 ms |
| Decoding thirty real 1554x1080 frames to a file sink | 354 ms |
| The same to a window sink | 1100 to 1750 ms, depending on the sink |

The sink is the reason a connection can feel slow, and it is why the backend can start its
video pipeline with the receiver (`AIRPLAY_GST_VIDEO_WARM`) rather than with the first
client: the window is created when the pipeline starts, not when a picture arrives.

## The advertisement is a contract

A receiver's mDNS text record decides how a client connects, and one field in it decided
more than the rest: bit 27 of the feature bitmap selects the client-pairing protocol. Set,
a client uses the older flow; clear, it uses the current one. This project originally
copied the constant out of UxPlay's header, which is the value a build starts from - UxPlay
clears that bit in code before advertising, and a running receiver reports a different
number than its header suggests.

The lesson is recorded here because it cost the most time of anything in this project: read
the advertisement from a running receiver, not from a constant in a header. The values this
port sends now match what a running UxPlay 1.74 reports, field for field, and the tests pin
them.

## Identity

A receiver's identity is a 32 byte seed. A device identifier is derived from it, so one seed
is one receiver across restarts, and clients that paired once recognise it again. Keys are
remembered per client only after a session has carried them: a client that never completed
one is not known yet, which is the honest limit of remembering verified keys.

The library does not store the seed. A caller that forgets it is a caller whose receiver
looks new every time, and that is the caller's decision to make.

## Testing

| Layer | How it is checked |
|---|---|
| Protocol parsers and primitives | unit tests, including property tests for the plist and wire formats |
| The control plane | tests that open a real TCP connection to a real server |
| The media session | tests that drive real UDP sockets and a real TCP connection |
| Keystream and counter rules | a differential test against UxPlay's own C code |
| The renderer boundary | a fake renderer counts what a session delivers |
| The GStreamer backend | a real runtime, including decoding frames captured from a device |

The differential test is the one that matters most: it is what turns "this looks right" into
"this agrees with the implementation it was ported from", and both serious defects in the
media path were found by it or because it existed.
