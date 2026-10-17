# API

English: [API.md](API.md)。生成的参考文档：[docs.rs/rust-airplay](https://docs.rs/rust-airplay)。

## `run`

```rust
pub async fn run(config: Config) -> Result<(), Error>
```

持续服务客户端，直到进程被要求停止。它会绑定控制服务、广播接收端、为每个客户端建立媒体会话，并把帧交给渲染器。只有接收端无法启动时才返回 `Err`：端口绑不上、服务广播不出去、媒体套接字打不开。

## `Config`

| 字段 | 含义 |
|---|---|
| `name: String` | 客户端看到的名字 |
| `port: u16` | 控制端口；`0` 自动挑一个 |
| `seed: [u8; 32]` | 接收端密钥的派生来源 |
| `device_id: Option<String>` | 广播的硬件地址；`None` 时由种子派生 |
| `pin: PinMode` | `Disabled`、`OneTime` 或 `Fixed(digits)` |
| `password: Option<String>` | `SETUP` 需要的 digest 口令 |
| `audio_codec: AudioCodec` | `Alac` 或 `AacEld` |
| `renderer: Arc<dyn Renderer>` | 播放媒体的实现 |
| `clients: Arc<Mutex<HashSet<[u8; 32]>>>` | 已验证客户端的密钥 |
| `on_client: Option<ClientCallback>` | 客户端首次通过验证时调用 |
| `trace_log: Option<PathBuf>` | 把请求轨迹追加到该文件 |
| `dump_video: Option<PathBuf>` | 把最早的视频帧以 Annex-B 追加到该文件 |
| `dump_audio: Option<PathBuf>` | 把音频负载带长度前缀追加到该文件 |

## Renderer

```rust
pub trait Renderer: Debug + Send + Sync {
    fn open_audio(&self, config: AudioConfig) -> Result<Box<dyn AudioStream>, RenderError>;
    fn open_video(&self, config: VideoConfig) -> Result<Box<dyn VideoStream>, RenderError>;
    fn set_volume(&self, volume: Volume) -> Result<(), RenderError>;
}
```

`AudioStream` 接收 `AudioFrame`，另有 `set_volume`、`flush`、`close`。`VideoStream` 接收 `VideoFrame`，另有 `set_pace`、`flush`、`close`。

- 后端吃不下更多时 `push` 会阻塞；接收循环不会调用它。
- `close` 表示本会话没有更多媒体了。
- 丢弃流才会释放它持有的资源。

`NullRenderer` 全部接受但不播放。

## 媒体类型

| 类型 | 内容 |
|---|---|
| `AudioConfig` | 编码、采样率、声道数、每帧采样数 |
| `AudioFrame` | 编码后负载，以及以采样数计的时间戳 |
| `VideoConfig` | 客户端声明过的尺寸 |
| `VideoFrame` | Annex-B 访问单元、时间戳、是否为关键帧 |
| `Volume` | 从静音到满音量，越界会被夹紧 |
| `RenderError` | `Unavailable` 或 `Failed` |

## 模块

运行接收端只需要 `receiver`；其余模块公开是为了让程序单独使用协议的一部分。

| 模块 | 内容 |
|---|---|
| `receiver` | `run`、`Config`、`Error`、`ClientCallback` |
| `render` | 渲染 trait 与媒体类型 |
| `discovery` | `Advertisement`、`Advertiser`、`Protection` |
| `server` | `Server`、`RouteHandlers`、`DeviceIdentity`、`PairingState`、`PinMode` |
| `media` | 音频重排、镜像解密、数据报套接字、时钟交换 |
| `control` | RTSP 请求解析与路由表 |
| `pairing` | `/pair-verify` 与 `/pair-setup` 的密码学 |
| `srp`、`crypto`、`fairplay`、`plist`、`rtp`、`ntp`、`wire` | 更底层的原语 |

## 命令行

`rust-airplay-cli` 生成名为 `rust-airplay` 的可执行文件。

| 参数 | 含义 |
|---|---|
| `--name <name>` | 客户端显示的名字 |
| `--port <port>` | 控制端口；`0` 自动挑一个 |
| `--pin [<digits>]` | 要求 PIN，可固定 |
| `--no-pin` | 不要求 PIN |
| `--password <password>` | 额外要求 `SETUP` digest 口令 |
| `--device-id <address>` | 广播的硬件地址 |
| `--seed <hex>` | 32 字节身份种子 |
| `--log <file>` | 追加请求轨迹 |
| `--render <null\|gst>` | 由什么播放媒体 |
| `--audio-codec <alac\|aac-eld>` | 客户端发送的编码 |
| `--help` | 用法说明 |

它的文件位于用户的本地应用数据目录：`identity.seed` 与 `clients`。删除前者等于换一个接收端身份；删除后者会让所有客户端重新配对。

## 日志

库通过 `log` 输出记录：`info` 为进度，`warn` 为会话出错，`debug` 为包级细节。安装 logger 才能看到。
