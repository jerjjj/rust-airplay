# ADR-0003：媒体后端先实现 GStreamer，并为 FFmpeg 保留替换点

- 状态：已接受（2026-10-05）；**第 5 条于 2026-10-06 补充**（GStreamer 的接入方式，见下）
- 决策者：用户（"ffmpeg 先不实现，先实现上游的 GStreamer，但要保留后续实现 FFmpeg 的优化"；2026-10-06 用户追加："如果我将源代码发布到 github 上，怎么选更好"）
- 相关：[MIGRATION_PLAN.md](MIGRATION_PLAN.md) S4、[ARCHITECTURE.md](ARCHITECTURE.md)、[ADR-0001](ADR-0001-network-stack.md)

## 背景

S3 只做到控制面、配对与发现，媒体（音频 RTP、镜像 TCP、重传、NTP 同步、解码与呈现）属 S4。媒体后端有两条现实路径：

1. **上游的 GStreamer 路径**。上游把 GStreamer 放在**独立子工程** `upstream/UxPlay/renderers/`：`renderers/CMakeLists.txt` 自己找 pkg-config 与 GStreamer（支持 macOS 的 `GStreamer.framework`、`GSTREAMER_ROOT_DIR`、Brew/自装路径），实现 audio/video 两个 renderer；应用层 `uxplay.cpp` 负责建 GLib 主循环、挂 `video_renderer_listen`/`audio_renderer_listen` 的 bus watch（最多 `MAX_VIDEO_RENDERERS`/`MAX_AUDIO_RENDERERS` 个），HLS/URL 播放另走 `playbin`（`DEFAULT_PLAYBIN_VERSION 3`、x11 窗口回调、`-avdec` 选择软/硬解码器）。也就是说：**上游自己就把"协议核心"与"GStreamer 渲染"分成两层**，核心不链接 GStreamer。
2. FFmpeg/libav 直接解码。控制力更强、依赖更轻，但需要自己写 ALAC/AAC-ELD 的封装与呈现，偏离"固定上游基线、功能对等"的项目目标；用户当前选择不做。

项目目标是**与固定上游基线功能对等**，而不是自创媒体路径。因此先实现 GStreamer 路径才是最短、最可对照的路。

## 决策

1. **S4 的媒体后端按上游形态实现：GStreamer**，audio 与 video 两个 renderer，行为对照 `upstream/UxPlay/renderers/`；HLS/URL 用 `playbin`。
2. **协议核心（`crates/airplay-core`）不链接 GStreamer，也不链接 FFmpeg**。这一条既延续 [ARCHITECTURE.md](ARCHITECTURE.md) 的既有约定（S1–S3 的依赖集里没有媒体库），也让 README 里"协议核心阶段不依赖 GStreamer SDK"的承诺在 S4 之后依然成立。
3. **GStreamer renderer 落在独立 crate**（暂名 `crates/airplay-renderer-gst`），与上游 `renderers/` 子工程一一对应；将来加 FFmpeg 后端就是**再加一个同级 crate**，CLI 通过选择后端把二者接上。
4. **替换点（为 FFmpeg 保留的边界）**：核心向渲染层交付的是**帧与配置**，不是管线。S4 落地时定义最小接口——音频/视频各自 `open(config)`、`push(frame, timestamp)`、`flush()`、`close()`，加上 `set_volume`——由 CLI 注入。核心只认识这个接口，因此：
   - 换成 FFmpeg 后端时，协议侧（RTP、重传、解密、NTP、A/V 同步的协议部分）**不动**；
   - 两个后端可以并存（例如 GStreamer 负责视频显示、FFmpeg 负责音频），互不影响；
   - 接口现在**不预先创建**（[ARCHITECTURE.md](ARCHITECTURE.md)：不为未实现的功能造抽象），在 S4 第一个 renderer 落地时随代码一起定义，并由该 renderer 的测试覆盖。

