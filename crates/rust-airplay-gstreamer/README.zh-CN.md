# rust-airplay-gstreamer

[`rust-airplay`](https://crates.io/crates/rust-airplay) 的 GStreamer 渲染器。

这个 crate 分两半实现 `Renderer` trait。视频送到 `gst-launch-1.0` 子进程的标准输入，因为 H.264 带起始码，字节流本身就保持了帧边界。音频在进程内通过 `appsrc` 驱动，一帧一 buffer，因为 raw AAC-ELD 帧既没有同步字也没有采样率，字节流里没有任何东西能说明一帧到哪里结束。

## 依赖

构建需要 GStreamer 开发库，由 `pkg-config` 查找：

- `gstreamer-1.0`
- `gstreamer-app-1.0`

运行需要 GStreamer 运行时，且下列元素可用：

| 用途 | 元素 |
| --- | --- |
| 视频 | `h264parse`、`avdec_h264` 或 `d3d11h264dec`、`videoconvert`、一个 sink |
| 音频 | `avdec_aac` 或 `avdec_alac`、`audioconvert`、`audioresample`、一个 sink |

Windows 上 MSYS2 UCRT64 两者都提供，把 `C:\msys64\ucrt64\bin` 加进 `PATH`。Debian 与 Ubuntu 上，构建用 `libgstreamer1.0-dev` 与 `libgstreamer-plugins-base1.0-dev` 就够，运行还需要提供上述元素的插件包。

## 用法

```rust,no_run
use std::sync::Arc;
use rust_airplay::Renderer;

# fn example() {
let renderer: Arc<dyn Renderer> = Arc::new(
    rust_airplay_gstreamer::GstRenderer::new().expect("a GStreamer runtime"),
);
# }
```

渲染器通常构建一次并共享，因为启用预热时它也持有那条视频管线。

## 环境变量

| 变量 | 作用 |
| --- | --- |
| `AIRPLAY_GST_LAUNCH` | 指定要运行的 `gst-launch-1.0` |
| `AIRPLAY_GST_AUDIO_SINK` | 音频 sink，Windows 上为 `wasapisink` |
| `AIRPLAY_GST_VIDEO_SINK` | 视频 sink，默认 `autovideosink` |
| `AIRPLAY_GST_VIDEO_WARM` | 置 `1` 时视频管线随接收端一起启动 |
| `AIRPLAY_GST_KEEP_STDERR` | 保留子进程错误输出，不重定向到文件 |

窗口类 sink 在拿到第一帧之前要约一秒才能起来，第一个客户端就在等这一秒。`AIRPLAY_GST_VIDEO_WARM` 把这个代价挪到启动时，之后管线在会话之间复用。Windows 上 `d3d11videosink` 比默认 sink 启动更快，而且可以直接接收 `d3d11h264dec` 产出的帧，不需要拷贝。

## 测试

```sh
cargo test
cargo test -- --ignored
```

被忽略的测试需要 GStreamer 运行时，它们解码真机会话抓取的帧。这正是它们有用的原因：一条管线可以接受每一个 buffer 却一帧都没解出来，只有解码输出能回答这个问题。
