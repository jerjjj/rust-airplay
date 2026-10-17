# rust-airplay

用 Rust 写的 AirPlay 1/2 接收端，支持 Windows、macOS 与 Linux。

接收端接受来自 iPhone、iPad 或 Mac 的屏幕镜像会话与音频流，解密后通过你选择的渲染器播放。这个 crate 就是接收端本身：协议、媒体会话，以及渲染器接入的边界。它是 [UxPlay](https://github.com/FDH2/UxPlay) 的移植，线上行为与之一致。

## 概览

- **应答 AirPlay**：通过 mDNS 广播 `_raop._tcp` 与 `_airplay._tcp`，在 TCP 上提供控制协议：`/info`、`/fp-setup`、`/pair-setup`、`/pair-verify`、`/feedback`、`SETUP`、`RECORD`、`SET_PARAMETER`、`TEARDOWN`。
- **与客户端配对**：一次性或固定 PIN 走 SRP-6a，已持有密钥的客户端走 Curve25519。通过验证的密钥会被记住，配对过的客户端无需再次输入 PIN。
- **处理媒体**：音频解密后进入有界重排窗口，缺包时请求重传。镜像按"读自 UxPlay 自身代码"的密钥流位置规则解密，并重组为帧。
- **渲染交给外部**：帧跨过一组很小的 trait 边界。仓库提供 GStreamer 后端；`NullRenderer` 全部接受但不播放，用于在没有声卡的机器上验证会话。
- **不含策略**：库不读环境变量、不选路径、不打印。身份种子、客户端名单与日志器由运行它的程序提供。

## 状态

| 范围 | 验证环境 |
|---|---|
| 屏幕镜像与音频播放 | Windows 11，真实 iPad，GStreamer 1.28 |
| 控制面、配对、发现 | Windows 11，真实 iPad |
| 其余 | 测试套件，以及针对 UxPlay C 代码的差分测试 |

macOS 与 Linux 能编译、测试通过，但没有在真实设备上跑过。会话所需的 GStreamer 元素各平台不同，因此这两者按"未验证"对待。

尚未实现：音视频与同一时钟的同步。两条播放路径都按收到帧的速率播放。

## 依赖

Rust 1.85 或更新版本。默认构建不链接任何 C 库。

播放媒体需要 GStreamer，它是可选特性，没有它照样能构建、测试与 lint 整个工作区。各平台说明见 [docs/BUILD.zh-CN.md](docs/BUILD.zh-CN.md)。

## 开始使用

构建库与命令行接收端：

```sh
cargo build --workspace
```

构建 GStreamer 后端（需要 GStreamer 开发库）：

```sh
cargo build -p rust-airplay-cli --features gst
```

运行接收端：

```sh
cargo run -p rust-airplay-cli --features gst -- --name "my receiver" --port 7000 --render gst
```

在 iPhone 或 iPad 上打开 AirPlay 菜单，选择这个接收端，输入程序打印的 PIN。把 `--render gst` 换成 `--render null`，同一个接收端就会运行而不播放任何东西。

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
    name: "my receiver".to_owned(),
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

`run` 持续服务客户端，直到进程被要求停止。要真正播放，实现 `Renderer`，或使用 [`rust-airplay-gstreamer`](crates/rust-airplay-gstreamer)。

## 文档

- [API 参考](https://docs.rs/rust-airplay)（docs.rs）
- [docs/API.zh-CN.md](docs/API.zh-CN.md) — 配置、渲染 trait、命令行
- [docs/BUILD.zh-CN.md](docs/BUILD.zh-CN.md) — 依赖、各平台说明、排错
- [docs/ARCHITECTURE.zh-CN.md](docs/ARCHITECTURE.zh-CN.md) — 各部分如何协作，以及为什么
- [AGENT.md](AGENT.md) — 给自动化贡献者的说明（仅英文）

除 `AGENT.md` 外，每份文档都有英文版本，位于同名 `.md` 文件。

## 开发

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

GStreamer 后端是独立的 crate，有一套同样的门禁，见 [AGENT.md](AGENT.md)。

另有两组针对真实机制的测试，不在默认运行范围内：

```sh
cargo test -p rust-airplay --test upstream_oracle -- --ignored
cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --ignored
```

前者把本实现与 UxPlay 自身的 C 代码对比，需要本地 UxPlay 检出与 C 工具链；后者解码真机会话抓取的帧。

## 最低 Rust 版本

1.85。工作区在 `Cargo.toml` 里固定该版本，旧工具链会得到明确提示，而不是难以理解的报错。

## 许可证

GPL-3.0-or-later。crate 中部分代码译自 UxPlay 的 GPL 与 LGPL 源码，因此许可证是继承而来，不是自选：链接本 crate 的程序同样受其约束。逐文件归属见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 参与贡献

欢迎提交问题与合并请求。[AGENT.md](AGENT.md) 说明了门禁、代码规则，以及哪些内容已验证、哪些没有——请不要声称改动能在一个本项目从未运行过的平台上工作。
