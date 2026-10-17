# API

Chinese: [API.zh-CN.md](API.zh-CN.md). Generated reference:
[docs.rs/rust-airplay](https://docs.rs/rust-airplay).

## The receiver

```rust
pub async fn run(config: Config) -> Result<(), Error>
```

Serves clients until the process is asked to stop. It binds the control service, announces
the receiver, sets up a media session per client, and hands frames to the configured
renderer. It returns `Ok(())` when the process is asked to stop, and `Err` only for
something that prevents a receiver from starting: a port that cannot be bound, a service
that cannot be announced, media sockets that cannot be opened. A session that fails later
is reported through `log` and does not stop the receiver.

The future is cancelled by dropping it; the receive loops and the renderer's pipelines are
released with it.

## `Config`

| Field | Meaning |
|---|---|
| `name: String` | the name clients see in the AirPlay menu |
| `port: u16` | control port; `0` binds an ephemeral one |
| `seed: [u8; 32]` | the bytes every identity key derives from; keep it to keep an identity |
| `device_id: Option<String>` | hardware address to announce; derived from the seed when `None` |
| `pin: PinMode` | `Disabled`, `OneTime` or `Fixed(digits)` |
| `password: Option<String>` | digest password required at `SETUP`, when set |
| `audio_codec: AudioCodec` | `Alac` or `AacEld`, the coding the client's frames carry |
| `renderer: Arc<dyn Renderer>` | what plays the media |
| `clients: Arc<Mutex<HashSet<[u8; 32]>>>` | keys of clients already verified |
| `on_client: Option<ClientCallback>` | called the first time a client is verified |
| `trace_log: Option<PathBuf>` | append the request trace here |
| `dump_video: Option<PathBuf>` | append the first video frames here, as Annex-B |
| `dump_audio: Option<PathBuf>` | append the audio payloads here, length-prefixed |

`Config` holds no paths of its own and reads no environment. Persistence, defaults and
printing belong to the embedder.

## Errors

`Error` carries a message and implements `Display` and `std::error::Error`. It is not an
enum, because the failures it reports are all "this receiver cannot start" and a caller
that wants to act differently inspects the message or checks the inputs first.

## The renderer boundary

```rust
pub trait Renderer: Debug + Send + Sync {
    fn open_audio(&self, config: AudioConfig) -> Result<Box<dyn AudioStream>, RenderError>;
    fn open_video(&self, config: VideoConfig) -> Result<Box<dyn VideoStream>, RenderError>;
    fn set_volume(&self, volume: Volume) -> Result<(), RenderError>;
}
```

`AudioStream` takes `AudioFrame`s and has `set_volume`, `flush` and `close`. `VideoStream`
takes `VideoFrame`s and has `set_pace`, `flush` and `close`. Streams are `'static` and
`Send + Sync`, because a session plays them on threads of its own: a stream that blocked a
receive loop would stall the whole session.

Contract worth knowing when implementing one:

- `push` blocks when the backend cannot take more. That backpressure is intended; the
  receive loop never calls it.
- `close` means "no more media"; it must not be assumed to destroy the stream.
- Dropping a stream is what releases whatever it holds.

`NullRenderer` accepts everything and plays nothing. It is the reference implementation of
the trait and the way to run a full session on a machine with no sound card.

## Media types

| Type | Contents |
|---|---|
| `AudioConfig` | codec, sample rate, channels, samples per frame |
| `AudioFrame` | coded payload, and the client's timestamp in its own clock |
| `VideoConfig` | size, when the client stated one |
| `VideoFrame` | Annex-B access unit, client timestamp, whether it is a key frame |
| `Volume` | silence to full, clamped |
| `RenderError` | `Unavailable` (nothing can play) or `Failed` (playback went wrong) |

An `AudioFrame`'s timestamp counts samples in the session's rate, not nanoseconds. A
renderer that schedules against a clock converts it; the GStreamer backend does.

## Modules

The lower layers are public, because a program may want a piece of the protocol without the
whole receiver. Nothing outside `receiver` is needed to run one.

| Module | Contents |
|---|---|
| `receiver` | `run`, `Config`, `Error`, `ClientCallback` |
| `render` | the traits and media types above |
| `discovery` | `Advertisement`, `Advertiser`, `Protection`, the text records |
| `server` | `Server`, `RouteHandlers`, `DeviceIdentity`, `PairingState`, `PinMode`, `SetupPorts` |
| `control` | the RTSP request parser and the route table |
| `media` | audio reordering, mirror decryption, the datagram sockets, the clock exchange |
| `pairing` | the `/pair-verify` and `/pair-setup` cryptography |
| `srp`, `crypto`, `fairplay`, `plist`, `rtp`, `ntp`, `wire` | the primitives underneath |

## The command line

`rust-airplay-cli` builds a binary called `rust-airplay`.

| Option | Meaning |
|---|---|
| `--name <name>` | name clients show |
| `--port <port>` | control port; `0` picks a free one |
| `--pin [<digits>]` | ask for a PIN, optionally fixed |
| `--no-pin` | do not ask for a PIN |
| `--password <password>` | also require the `SETUP` digest password |
| `--device-id <address>` | hardware address to announce |
| `--seed <hex>` | 32 bytes of identity seed |
| `--log <file>` | append the request trace |
| `--render <null\|gst>` | what plays the media |
| `--audio-codec <alac\|aac-eld>` | the coding a client sends |
| `--help` | usage text |

Its files, under the user's local application data directory: `identity.seed` and
`clients`. Deleting the first changes the receiver's identity; deleting the second makes
every client pair again.

## Logging

The library emits `log` records at `info` for progress, `warn` for a session that went
wrong, and `debug` for packet-level detail. Install a logger to see them; without one the
receiver runs silently. The CLI installs one that prints to standard output.
