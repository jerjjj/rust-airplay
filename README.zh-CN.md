# rust-airplay

rust-airplay 把一台普通电脑变成 AirPlay 接收端。iPhone、iPad 或 Mac 会在 AirPlay 菜单里找到它，把屏幕镜像或音频推过来，接收端解密后把帧交给渲染器。音频与屏幕镜像都已可用：它在 TCP 上提供 AirPlay 控制协议，在 UDP 与另一条 TCP 连接上收媒体，并通过 mDNS 广播自己。

仓库包含三个 crate。`rust-airplay` 是作为库的接收端，`rust-airplay-cli` 是运行它的命令行程序，`rust-airplay-gstreamer` 用 GStreamer 实现渲染。库不依赖任何 C 库；只想检查协议的程序，可以用一个什么都不播的渲染器把整套流程跑起来。

## 特性

- 接收 iOS 与 macOS 客户端的屏幕镜像与音频。
- 用 PIN 配对，并在重启后保留已配对的客户端。
- 通过 mDNS 广播 `_raop._tcp` 与 `_airplay._tcp`，并提供 `/info`、`/pair-verify`、`/pair-setup`、`/fp-setup`、`SETUP`、`RECORD`、`SET_PARAMETER`、`/feedback` 与 `TEARDOWN`。
- 音频重排；丢包时向客户端请求重传。
- 一个很小的渲染器 trait，外加 GStreamer 实现与空实现。

## 依赖

Rust 1.85 或更新版本。

要真正播放，还需要 GStreamer。只构建库，或者用 `--render null` 运行命令行接收端，都不需要它；只有 `gst` 特性会引入 GStreamer。

| 构建目标 | 需要 |
| --- | --- |
| 库 | Rust 1.85 |
| 命令行接收端，`--render null` | Rust 1.85 |
| 命令行接收端，`--render gst` | 构建期需要 GStreamer 开发库，运行期需要 GStreamer 运行时与插件 |

各平台的软件包见 `docs/BUILD.zh-CN.md`。

## 开始使用

构建带 GStreamer 后端的接收端：

```sh
cargo build -p rust-airplay-cli --features gst
```

启动它。名字就是客户端菜单里显示的名称；不写 `--port` 时使用 7000。

```sh
cargo run -p rust-airplay-cli --features gst -- --name "Living room" --port 7000 --render gst
```

程序会打印它绑定的端口、派生出的身份，以及一个 PIN。在 iPhone 或 iPad 上打开 AirPlay 菜单，选中这个接收端，输入 PIN。客户端一连上就开始镜像；客户端开始播放时就有声音。

要在没有声卡、没有显示器的机器上检查一次会话，用 `--render null` 运行同一个接收端。它照样解密并统计收到的一切，只是不播放，用来确认客户端是否连上、密钥是否派生、帧是否到达。

```sh
cargo run -p rust-airplay-cli -- --port 7000 --render null
```

全部参数见 `--help`。

## 作为库使用

```toml
[dependencies]
rust-airplay = "0.1"
```

```rust,no_run
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use rust_airplay::server::PinMode;
use rust_airplay::{AudioCodec, Config, NullRenderer};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = Config {
    name: "Living room".to_owned(),
    port: 7000,
    seed: [0x42; 32],
    device_id: None,
    pin: PinMode::OneTime,
    password: None,
    audio_codec: AudioCodec::AacEld,
    renderer: Arc::new(NullRenderer::new()),
    clients: Arc::new(Mutex::new(HashSet::new())),
    on_client: None,
    trace_log: None,
    dump_video: None,
    dump_audio: None,
};

rust_airplay::run(config).await?;
# Ok(())
# }
```

`run` 持续服务客户端，直到进程被要求停止；只有接收端根本无法启动时才返回错误。进度与问题通过 `log` 输出，因此消息去哪里由嵌入它的程序决定。存储同样是调用方的事：`seed` 决定接收端跨重启是否还是同一个身份，`clients` 决定配对过的客户端能否跳过 PIN，两者都由调用方传入并持久化。

要真正播放，实现 `Renderer`，或使用 GStreamer 后端：

```toml
[dependencies]
rust-airplay = "0.1"
rust-airplay-gstreamer = "0.1"
```

## 使用要点

第一次运行前，有几件事值得知道。

**端口。** 控制服务用一个端口，默认 7000。Windows 有时会把一段端口预留给 Hyper-V 或 WSL，此时绑定失败并报 `os error 10013`；`netsh int ipv4 show excludedportrange protocol=tcp` 可以列出被保留的区间。用 `--port 0` 会挑一个空闲端口，但广播记录里带的就是这个临时端口，每次启动都会变。

**文件。** 命令行接收端在本地应用数据目录下保存两个文件：`rust-airplay/identity.seed` 与 `rust-airplay/clients`。删除前者等于换一个身份，所有客户端都要重新配对；删除后者保留身份、清空已配对记录。库本身不写这两个文件。

**GStreamer。** 后端的视频是一个 `gst-launch-1.0` 子进程，音频在进程内驱动。如果接收端报告某个流打不开，消息里会指出缺少哪个 GStreamer 元素。`AIRPLAY_GST_VIDEO_SINK` 可以换视频 sink；Windows 上 `d3d11videosink` 比默认的 `autovideosink` 启动更快。`AIRPLAY_GST_VIDEO_WARM=1` 会在启动时就建好视频管线，第一个客户端因此不必等窗口。

**音频编码。** `SETUP` 不会说明音频流用的是哪种编码，所以要用 `--audio-codec` 告诉接收端。当前 iOS 客户端发的是 `aac-eld`，AirPlay 1 客户端发的是 `alac`。

## 文档

- [API 参考](https://docs.rs/rust-airplay)，由源码生成
- [docs/API.zh-CN.md](docs/API.zh-CN.md) 说明配置、渲染器 trait 与命令行
- [docs/BUILD.zh-CN.md](docs/BUILD.zh-CN.md) 说明依赖、各平台的 GStreamer 与排错
- [docs/ARCHITECTURE.zh-CN.md](docs/ARCHITECTURE.zh-CN.md) 说明 crate 划分与会话路径
- [AGENT.md](AGENT.md) 给自动化贡献者

每份文档旁都有英文版本，无后缀的那份；中文版以 `.zh-CN.md` 结尾，起点是本文。

## 许可证

GPL-3.0-or-later。全文见 [LICENSE](LICENSE)，各译文所带的上游声明记录在 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
