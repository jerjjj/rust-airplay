# API

English: [API.md](API.md)。生成的参考文档在 [docs.rs/rust-airplay](https://docs.rs/rust-airplay)。

## 运行接收端

```rust
pub async fn run(config: Config) -> Result<(), Error>
```

这一个 future 做完全部工作：绑定控制服务、注册 mDNS 记录、绑定媒体套接字、监听进程的停止信号，然后服务客户端。每条控制连接有自己的处理器集，但媒体会话是共享的。客户端的音频密钥在某次 `SETUP` 里到达，镜像流的标识在之后另一次 `SETUP` 里到达，而这两次未必是同一条连接。

进程被要求停止时返回 `Ok(())`；只有"接收端无法启动"才返回 `Err`：

- 控制端口绑不上；
- mDNS 记录注册不出去；
- 某个媒体套接字打不开。

接收端已经运行之后某个会话出问题，会通过 `log` 记录，接收端继续服务。丢弃这个 future 与停止信号等价。

## `Config`

接收端需要的一切都在这里；凡是调用方能决定的，都做成字段，而不是藏起来的默认值。库不读环境变量，也不打开文件。

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `name` | `String` | 客户端菜单里显示的名字 |
| `port` | `u16` | 控制端口；`0` 绑定临时端口并广播该端口 |
| `seed` | `[u8; 32]` | 身份派生所用的秘密。传入保存的种子，接收端跨重启仍是同一个 |
| `device_id` | `Option<String>` | 要广播的硬件地址；`None` 时由种子派生 |
| `pin` | `PinMode` | `Disabled`、`OneTime` 或 `Fixed(digits)`。PIN 通过 `log` 输出，想用别的方式显示的程序可以自己装 logger |
| `password` | `Option<String>` | 设置后，`SETUP` 还需要这个 digest 口令 |
| `audio_codec` | `AudioCodec` | `Alac` 或 `AacEld`。客户端不会声明，所以由调用方给出 |
| `renderer` | `Arc<dyn Renderer>` | 播放媒体的实现 |
| `clients` | `Arc<Mutex<HashSet<[u8; 32]>>>` | 已验证的客户端密钥。启动时读取，验证通过后就加进去 |
| `on_client` | `Option<ClientCallback>` | 客户端首次通过验证时调用，便于调用方持久化该密钥 |
| `trace_log` | `Option<PathBuf>` | 设置后把请求轨迹追加到该文件 |
| `dump_video` | `Option<PathBuf>` | 设置后把最早的视频帧以 Annex-B 追加到该文件 |
| `dump_audio` | `Option<PathBuf>` | 设置后把音频负载带长度前缀追加到该文件 |
| `pace_video` | `bool` | 是否按采集间隔节流镜像视频。默认关闭：投屏看的是画面跟手的快慢 |

`ClientCallback` 的类型是 `Arc<dyn Fn(&[u8; 32]) + Send + Sync>`。它在验证客户端的连接上执行，因此写文件的实现应当让工作量足够小，或者干脆交给另一个线程。

## 渲染器 trait

```rust
pub trait Renderer: Debug + Send + Sync {
    fn open_audio(&self, config: AudioConfig) -> Result<Box<dyn AudioStream>, RenderError>;
    fn open_video(&self, config: VideoConfig) -> Result<Box<dyn VideoStream>, RenderError>;
    fn set_volume(&self, volume: Volume) -> Result<(), RenderError>;
}
```

每个会话打开一次流，两者都是 `'static`，因为会话会把每一个移到只负责写帧的线程上。这是对实现者影响最大的设计点：写入可以阻塞任意长时间，接收循环 meanwhile 继续读套接字。也正因为如此，每个流前面都有一个队列；一个需要阻塞的后端在这里不是问题。

`AudioStream` 接收 `AudioFrame`，另有 `set_volume`、`flush`、`close`。`VideoStream` 接收 `VideoFrame`，另有 `set_pace`、`flush`、`close`。

- `push` 可以阻塞，它由专用线程调用。
- `close` 表示本会话没有更多媒体了，不要求销毁流。
- 丢弃流必须释放后端持有的资源。

`NullRenderer` 用"全部接受、什么都不播"实现这个 trait，并统计收到了什么，因此既适合检查会话，也适合写测试。

## 媒体类型

| 类型 | 内容 |
| --- | --- |
| `AudioConfig` | 编码、采样率、声道数、每帧采样数 |
| `AudioFrame` | 编码后的负载，以及以会话采样数计的时间戳 |
| `VideoConfig` | 客户端声明过的宽高 |
| `VideoFrame` | Annex-B 访问单元、时间戳、是否为关键帧 |
| `Volume` | 从静音到满音量，构造时夹紧 |
| `RenderError` | 无法播放用 `Unavailable`，播放出错用 `Failed` |

音频时间戳的单位是采样数而非纳秒，因为客户端就是这么发的。按时间排程的渲染器需要换算；GStreamer 后端已经这样做。

## 模块

运行接收端只需要 `receiver`；其余模块公开，是因为单个部件本身有用，例如测试里想只验证解密而不启动服务器。

| 模块 | 内容 |
| --- | --- |
| `receiver` | `run`、`Config`、`Error`、`ClientCallback` |
| `render` | 上文那些渲染 trait 与媒体类型 |
| `discovery` | `Advertisement`、`Advertiser`、`Protection` 与文本记录 |
| `server` | `Server`、`RouteHandlers`、`DeviceIdentity`、`PairingState`、`PinMode`、`SetupPorts` |
| `control` | RTSP 请求解析与路由表 |
| `media` | 音频重排、镜像解密、数据报套接字、时钟交换 |
| `pairing` | `/pair-verify` 与 `/pair-setup` 的密码学 |
| `srp`、`crypto`、`fairplay`、`plist`、`rtp`、`ntp`、`wire` | 上述模块赖以构建的原语 |

## 命令行接收端

`rust-airplay-cli` 生成名为 `rust-airplay` 的可执行文件。参数：

| 参数 | 含义 |
| --- | --- |
| `--name <name>` | 客户端显示的名字，默认 `rust-airplay` |
| `--port <port>` | 控制端口，默认 7000；`0` 表示自动挑一个 |
| `--pin [<digits>]` | 要求 PIN，可指定固定值；这是默认行为 |
| `--no-pin` | 不要求 PIN |
| `--password <password>` | `SETUP` 时额外要求 digest 口令 |
| `--device-id <address>` | 广播指定的硬件地址 |
| `--seed <hex>` | 32 字节身份，写成 64 个十六进制字符 |
| `--log <file>` | 把请求轨迹追加到文件 |
| `--render <null\|gst>` | 由什么播放媒体，默认 `null` |
| `--audio-sink <name>` | GStreamer 音频 sink，覆盖环境变量 |
| `--video-sink <name>` | GStreamer 视频 sink，覆盖环境变量 |
| `--video-parser <name>` | 镜像流的 GStreamer parser |
| `--video-decoder <name>` | 镜像流的 GStreamer 解码器 |
| `--warm-video` | 视频管线随接收端一起启动 |
| `--pace-video` | 按采集间隔节流视频 |
| `--audio-codec <alac\|aac-eld>` | 客户端发送的编码，默认 `aac-eld` |
| `--help` | 打印用法并退出 |

程序在本地应用数据目录下保存两个文件：`rust-airplay/identity.seed` 与 `rust-airplay/clients`。前者首次运行时创建，是接收端身份保持稳定的原因；后者是客户端密钥列表，每行一个 64 字符的十六进制密钥。删除身份文件会让所有客户端重新配对；删除客户端列表则保留身份、清空配对记录。

## 日志

库通过 `log` 输出记录。`info` 记录旁观会话的人想看到的进度，`warn` 记录出问题的会话，`debug` 记录包级细节。没有安装 logger 就什么也不打印。命令行接收端安装的 logger 把 `info` 及以下写到标准输出，把警告写到标准错误。
