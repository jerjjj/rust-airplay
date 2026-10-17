# rust-airplay

作为库使用的 AirPlay 1/2 接收端。

它通过 mDNS 广播接收端、应答 AirPlay 控制协议、与客户端配对、接收屏幕镜像与音频、解密，并把帧交给渲染器。它没有命令行、不访问文件系统、不打印任何东西，这些决定权留给嵌入它的程序。

## 用法

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

`run` 持续服务客户端，直到进程被要求停止。

## 播放媒体

帧交给 `Renderer`。`NullRenderer` 全部接受但不播放，适合用于检查会话。要真正播放，实现 `Renderer`，或使用 [`rust-airplay-gstreamer`](https://crates.io/crates/rust-airplay-gstreamer)。

## 依赖

Rust 1.85 或更新版本，不链接任何 C 库。

## 文档

- [API 参考](https://docs.rs/rust-airplay)
- [README](https://github.com/jerjjj/rust-airplay) - 命令行接收端与构建指南

## 许可证

GPL-3.0-or-later。

English: [README.md](README.md)。
