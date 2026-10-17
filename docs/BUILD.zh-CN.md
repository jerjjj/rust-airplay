# 构建指南

English: [BUILD.md](BUILD.md)。

## 依赖

Rust 1.85 或更新版本（edition 2024），以及 Rust 工具链本身需要的 C 链接器：Windows 上用 MSVC，macOS 与 Linux 用系统编译器。

默认工作区不链接任何 C 库。GStreamer 只在需要播放时使用。

## 构建

```sh
cargo build --workspace
cargo build -p rust-airplay-cli --features gst
```

## GStreamer

后端 crate 被排除在 workspace 之外，因此上面的门禁永远不需要系统库。

### 开发库

由 `pkg-config` 查找：

| 模块 | 包 |
|---|---|
| `gstreamer-1.0` | GStreamer 核心开发库 |
| `gstreamer-app-1.0` | GStreamer plugins-base 开发库 |
| `glib-2.0`、`gobject-2.0`、`gio-2.0` | GLib 开发库 |

Windows（MSYS2 UCRT64）：

```powershell
$env:PATH = 'C:\msys64\ucrt64\bin;' + $env:PATH
pkg-config --modversion gstreamer-1.0
cargo build -p rust-airplay-cli --features gst
```

Debian 与 Ubuntu：

```sh
sudo apt-get install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
```

macOS：安装 GStreamer 开发库，并把其路径导出给 `pkg-config`。

### 运行时元素

| 用途 | 元素 |
|---|---|
| 视频 | `h264parse`、`avdec_h264` 或 `d3d11h264dec`、`videoconvert`、一个 sink |
| 音频 | `avdec_aac` 或 `avdec_alac`、`audioconvert`、`audioresample`、一个 sink |

不需要 Bonjour 或 Avahi：发现由 Rust 实现。也不需要单独安装 FFmpeg：解码器来自 GStreamer 的 libav 插件包。

## 运行

```sh
cargo run -p rust-airplay-cli --features gst -- --name "my receiver" --port 7000 --render gst
```

## 环境变量

| 变量 | 作用 |
|---|---|
| `AIRPLAY_GST_LAUNCH` | 指定要运行的 `gst-launch-1.0` |
| `AIRPLAY_GST_AUDIO_SINK` | 音频 sink 名 |
| `AIRPLAY_GST_VIDEO_SINK` | 视频 sink 名 |
| `AIRPLAY_GST_VIDEO_WARM` | 置 `1` 时视频管线随接收端一起启动 |
| `AIRPLAY_DUMP_VIDEO` | 把最早的视频帧以 Annex-B 追加写入该文件 |
| `AIRPLAY_DUMP_AUDIO` | 把音频负载带长度前缀追加写入该文件 |

## 测试

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

另有两组需要工作区之外的依赖：

```sh
pwsh -File tools/oracle/run.ps1
cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --ignored
```

前者用 UxPlay 构建一个 C 程序（说明见 `tools/oracle/README.md`），并把本实现与它对比。它需要 `upstream/UxPlay` 下的提交 `57ea83411d5f7e0b38c5841987439340543f025c` 检出，以及 MSYS2。后者需要 GStreamer 运行时。

## 排错

| 现象 | 原因 |
|---|---|
| 控制端口报 `os error 10013` 或 `10048` | 端口被保留或占用；Windows 会为 Hyper-V 保留端口段。换一个端口，或用 `--port 0` |
| `--render gst` 报缺少特性 | 构建时没加 `--features gst` |
| 客户端找不到接收端 | 检查 mDNS 组播是否被拦截，以及两者是否在同一网络 |
| 重新构建时 `LNK1104: cannot open file ...rust_airplay.exe` | 之前的接收端还在运行；停掉它，或运行二进制的副本 |
| 画面卡住并丢帧 | sink 跟不上；Windows 上可试 `AIRPLAY_GST_VIDEO_SINK=d3d11videosink` |
