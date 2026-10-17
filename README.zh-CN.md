# rust-airplay

用 Rust 写的 AirPlay 1/2 接收端，支持 Windows、macOS 与 Linux。

它接收来自 iPhone、iPad 或 Mac 的屏幕镜像会话与音频流，解密后通过渲染器播放。仓库包含三部分：作为库的接收端、运行它的命令行程序，以及一个 GStreamer 渲染器。

## 特性

- 通过 mDNS 广播并应答 AirPlay 控制协议，因此在 iPhone、iPad 或 Mac 的 AirPlay 菜单中可见。
- 用 PIN 与客户端配对并记住它们，配对过的客户端再次连接无需输入 PIN。
- 接收屏幕镜像与音频，解密后把帧交给渲染器。
- 通过 GStreamer 播放；只想检查会话时也可以什么都不播。

## 依赖

Rust 1.85 或更新版本。播放媒体需要 GStreamer：

| 构建目标 | 需要 |
|---|---|
| 库，以及 `--render null` | Rust 1.85 |
| `--render gst` | 构建期需要 GStreamer 开发库，运行期需要 GStreamer 运行时 |

各平台说明见 [docs/BUILD.zh-CN.md](docs/BUILD.zh-CN.md)。

## 构建与运行

```sh
cargo build -p rust-airplay-cli --features gst
cargo run -p rust-airplay-cli --features gst -- --name "my receiver" --port 7000 --render gst
```

在 iPhone 或 iPad 上打开 AirPlay 菜单，选择这个接收端，输入程序打印的 PIN。

只想检查会话而不播放时，用 `--render null`：

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

要真正播放，实现 `Renderer`，或使用 [`rust-airplay-gstreamer`](crates/rust-airplay-gstreamer)。

## 文档

- [API 参考](https://docs.rs/rust-airplay)
- [docs/API.zh-CN.md](docs/API.zh-CN.md) - 配置、渲染 trait、命令行
- [docs/BUILD.zh-CN.md](docs/BUILD.zh-CN.md) - 依赖与排错
- [docs/ARCHITECTURE.zh-CN.md](docs/ARCHITECTURE.zh-CN.md) - crate 布局与会话路径
- [AGENT.md](AGENT.md) - 给自动化贡献者的说明

英文版：[README.md](README.md)。除 `AGENT.md` 外，每份文档都有英文版本。

## 许可证

GPL-3.0-or-later。见 [LICENSE](LICENSE) 与 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
