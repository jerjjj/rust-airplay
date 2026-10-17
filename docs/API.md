# API

Chinese: [API.zh-CN.md](API.zh-CN.md). Generated reference:
[docs.rs/rust-airplay](https://docs.rs/rust-airplay).

## `run`

```rust
pub async fn run(config: Config) -> Result<(), Error>
```

Serves clients until the process is asked to stop. Binds the control service, announces the
receiver, sets up a media session per client and hands frames to the renderer. Returns
`Err` only when a receiver cannot start: a port that cannot be bound, a service that cannot
be announced, media sockets that cannot be opened.

## `Config`

| Field | Meaning |
|---|---|
| `name: String` | the name clients see |
| `port: u16` | control port; `0` picks a free one |
| `seed: [u8; 32]` | the bytes the receiver's keys derive from |
| `device_id: Option<String>` | hardware address to announce; derived from the seed when `None` |
| `pin: PinMode` | `Disabled`, `OneTime` or `Fixed(digits)` |
| `password: Option<String>` | digest password required at `SETUP` |
| `audio_codec: AudioCodec` | `Alac` or `AacEld` |
| `renderer: Arc<dyn Renderer>` | what plays the media |
| `clients: Arc<Mutex<HashSet<[u8; 32]>>>` | keys of clients already verified |
| `on_client: Option<ClientCallback>` | called the first time a client is verified |
| `trace_log: Option<PathBuf>` | append the request trace here |
| `dump_video: Option<PathBuf>` | append the first video frames here, as Annex-B |
| `dump_audio: Option<PathBuf>` | append the audio payloads here, length-prefixed |

## Renderer

```rust
pub trait Renderer: Debug + Send + Sync {
    fn open_audio(&self, config: AudioConfig) -> Result<Box<dyn AudioStream>, RenderError>;
    fn open_video(&self, config: VideoConfig) -> Result<Box<dyn VideoStream>, RenderError>;
    fn set_volume(&self, volume: Volume) -> Result<(), RenderError>;
}
```

`AudioStream` takes `AudioFrame`s and has `set_volume`, `flush` and `close`. `VideoStream`
takes `VideoFrame`s and has `set_pace`, `flush` and `close`.

- `push` blocks when the backend cannot take more; the receive loop never calls it.
- `close` means no more media for this session.
- Dropping a stream releases what it holds.

`NullRenderer` accepts everything and plays nothing.

## Media types

| Type | Contents |
|---|---|
| `AudioConfig` | codec, sample rate, channels, samples per frame |
| `AudioFrame` | coded payload and a timestamp counted in samples |
| `VideoConfig` | size, when the client stated one |
| `VideoFrame` | Annex-B access unit, timestamp, whether it is a key frame |
| `Volume` | silence to full, clamped |
| `RenderError` | `Unavailable` or `Failed` |

## Modules

`receiver` is all a program needs to run a receiver. The rest is public so a program can use
part of the protocol on its own.

| Module | Contents |
|---|---|
| `receiver` | `run`, `Config`, `Error`, `ClientCallback` |
| `render` | the renderer traits and media types |
| `discovery` | `Advertisement`, `Advertiser`, `Protection` |
| `server` | `Server`, `RouteHandlers`, `DeviceIdentity`, `PairingState`, `PinMode` |
| `media` | audio reordering, mirror decryption, the datagram sockets, the clock exchange |
| `control` | RTSP request parsing and the route table |
| `pairing` | the `/pair-verify` and `/pair-setup` cryptography |
| `srp`, `crypto`, `fairplay`, `plist`, `rtp`, `ntp`, `wire` | the primitives underneath |

## Command line

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

Its files live in the user's local application data directory: `identity.seed` and
`clients`. Deleting the first changes the receiver's identity; deleting the second makes
every client pair again.

## Logging

The library emits `log` records: `info` for progress, `warn` for a session that went wrong,
`debug` for packet-level detail. Install a logger to see them.
