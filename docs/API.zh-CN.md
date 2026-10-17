# API

English: [API.md](API.md)。生成的参考文档：[docs.rs/rust-airplay](https://docs.rs/rust-airplay)。

## 接收端

```rust
pub async fn run(config: Config) -> Result<(), Error>
```

持续服务客户端，直到进程被要求停止。它会绑定控制服务、广播接收端、为每个客户端建立媒体会话，并把帧交给配置好的渲染器。进程被要求停止时返回 `Ok(())`；只有"接收端无法启动"才返回 `Err`：端口绑不上、服务广播不出去、媒体套接字打不开。之后某个会话出问题会通过 `log` 记录，不会让接收端退出。

丢弃这个 future 即取消；接收循环与渲染器管线随之释放。

## `Config`

| 字段 | 含义 |
|---|---|
| `name: String` | 客户端在 AirPlay 菜单里看到的名字 |
| `port: u16` | 控制端口；`0` 表示自动挑一个空闲端口 |
| `seed: [u8; 32]` | 所有身份密钥的派生来源；保住它就保住身份 |
| `device_id: Option<String>` | 广播的硬件地址；`None` 时由种子派生 |
| `pin: PinMode` | `Disabled`、`OneTime` 或 `Fixed(digits)` |
| `password: Option<String>` | 设置后，`SETUP` 需要 digest 口令 |
| `audio_codec: AudioCodec` | `Alac` 或 `AacEld`，即客户端帧的编码 |
| `renderer: Arc<dyn Renderer>` | 播放媒体的实现 |
| `clients: Arc<Mutex<HashSet<[u8; 32]>>>` | 已验证客户端的密钥 |
| `on_client: Option<ClientCallback>` | 客户端首次通过验证时调用 |
| `trace_log: Option<PathBuf>` | 把请求轨迹追加到该文件 |
| `dump_video: Option<PathBuf>` | 把最早的视频帧以 Annex-B 追加到该文件 |
| `dump_audio: Option<PathBuf>` | 把音频负载带长度前缀追加到该文件 |

`Config` 不含任何路径，也不读取环境变量。持久化、默认值与打印属于嵌入方。

## 错误

`Error` 携带一条消息，实现了 `Display` 与 `std::error::Error`。它没有做成枚举，因为它报告的都是"这个接收端无法启动"，需要区分处理的调用方应当先检查输入。

## 渲染边界

```rust
pub trait Renderer: Debug + Send + Sync {
    fn open_audio(&self, config: AudioConfig) -> Result<Box<dyn AudioStream>, RenderError>;
    fn open_video(&self, config: VideoConfig) -> Result<Box<dyn VideoStream>, RenderError>;
    fn set_volume(&self, volume: Volume) -> Result<(), RenderError>;
}
```

`AudioStream` 接收 `AudioFrame`，另有 `set_volume`、`flush`、`close`。`VideoStream` 接收 `VideoFrame`，另有 `set_pace`、`flush`、`close`。流的类型是 `'static + Send + Sync`，因为会话会在自己的线程上播放它们：会阻塞接收循环的流会让整个会话停住。

实现时需要注意的约定：

- 后端吃不下更多时，`push` **阻塞**。这个反压是故意的；接收循环不会调用它。
- `close` 表示"没有更多媒体了"，不要假设它会销毁流。
- 丢弃流才会释放它持有的资源。

`NullRenderer` 全部接受但不播放，是该 trait 的参考实现，也是在无声卡机器上跑完整会话的方式。

## 媒体类型

| 类型 | 内容 |
|---|---|
| `AudioConfig` | 编码、采样率、声道数、每帧采样数 |
| `AudioFrame` | 编码后负载，以及客户端自己时钟下的时间戳 |
| `VideoConfig` | 客户端声明过的尺寸 |
| `VideoFrame` | Annex-B 访问单元、客户端时间戳、是否为关键帧 |
| `Volume` | 从静音到满音量，越界会被夹紧 |
| `RenderError` | `Unavailable`（无法播放）或 `Failed`（播放出错） |

`AudioFrame` 的时间戳单位是**会话采样率下的采样数**，不是纳秒。按时间排程的渲染器需要换算；GStreamer 后端已经这样做。

## 模块

底层模块是公开的，因为程序可能只想用协议的一部分而不跑整个接收端。运行接收端只需 `receiver`。

| 模块 | 内容 |
|---|---|
| `receiver` | `run`、`Config`、`Error`、`ClientCallback` |
| `render` | 上文那些 trait 与媒体类型 |
| `discovery` | `Advertisement`、`Advertiser`、`Protection`、文本记录 |
| `server` | `Server`、`RouteHandlers`、`DeviceIdentity`、`PairingState`、`PinMode`、`SetupPorts` |
| `control` | RTSP 请求解析与路由表 |
| `media` | 音频重排、镜像解密、数据报套接字、时钟交换 |
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

库通过 `log` 输出记录：`info` 用于进度，`warn` 用于会话出错，`debug` 用于包级细节。需要安装 logger 才能看到；没有安装则静默运行。CLI 装了一个打印到标准输出的。
