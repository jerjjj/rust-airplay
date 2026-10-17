# 构建指南

English: [BUILD.md](BUILD.md)。

## 工具链

工作区需要 Rust 1.85 或更新版本，因为使用了 edition 2024。版本固定在 `Cargo.toml` 里，太旧的工具链会直接说明它需要哪个版本，而不是在某个宏内部报一个看不懂的错。

C 链接器是 Rust 本身需要的，不是这份代码需要的。Windows 上是 MSVC 构建工具，`rustup` 装默认工具链时一并装好；macOS 与 Linux 用系统编译器就够。

## 构建工作区

```sh
cargo build --workspace
```

这会构建库与命令行接收端，且不链接任何 C 库。`cargo tree --workspace` 里没有任何 `-sys` crate，这个性质值得保持：没有 GStreamer、没有声卡、没有显示器的机器照样能构建、测试与 lint 整个工作区。

## 构建 GStreamer 后端

```sh
cargo build -p rust-airplay-cli --features gst
```

后端是独立的 crate，被排除在 workspace 之外，因此默认门禁不需要系统库。它需要 GStreamer 开发库，由 `pkg-config` 查找：

| 模块 | 包 |
| --- | --- |
| `gstreamer-1.0` | GStreamer 核心开发库 |
| `gstreamer-app-1.0` | GStreamer plugins-base 开发库 |
| `glib-2.0`、`gobject-2.0`、`gio-2.0` | GLib 开发库 |

Windows 上 MSYS2 UCRT64 全都提供。把它的 `bin` 目录加进 `PATH`，让 `pkg-config.exe` 与 GStreamer 工具能被找到：

```powershell
$env:PATH = 'C:\msys64\ucrt64\bin;' + $env:PATH
pkg-config --modversion gstreamer-1.0
cargo build -p rust-airplay-cli --features gst
```

Debian 与 Ubuntu：

```sh
sudo apt-get install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
```

macOS 上，用官方安装器或 Homebrew 安装 GStreamer 开发库，并把 pkg-config 路径导出。这个后端还没有在 macOS 的 GStreamer 上链接过，第一次在该平台构建时请按未探索对待。

## 运行期的 GStreamer 需求

管线是写死元素的，所以缺哪个插件，错误信息里就会出现哪个元素名。大致如下：

| 用途 | 元素 |
| --- | --- |
| 视频 | `h264parse`、`avdec_h264` 或 `d3d11h264dec`、`videoconvert`、一个 sink |
| 音频 | `avdec_aac` 或 `avdec_alac`、`audioconvert`、`audioresample`、一个 sink |

有两样东西常被以为需要，本项目并不需要。一是不需要 Bonjour 或 Avahi：发现逻辑用 `mdns-sd` 在 crate 内实现，不涉及系统守护进程。二是不需要单独安装 FFmpeg：`avdec_h264` 与 `avdec_aac` 随 GStreamer 的 libav 插件包提供。

## 环境变量

| 变量 | 作用 |
| --- | --- |
| `AIRPLAY_GST_LAUNCH` | 指定要运行的 `gst-launch-1.0` |
| `AIRPLAY_GST_AUDIO_SINK` | 音频 sink，Windows 上默认 `wasapisink` |
| `AIRPLAY_GST_VIDEO_SINK` | 视频 sink，默认 `autovideosink` |
| `AIRPLAY_GST_VIDEO_WARM` | 置 `1` 时视频管线随接收端一起启动 |
| `AIRPLAY_DUMP_VIDEO` | 把最早的若干视频帧以 Annex-B 追加写入该文件 |
| `AIRPLAY_DUMP_AUDIO` | 把音频负载带长度前缀追加写入该文件 |

两个 dump 文件都是给诊断用的。视频那份是 Annex-B 流，任何解码器都能读，从而把"流本身不合法"和"流只是难以跟上"区分开。音频那份在每个负载前放四字节小端长度，足以让接收端之外的解码器恢复帧边界。

## 测试

改动完成前必须通过的门禁：

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

另有两组测试需要工作区之外的东西，因此不在上面的列表里。

差分测试把本实现与 UxPlay 自身的 C 代码对比。它需要 `upstream/UxPlay` 下提交 `57ea83411d5f7e0b38c5841987439340543f025c` 的检出、用于编译的 MSYS2，然后：

```sh
pwsh -File tools/oracle/run.ps1
```

GStreamer 那组驱动真实运行时，其中包含解码会话抓取的帧：

```sh
cargo test --manifest-path crates/rust-airplay-gstreamer/Cargo.toml -- --ignored
```

## 排错

| 现象 | 原因与处理 |
| --- | --- |
| 控制端口报 `os error 10013` 或 `10048` | 端口被预留或已被占用。Windows 会为 Hyper-V 保留端口段，`netsh int ipv4 show excludedportrange protocol=tcp` 可查。换一个端口，或用 `--port 0` |
| `--render gst` 报缺少特性 | 构建时没有加 `--features gst` |
| 客户端看不到接收端 | mDNS 组播被拦截，或两者不在同一网络。必须在同一链路上 |
| 重新构建时 `LNK1104: cannot open file ...rust_airplay.exe` | 之前启动的接收端还在运行并占着文件。停掉它，或把二进制复制到别处运行副本 |
| 画面卡住，日志报丢帧 | sink 跟不上。Windows 上试试 `AIRPLAY_GST_VIDEO_SINK=d3d11videosink`，并设 `AIRPLAY_GST_VIDEO_WARM=1` 让窗口在第一个客户端之前就建好 |
| 有声音没画面，或反之 | 其中一项的元素缺失，错误信息里会写出名字，装上提供它的插件包即可 |
