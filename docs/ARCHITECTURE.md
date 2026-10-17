# Architecture

Chinese: [ARCHITECTURE.zh-CN.md](ARCHITECTURE.zh-CN.md).

## The crates

```
rust-airplay-cli          the command-line receiver
        |
        v
rust-airplay              the library
  receiver                the session: sockets, discovery, keys, streams
  server                  the control protocol over TCP
  media                   audio, mirror, timing, the datagram sockets
  discovery               the mDNS records
  pairing, srp, crypto, fairplay, plist, wire
  render                  the renderer boundary
        ^
        |
rust-airplay-gstreamer    a renderer
```

An arrow means "depends on". The library depends on no other crate in this repository, which
is what lets the workspace build on a machine with no media libraries at all. The backend
depends on the library and on GStreamer. The program depends on the library, and on the
backend when it is built with the `gst` feature.

## What a session looks like

A client finds the receiver through mDNS, opens a TCP connection to the control port, and
speaks RTSP over it.

1. `GET /info` returns the same text record the receiver advertises, so a client that read
   it either way sees one advertisement.
2. `POST /pair-verify` proves the client holds the key it paired with. A client that has
   never paired uses `POST /pair-setup`, where the SRP exchange and a PIN on screen prove
   the user is in the room.
3. `POST /fp-setup` is the FairPlay handshake a mirroring client expects before it will
   send encrypted video.
4. `SETUP` names the streams. The reply carries the ports the receiver bound, and the
   request carries the keys the session will use.
5. `RECORD` starts playback. Media then arrives on the timing, control and data sockets,
   and the mirrored screen on the mirror listener.
6. `TEARDOWN`, or the connection closing, ends the session.

The audio key and the mirror identifier are the reason the session is shared between
connections. A client sends the audio key in one `SETUP`, then sends the mirror stream's
`streamConnectionID` in a later `SETUP` without repeating the key. Since the receiver builds
a handler set per connection, per-connection state would lose the key; the key therefore
belongs to the session.

## Receiving media

Audio arrives as RTP over UDP. Each packet is decrypted with AES-CBC, where the reset is per
packet, and put into a bounded reordering window. When a sequence number is missing and the
client advertised one, the receiver asks for it again on the control socket; that request is
what turns a lost packet into a recoverable one. Frames leave the window in order, and a gap
that cannot be filled is counted and skipped so a stream that is missing one packet keeps
playing.

Mirroring arrives over TCP as packets behind a 128-byte header. The keystream for a packet
is AES-CTR over the stream's key and IV, and where a packet's bytes are taken from is the
subtle part: it starts where the previous packet ended, whole blocks at a time. A packet
that does not fill its final block leaves the rest of that block for the next packet, and
those bytes exist nowhere in the stream because they are the tail of a block that was
already read past. The counter then advances by the blocks the packet's own bytes filled.
This rule is checked against UxPlay's implementation rather than inferred, and the test that
does it is the reason it is written down this precisely here.

Frames leave the media path as Annex-B access units. A client that sends its parameter sets
separately gets them prepended to the frame they belong to.

## Rendering

Frames cross a trait boundary as owned buffers with a timestamp. The receive loop must never
wait for a renderer, so each stream is written from its own thread, fed by a bounded queue.
When a queue is full the oldest frame is dropped, which keeps the lag at the depth of the
queue instead of letting it grow; the picture may then be incomplete until the next key
frame, and the session says so in the log.

The GStreamer backend has to deliver the two media differently, and the reason is in the
formats. H.264 carries start codes, so a decoder can find picture boundaries in a byte
stream, which is what a `gst-launch-1.0` subprocess gets on its standard input. A raw
AAC-ELD frame has neither a sync word nor a sample rate, so a byte stream says nothing about
where one frame ends; the only way to deliver it is one buffer per frame through `appsrc`,
which means linking `libgstreamer`. That is why the backend is a separate crate that needs
the GStreamer development files while the default workspace links nothing.

## Discovery

The receiver registers both `_raop._tcp` and `_airplay._tcp` and fills their text records
with what a current client reads. Those values are load-bearing rather than cosmetic: bit 27
of the feature bitmap, for example, picks which client-pairing protocol the client uses.
Values are pinned field by field in the tests, and they were taken from a running receiver
rather than from a header, because a header records what a build starts from and the program
may change it before advertising.

## Identity

An identity is 32 bytes of seed. The device identifier and the keys a session uses are
derived from it, so one seed is one receiver, and a client that paired with it once
recognises it after a restart. A client key is remembered after a session has carried it,
which is the honest limit: a client that never completed a session has not proved anything
yet.

The library stores neither the seed nor the client list. A program that persists them is what
makes a receiver recognisable; a program that does not is a receiver that looks new every
time, and that is a legitimate choice for a receiver that should not remember anything.
