# rust-airplay

用安全 Rust 写的 AirPlay 1/2 接收端，支持 Windows、macOS 与 Linux。

`rust-airplay` 接收来自 iPhone、iPad 或 Mac 的屏幕镜像会话与音频流，解密后交给你选择的渲染器播放。它是 [UxPlay](https://github.com/FDH2/UxPlay) 的移植：线上行为保持一致，但多了一层 UxPlay 没有的拆分——协议与会话做成库，命令行只是库之上的一层薄程序。

English: [README.md](README.md)。

## 能做什么

- 通过 mDNS 把自己广播成 AirPlay 接收端，广播内容与当前客户端期望的一致，因此在 AirPlay 菜单里无需额外配置即可出现。
- 应答控制协议：`/info`、`/fp-setup`、`/pair-setup`、`/pair-verify`、`/feedback`、`SETUP`、`RECORD`、`SET_PARAMETER` 与 `TEARDOWN`。
- 用一次性或固定 PIN 与客户端配对，并记住已验证客户端的密钥。
- 建立音频、控制、时序与镜像四条流，派生会话所需的密钥，并解密收到的数据。
- 把 H.264 镜像包重组为帧；参数集被单独发送时，按上游规则拼到相应帧之前。
- 重排音频、在缺包时请求重传、应答时钟交换。
- 把帧交给 `Renderer`。内置 GStreamer 后端；另有 `NullRenderer` 全部接受但不播放，用于在没有声卡和显示器的机器上验证会话。

## 不能做什么

- **不编码**：接收端只解密与播放，编码在客户端。
- **不录屏**：AirPlay 镜像是网络协议，不是屏幕采集接口。
- **不自行存储**：身份种子与客户端名单由调用方持久化，这样库本身不碰文件系统。
- **尚未做音画同步**：两条播放路径各自按收到帧的速率播放，A/V 同步是下一步工作。

## 已验证环境

| 范围 | 验证环境 |
|---|---|
| 屏幕镜像与音频播放 | Windows 11，真实 iPad，MSYS2 UCRT64 GStreamer 1.28 |
| 控制面、配对、发现 | Windows 11，真实 iPad |
| 其余 | 测试套件，以及与 UxPlay 自身 C 代码的差分测试 |

macOS 与 Linux 能编译、测试通过，但**没有在真实设备上跑过**；GStreamer 的元素名与 sink 各平台不同，因此这两个平台按"未验证"对待。

## 依赖

构建本工作区需要 Rust 1.85 或更新版本。默认构建不链接任何 C 库，此外不需要任何东西。

播放媒体需要 GStreamer，它是可选特性：

| 构建目标 | 需要 |
|---|---|
| `rust-airplay`（库，默认） | Rust 1.85 |
| `rust-airplay-cli --render null` | Rust 1.85 |
| `rust-airplay-cli --render gst` | 构建期：GStreamer 开发库；运行期：GStreamer 运行时与插件 |
| `rust-airplay-gstreamer` | 同上 |

各平台细节见 [docs/BUILD.zh-CN.md](docs/BUILD.zh-CN.md)。

## 构建

```sh
cargo build --workspace
cargo build -p rust-airplay-cli --features gst
```

## 运行

```sh
cargo run -p rust-airplay-cli -- --name "my receiver" --port 7000 --render gst
```

然后在 iPhone 或 iPad 上打开 AirPlay 菜单，选择这个接收端，输入程序打印的 PIN。`--render null` 会在不播放任何东西的情况下运行同一个接收端，这是在没有声卡的机器上验证协议的方式。

全部参数见 `--help`，以及 [docs/API.zh-CN.md](docs/API.zh-CN.md) 的命令行一节。

## 作为库使用

```toml
[dependencies]
rust-airplay = "0.1"
```

```rust,no_run
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use rust_airplay::{AudioCodec, Config, NullRenderer};
use rust_airplay::server::PinMode;

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

完整接口见 [docs/API.zh-CN.md](docs/API.zh-CN.md)，生成的文档在 [docs.rs](https://docs.rs/rust-airplay)。

## 文档

- [docs/BUILD.zh-CN.md](docs/BUILD.zh-CN.md) - 依赖、各平台说明、排错
- [docs/API.zh-CN.md](docs/API.zh-CN.md) - 库 API 与命令行
- [docs/ARCHITECTURE.zh-CN.md](docs/ARCHITECTURE.zh-CN.md) - 各部分如何协作，以及为什么这样拆
- [AGENT.md](AGENT.md) - 给自动化贡献者的说明

除 `AGENT.md` 外，每份文档都有中文与英文两个版本。

## 测试

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

另有两组不进默认门禁的测试：

- `cargo test -p rust-airplay --test upstream_oracle -- --ignored`：与 UxPlay 自身 C 代码逐字节对比，需要本地 UxPlay 检出与 C 工具链，见 [docs/BUILD.zh-CN.md](docs/BUILD.zh-CN.md)。
- `cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --ignored`：驱动真实 GStreamer 运行时，其中包含解码真机抓取的帧。

## 许可证

GPL-3.0-or-later。crate 中部分代码译自 UxPlay 的 GPL 与 LGPL 源码，因此许可证是继承而来，不是自选。逐文件的归属见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
