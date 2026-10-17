# rust-airplay-gstreamer

[`rust-airplay`](https://crates.io/crates/rust-airplay) 的 GStreamer 渲染器。

它实现 `Renderer` trait：音频在进程内交给 GStreamer 管线，视频写入 `gst-launch-1.0` 子进程。

## 依赖

构建期：GStreamer 开发库，由 `pkg-config` 查找（`gstreamer-1.0`、`gstreamer-app-1.0`）。

运行期：GStreamer 运行时，且下列元素可用。

| 用途 | 元素 |
|---|---|
| 视频 | `h264parse`、`avdec_h264` 或 `d3d11h264dec`、`videoconvert`、一个 sink |
| 音频 | `avdec_aac` 或 `avdec_alac`、`audioconvert`、`audioresample`、一个 sink |

Windows 上 MSYS2 UCRT64 两者都提供，把 `C:\msys64\ucrt64\bin` 加进 `PATH` 即可。

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

环境变量：

| 变量 | 作用 |
|---|---|
| `AIRPLAY_GST_LAUNCH` | 指定要运行的 `gst-launch-1.0` |
| `AIRPLAY_GST_AUDIO_SINK` | 音频 sink 名，Windows 上为 `wasapisink` |
| `AIRPLAY_GST_VIDEO_SINK` | 视频 sink 名，默认 `autovideosink` |
| `AIRPLAY_GST_VIDEO_WARM` | 置 `1` 时视频管线随接收端一起启动 |
| `AIRPLAY_GST_KEEP_STDERR` | 保留子进程的错误输出 |

## 测试

```sh
cargo test
cargo test -- --ignored
```

被忽略的测试需要 GStreamer 运行时，它们会解码会话抓取的帧。

## 许可证

GPL-3.0-or-later。

English: [README.md](README.md)。
