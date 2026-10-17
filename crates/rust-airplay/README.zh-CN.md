# rust-airplay

作为库使用的 AirPlay 1/2 接收端。

`rust-airplay` 通过 mDNS 广播接收端、应答 AirPlay 控制协议、与客户端配对、接收屏幕镜像与音频、解密，并把帧交给渲染器。它不打开文件、不读环境变量、不打印任何东西，因此日志去哪里、身份存在哪里，都由嵌入它的程序决定。

## 引入

```toml
[dependencies]
rust-airplay = "0.1"
```

需要 Rust 1.85 或更新版本。这个 crate 不链接任何 C 库。

## 启动接收端

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

`run` 持续服务客户端，直到进程被要求停止。有两个字段值得解释，因为接收端跨重启的行为就来自它们。`seed` 是身份派生所用的秘密，传入同一个种子，接收端就还是同一个。`clients` 是已验证的客户端密钥集合，`on_client` 在新的密钥加入时被调用，那时正是把它写下来的时机。

## 播放媒体

帧到达 `Renderer`，由调用方实现。`NullRenderer` 全部接受但不播放，适合测试，也适合在没有声卡的机器上检查会话。

GStreamer 实现是独立的 crate：

```toml
[dependencies]
rust-airplay = "0.1"
rust-airplay-gstreamer = "0.1"
```

## 日志

记录通过 `log` 以 `info`、`warn`、`debug` 输出。没有安装 logger 就什么也不打印。客户端需要输入的 PIN 也在 `info` 记录里，因此想把它显示得更醒目的程序应当安装自己的 logger。

## 许可证

GPL-3.0-or-later。

English: [README.md](README.md)。
