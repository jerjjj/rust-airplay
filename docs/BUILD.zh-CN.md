# 构建与运行

English: [BUILD.md](BUILD.md)。

## 工具链

Rust 1.85 或更新版本，edition 2024。工作区在 `Cargo.toml` 里固定了 `rust-version`，因此旧工具链会得到明确提示，而不是难以理解的报错。

C 链接器是 Rust 工具链本身需要的，不是这份代码需要的：Windows 上用 rustup 默认 host triple 附带的 MSVC 链接器，macOS 与 Linux 用系统编译器。

## 默认构建

```sh
cargo build --workspace
```

不链接任何 C 库。`cargo tree --workspace` 里没有任何 `-sys` crate，这是刻意保持的性质：没有 GStreamer、没有声卡、没有显示器的机器照样能构建、测试与 lint 整个工作区。

## GStreamer 后端

`rust-airplay-gstreamer` 是独立的 crate，被排除在 workspace 之外，因此上面的门禁永远不需要系统库。

### 构建期需要

GStreamer 开发库，通过 `pkg-config` 查找：

| 模块 | 来源 |
|---|---|
| `gstreamer-1.0` | GStreamer 核心开发包 |
| `gstreamer-app-1.0` | GStreamer plugins-base 开发包（或单独的 `gstreamer-app-1.0`） |
| `glib-2.0`、`gobject-2.0`、`gio-2.0` | GLib 开发文件，GStreamer 包会依赖它们 |

### Windows（MSYS2 UCRT64）

安装 MSYS2 的 GStreamer 及其开发包，并确保能找到 `pkg-config`：

```powershell
$env:PATH = 'C:\msys64\ucrt64\bin;' + $env:PATH
pkg-config --modversion gstreamer-1.0
cargo build -p rust-airplay-cli --features gst
```

MSYS2 提供的是 MinGW 导入库。在这台机器上它们能与 MSVC 工具链链接，但这个组合不是两个项目中的任何一个所支持的配置；如果链接阶段在这台机器上失败，可以只用库本身，或者用你本机 GStreamer 安装所支持的方式走 `gst-launch` 子进程路径。

### Debian 与 Ubuntu

```sh
sudo apt-get install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
cargo build -p rust-airplay-cli --features gst
```

### macOS

未验证。用官方安装器或 Homebrew 装的 GStreamer 开发文件，在导出路径后应能被 `pkg-config` 找到。产物请按"未验证"对待。

## 运行期需要

GStreamer 运行时，且下列元素可用：

| 用途 | 元素 |
|---|---|
| 镜像视频 | `h264parse`、`avdec_h264`（Windows 上还可用 `d3d11h264dec`）、`videoconvert`、sink（`autovideosink`、`d3d11videosink`） |
| 音频 | AAC-ELD 用 `avdec_aac`，ALAC 用 `avdec_alac`，另有 `audioconvert`、`audioresample`、sink（Windows 上 `wasapisink`，其他平台 `autoaudiosink`） |

两件常被误以为需要、本项目并不需要的东西：

- **不需要 Bonjour 或 Avahi**：发现由 `mdns-sd` 在 Rust 内实现，不涉及系统 DNS-SD 守护进程。
- **不需要单独安装 FFmpeg**：`avdec_h264` 与 `avdec_aac` 来自 GStreamer 的 libav 插件包。

缺少元素时，会在打开流时报明确错误，而不是表现为无声。

## 环境变量

| 变量 | 作用 |
|---|---|
| `AIRPLAY_GST_LAUNCH` | 指定要运行的 `gst-launch-1.0` |
| `AIRPLAY_GST_AUDIO_SINK` | 音频 sink 名 |
| `AIRPLAY_GST_VIDEO_SINK` | 视频 sink 名 |
| `AIRPLAY_GST_VIDEO_WARM` | 置 `1` 时视频管线随接收端一起启动，而不是等第一个客户端 |
| `AIRPLAY_DUMP_VIDEO` | 把最早的若干视频帧以 Annex-B 追加写入该文件 |
| `AIRPLAY_DUMP_AUDIO` | 把音频负载带长度前缀追加写入该文件 |

## 差分测试

`cargo test -p rust-airplay --test upstream_oracle -- --ignored` 把本实现与 UxPlay 自身的 C 代码对比：密钥流位置、解密器 carry 行为、配对向量与计数器算术。

需要：

1. 在 `upstream/UxPlay` 放一份上游检出，提交号见 `research/evidence/upstream-commit.txt`（该目录被 git 忽略）；
2. 用 `pwsh -File tools/oracle/build.ps1` 构建 oracle，它依赖 MSYS2/UCRT64；
3. 设置 `AIRPLAY_UPSTREAM_ORACLE` 指向构建出的可执行文件，或直接用 `pwsh -File tools/oracle/run.ps1`（它会替你设置）。

这组测试不进 CI：上游检出没有随仓库分发，oracle 构建脚本依赖 Windows 工具链。

## 常见问题

| 现象 | 原因 |
|---|---|
| 控制端口报 `os error 10013` 或 `10048` | 端口被保留或已被占用；Windows 会为 Hyper-V 保留端口段。换一个端口，或用 `--port 0` |
| `--render gst` 报缺少特性 | 构建时没加 `--features gst` |
| 接收端起来了但客户端找不到 | 检查 mDNS 组播是否被拦截，以及接收端与客户端是否在同一网络 |
| 重新构建时 `LNK1104: cannot open file ...rust_airplay.exe` | 上一次的接收端还在运行；停掉它，或把二进制复制到别处运行副本 |
| 画面卡住并丢帧 | sink 跟不上；参见 `crates/rust-airplay-gstreamer/README.md` 里关于 sink 的说明 |
