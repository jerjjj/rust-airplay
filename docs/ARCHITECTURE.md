# Architecture

Chinese: [ARCHITECTURE.zh-CN.md](ARCHITECTURE.zh-CN.md).

## Crates

```
rust-airplay-cli          the command-line receiver
        |
        v
rust-airplay              the library
  +-- receiver            the session: ports, discovery, keys, streams
  +-- server              the control protocol over TCP
  +-- media               audio, mirror, timing, the sockets
  +-- discovery           the mDNS advertisement
  +-- pairing, srp, crypto, fairplay, plist, wire
  +-- render              the boundary a renderer plugs into
        ^
        |
rust-airplay-gstreamer    a renderer
```

The library depends on no other crate in this repository. The backend depends on the
library and on GStreamer. The program depends on the library and optionally on the backend.

## A session

1. The client finds the receiver over mDNS and opens a TCP connection.
2. `/info` returns the same text records that were advertised.
3. `/pair-verify` recognises a known client; `/pair-setup` pairs a new one with a PIN.
4. `SETUP` names the streams. The receiver answers with the ports it bound and derives the
   keys the session needs.
5. `RECORD` starts playback.
6. Media arrives on the timing, control and data sockets, and the mirrored screen on a TCP
   listener. Frames are decrypted and ordered, then handed to the renderer.
7. `TEARDOWN`, or the connection closing, ends the session.

The audio key arrives in one `SETUP`; the mirror stream's identifier arrives in a later one,
without the key. They may be different connections, so the key is session state rather than
connection state.

## Rendering

Frames cross a trait boundary as owned buffers with a timestamp. A backend that blocked the
receive loop would stall everything the loop watches, so every write happens on a thread of
its own, fed by a bounded queue. A full queue drops the oldest frame, which bounds the lag.

The GStreamer backend delivers the two media differently:

- Video is a `gst-launch-1.0` subprocess, because H.264 carries start codes and so a byte
  stream keeps its own framing.
- Audio is linked and pushed as `appsrc`, because a raw AAC-ELD frame has neither a sync
  word nor a sample rate, so one buffer per frame is the only way to deliver it.

That is why GStreamer is a dependency of the backend crate only, and why the default
workspace links no C library.

## Discovery

Both `_raop._tcp` and `_airplay._tcp` are registered, with the text records a current client
expects. The values matter beyond appearance: for example bit 27 of the feature bitmap
selects which client-pairing protocol is used, so it is clear for the current one. The tests
pin the record field by field.

## Identity

An identity is a 32 byte seed. The device identifier is derived from it, so one seed is one
receiver across restarts. Client keys are remembered once a session has carried them.

The library stores nothing. A program that persists the seed and the client list is what
makes a receiver recognisable.
