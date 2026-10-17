# rust-airplay

用安全 Rust 写的 AirPlay 1/2 接收端。

这个 crate 就是完整的接收端：通过 mDNS 广播服务、应答控制协议、与客户端配对、派生媒体会话所需的密钥、解密并重排收到的数据，然后把帧交给调用方提供的渲染器。它没有命令行、不访问文件系统、不打印任何东西，这些决定权留给嵌入它的程序。

现成的程序是 [`rust-airplay-cli`](https://crates.io/crates/rust-airplay-cli) 包；GStreamer 后端是 [`rust-airplay-gstreamer`](https://crates.io/crates/rust-airplay-gstreamer) 包。

## 状态

已验证的内容与所在环境：

| 范围 | 验证环境 |
|---|---|
| 屏幕镜像、音频播放 | Windows，真实 iPad，MSYS2 UCRT64 GStreamer 1.28 |
| 控制面、配对、发现 | Windows，真实 iPad |
| 其余部分 | 仅测试套件 |

macOS 与 Linux 能编译、测试通过，但没有在真实设备上跑过。文档里应当如实写出这一点，而不是给出更多承诺。

## 引入

```toml
[dependencies]
rust-airplay = "0.1"
```

最低 Rust 版本 1.85。默认构建不链接任何 C 库。

## 用法

接收端需要嵌入方提供四样东西：身份种子、已信任的客户端集合、渲染器，以及日志的去处。

```rust,no_run
use std::sync::{Arc, Mutex};
use std::collections::HashSet;
use rust_airplay::{Config, NullRenderer};
use rust_airplay::server::PinMode;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = Config {
    name: "my receiver".to_owned(),
    port: 7000,
    seed: [0x42; 32],
    device_id: None,
    pin: PinMode::OneTime,
    password: None,
    audio_codec: rust_airplay::AudioCodec::AacEld,
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

[`run`] 在进程被要求停止时返回，只有"无法启动"才报告失败；之后某个会话出问题会通过 `log` 记录，不会让接收端退出。

## 播放媒体

帧交给 [`Renderer`]。[`NullRenderer`] 全部接受但不播放，这是在没有声卡、没有显示器的机器上验证会话的方式。要真正播放，实现 [`Renderer`]，或者使用 GStreamer 后端：

```rust,no_run
# #[cfg(feature = "unused")]
# fn example() {
use rust_airplay::Renderer;
let renderer: std::sync::Arc<dyn Renderer> =
    std::sync::Arc::new(rust_airplay_gstreamer::GstRenderer::new().unwrap());
# }
```

GStreamer 后端构建期需要 GStreamer 开发库，运行期需要 GStreamer 运行时与常用插件，详见它自己的 README。

## 日志

本 crate 通过 `log` 门面输出记录，从不打印。嵌入的程序负责安装 logger；没有安装就什么也看不到。

## 许可证

GPL-3.0-or-later。这不是偏好：crate 中有部分代码译自 UxPlay 的 GPL 与 LGPL 源码，因此链接它的程序同样受该许可证约束。逐文件的来源与归属记录在仓库的 `THIRD_PARTY_NOTICES.md`。

English: [README.md](README.md)。