5. **接入方式（2026-10-06 补充）：GStreamer 以 `gst-launch-1.0` 子进程接入**，不链接 `libgstreamer`。接口落在 `crates/airplay-renderer`（**默认 workspace 成员**，无系统依赖），后端落在 `crates/airplay-renderer-gst`（**独立 workspace，不进默认门禁**）。理由与代价记录在 `crates/airplay-renderer-gst/README.md`，要点：
   - Windows 上常见的 GStreamer 是 MSYS2 构建，其导入库 MSVC 链接器不接受；直接链接会把"能编译"变成对一个特定 C 构建的构建期依赖。
   - 用户要把源码发布到 GitHub，因此**默认 `cargo test`/`clippy` 必须在没有 GStreamer 的机器上全绿**，且不应要求贡献者装第二套 Rust 工具链。
   - 子进程接入不需要 `unsafe`、不跟踪 C ABI；代价是管线只能靠描述串配置，`set_volume` 这类需要触碰管线属性的操作被推迟到 S4-3。

6. **音频改为链接（2026-10-12 修订第 5 条，仅音频）**：**视频仍是子进程，音频改为链接 `gstreamer` + `gstreamer-app`**（`gstreamer-rs` 0.24）。这不是取舍偏好，而是**协议本身决定的**：
   - 子进程的音视频都是"往 stdin 写字节流"。H.264/H.265 有 start code，解码器能在字节流里找到图像边界；**raw AAC-ELD 没有同步字、也不带采样率**，字节流无法表达"一帧到哪里结束"。上游用 `appsrc`**一帧一 buffer**（`renderers/audio_renderer.c:332-367`），这是唯一可行的投递方式。
   - 实证：`tools/eld_probe.c` 用上游的方式喂真机数据，**300/300 buffer 解码出 PCM**；而同样数据走 `fdsrc` 字节流的每一种封装（`aacparse`、ADTS、LOAS）全部被拒。详见 `tools/eld_probe.md`。
   - 顺带修掉一个关键错误：raw AAC 的 caps **必须带 `codec_data`**（`AudioSpecificConfig`），上游值是 `f8e85000`（ELD/44100/stereo/480）；本移植此前没有，且猜的 `f82c00` 连 ELD 都不是。
   - **代价与边界**：`crates/airplay-renderer-gst` 的**构建期**从此需要 GStreamer 开发库（`pkg-config` 找 `gstreamer-1.0`/`gstreamer-app-1.0`）；默认 workspace（含 `airplay-renderer` 与 `airplay-core`）**仍然不链接任何 C 库**，第 2 条的承诺不变。门禁因此需要两种：默认门禁 + `--features airplay-cli/gst` 门禁。

## 后果

- **新增系统依赖**：GStreamer SDK（`gstreamer-1.0`、`gstreamer-app-1.0`、`gstreamer-video-1.0`、`gstreamer-audio-1.0`）与插件集（ALAC、AAC/AAC-ELD 解码、H.264 解码、音视频 sink）。Windows 上需要 MSVC 版 GStreamer 运行时安装器；macOS 上沿用上游偏好的 `GStreamer.framework`。**第 5 条之后这条放宽为运行期依赖**：构建期不需要 GStreamer，只有运行一个真要出声出画的会话才需要 `gst-launch-1.0` 与相应插件；缺什么在打开流时报明确错误（`RenderError::Unavailable`），按 ADR 原文归为环境问题。
- **CI 影响**：默认 workspace 的 fmt/test/clippy 必须能在**没有** GStreamer SDK 的机器上通过，因此 renderer crate 不能进默认 `members`，需要 feature 或独立 workspace 成员 + 显式构建命令；具体形式在 S4-1 决定并记录。
- **性能与保真**：GStreamer 路径的性能特征（首帧、延迟、CPU）将与上游**同构**，便于 MIGRATION_PLAN 测试策略第 5 条的同硬件对比；换 FFmpeg 后的收益必须用同一套设施测量，不能只凭感觉。
- **风险**：GStreamer 插件在不同发行版/Windows 上的可用性差异较大（AAC-ELD、硬件 H.264）。上游对此已有 `-avdec` 与 sink 选择选项；本移植照搬这些选项，并把"某插件缺失"归为环境问题而非移植缺陷，如实记录。

## 被否决/推迟的选项

- **FFmpeg 直接实现（推迟，非否决）**：用户明确先不做。保留替换点即是为它留门；将来若做，按上面第 4 条新增同级 crate，不改协议侧。
- **纯 Rust 解码器**（如 symphonia + openh264）：依赖最轻、无系统库，但 AAC-ELD 与硬件解码的保真风险高，且与"对照上游"的验收方式冲突。若将来要做，同样可以在第 4 条的接口下新增 crate。
