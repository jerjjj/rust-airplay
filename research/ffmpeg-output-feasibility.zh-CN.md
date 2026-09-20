# FFmpeg 视频输出可行性评估

日期：2026-09-20。代码基线：FDH2/UxPlay@57ea83411d5f7e0b38c5841987439340543f025c；本地 Rust HEAD d303500。

结论：可行。优先增加可选的 FFmpeg 录制、转码、推流输出，保持 GStreamer 播放后端；完整替代播放器属于更大的独立工程。本研究不调整 S2 验收状态，也不代表已实现 AirPlay 接收或实机互操作。

用户尚未限定输出用途，因此同时评估外部输出与本机播放。FFmpeg 是新增的外部原生媒体依赖；应用、协议、配对和密码处理仍用 Rust。若坚持外部依赖只有 GStreamer 和系统接口，则需先接受 FFmpeg 作为新增例外。研究用 Python 脚本只用于可重复的离线实验，不进入产品运行路径。

## 已核对的接入位置

- `upstream/UxPlay/lib/raop_rtp_mirror.c`：镜像数据解密，维护 SPS/PPS（HEVC 还有 VPS），准备压缩视频并调用 video_process。
- `upstream/UxPlay/uxplay.cpp:2407`：video_process 已把同一压缩数据分发到 dump、mux 和播放。播放前还处理远端与本地时钟偏移。
- `upstream/UxPlay/renderers/video_renderer.c:383`：appsrc → queue → parser → decoder → converter → videoscale → videosink。RTP 分支直接封包压缩码流。
- `upstream/UxPlay/renderers/video_renderer.c:634`：接收 Annex B 数据与时间戳，在启用同步时给 GstBuffer 设置 PTS。
- `upstream/UxPlay/renderers/mux_renderer.c:227`：压缩视频与时间戳进入录制分支，基于共同起点设置 PTS；上游直接令 DTS = PTS，Rust 移植不能未经样本验证就假设所有流都无重排序。
- `crates/airplay-core/src/mirror.rs:11,86`：已有原始镜像 timestamp 字段与长度前缀 NAL → Annex B 转换。它们尚不构成完整解密、codec 配置解析、时钟映射及媒体输出链。

推荐在解密、视频配置解析和时间归一化之后，解码之前分发编码视频包。无需修改 FairPlay/SRP，也无需为了录制先解码再编码。启用 libav 插件让 GStreamer 使用 FFmpeg 解码器，是另一种依赖方式，不等同于独立 FFmpeg 输出后端。

## 三种路径

| 路径 | 适用用途 | 优势 | 主要成本 |
|---|---|---|---|
| Rust 启动 ffmpeg 子进程 | 录制、转码、推流 | 进程隔离；无需链接 FFmpeg ABI；开发验证较快 | 管道协议、时间戳、背压、退出和日志管理 |
| Rust bindings 调用 libavcodec/libavformat | 精确时间控制、嵌入式媒体处理 | 直接传递 AVPacket 的 PTS/DTS/time_base；减少中间封装 | 原生 SDK 和版本矩阵；绑定 API 与内存所有权；硬件帧集成 |
| FFmpeg 解码 + 自建窗口/音频播放 | 本机显示替代 GStreamer | 播放策略可控 | FFmpeg 库不是完整播放器；还需显示、音频时钟、调度、同步、窗口事件 |

建议先用子进程验证外部输出。若明确需要内嵌播放器或精确逐包控制，再选择 libav 后端。ffplay 可作独立窗口原型，不应当作已有完整产品播放器。Rust 核心继续 forbid(unsafe_code)；绑定库内部可以有 FFI，自有核心不引入裸指针。所选 wrapper 若不能安全覆盖所需 API，应重新审视后端边界，不能偷偷放宽全局 lint。

## 时间戳是正式接入的关键

裸 Annex B stdin 只携带视频比特流，不携带每个 AirPlay 包的时间戳。指定固定帧率或让 FFmpeg 生成 PTS 可以完成演示，但不能保留屏幕静止、变帧率、时间跳变和音画同步。

正式方案二选一：

1. 子进程：经带时间戳容器（例如 MPEG-TS）输入 FFmpeg。保留 GStreamer 时可由 Rust GStreamer 后端完成封装，再送二进制管道；想让该输出完全独立于 GStreamer，需要实现或选用经验证的 Rust muxer，这不是零成本适配。
2. libav：直接交付编码包、time_base、PTS、可用的 DTS、codec 参数；时间映射由 Rust 会话层完成。

数据契约应包含 codec、编码数据、PTS、可选 DTS/duration、time_base、关键帧信息、配置版本及 discontinuity。分别传递 Config、Packet、Flush、End 事件；以流代次隔离重连后的旧帧。SPS/PPS/VPS 与随机访问帧一起支持后端重启。不要把任意 TCP chunk 当视频帧，也不要假定每个 NAL 都是一帧。

## 输出范围和限制

- MP4/MKV：目标支持输入 codec 时使用 stream copy，减少 CPU 和编码延迟。普通 MP4 需要正常写尾；需异常恢复时评估分片 MP4 或分段录制。重新协商分辨率/codec 时优先分段，避免混用不兼容轨道参数。
- MPEG-TS/SRT：适合网络输出，仍需接收端 codec 兼容。SRT 的重传和延迟配置会影响端到端延迟。
- RTMP：H.264 通常有更广的兼容性；HEVC 需确认 enhanced RTMP 与接收端支持，必要时转码。
- HLS：可实现，但切片、关键帧间隔和播放器缓冲决定延迟，不能当作低延迟镜像的默认替代。
- 音频：本研究只实测视频；AAC-ELD/ALAC 不能一概按 AAC-LC 处理，也不能保证所有目标容器和服务支持直拷贝。加入音频需检查 extradata、帧布局、时间基、重采样/转码及共同媒体时钟。
- JPEG 封面、HLS/URL/FCUP 播放不属于 Annex B 镜像输出的自动覆盖范围。

输出任务使用有界队列，慢磁盘或推流端不能无限阻塞协议接收。录制要求完整性时显式报告溢出并停止/分段；实时输出丢帧需考虑参考帧依赖，通常等待下一随机访问帧恢复，不能随意删除中间 P 帧。子进程持续排空 stderr，处理 broken pipe、退出码、关闭 stdin 后等待写尾，超时再终止。用 Rust Command 参数数组启动；不要拼接 shell 命令，也不要在 Windows 上用文本管道传二进制视频。

硬件加速需单独验证 GPU、驱动、像素格式及编码器实际初始化；能力列表有 NVENC/QSV/AMF 不代表本机可运行。当前 Rust 默认 MSVC，本机 FFmpeg 来源为 MSYS2 UCRT64；子进程不受 Rust 链接 ABI 影响，libav 绑定则必须实际验证工具链、头文件、导入库及 DLL 部署。

## 本机实验与证据

命令：`python research/probe-ffmpeg.py`。脚本仅生成合成视频，在 target/ffmpeg-probe 下写入可覆盖实验文件，不访问网络、不启动可见窗口。

环境：Windows x64，MSYS2 UCRT64 FFmpeg 8.1.1，libavcodec 62、libavformat 62；本机列出了 libx264、libx265、MP4、Matroska、MPEG-TS、pipe、RTMP、SRT。

| 实验 | 实际结果 | 证明范围 |
|---|---|---|
| H.264 裸流 → 二进制 stdin → 解码及 stream-copy MP4 | 160×90，30 帧，成功 | 子进程可消费该合成视频；固定 30 fps 补时间戳 |
| HEVC 裸流 → 二进制 stdin → 解码及 stream-copy MP4 | 160×90，30 帧，成功 | 同上；不证明 HEVC AirPlay 已接通 |
| 带时间跳变的 MPEG-TS → stdin → stream-copy MKV | 30 帧；保留 0.533 秒间隔；相对 PTS 最大误差约 0.333 ms | 容器管道能保留此合成样本的时序；误差为此封装下的时间精度表现 |

结果快照：research/evidence/ffmpeg-probe.json；脚本可重新生成 target 下的原始结果。没有进行真实设备、音画同步、硬件加速、RTMP/SRT 网络、长时运行或性能对比；没有验证 Rust libav bindings。

## 建议分阶段实施（提案）

- F0，现已完成研究验证：识别压缩码流边界，验证合成管道和时间戳。后端用途选择仍由用户决定。
- F1，可与剩余 S2 并行排期：Rust 离线 FFmpeg 子进程输出原型；验证进程生命周期、取消、错误传播、队列限额。裸流模式清楚标记为诊断用途。
- F2，随 S3/S4 接入：实现视频配置和时间映射，确定带时间戳输入方式；真实 H.264 流录制、重连、横竖屏切换、FFmpeg 意外退出恢复。保留可选 GStreamer 播放。
- F3，随 S5/S6 完善：HEVC、音频同步、分段/分片文件、推流和硬件编码；按目标输出组合验收。
- F4，仅在需要完整替代播放时开展：libav 解码、显示和音频设备、主时钟、硬件帧及跨平台播放器测试。

F2 退出标准包括：目标 codec 可解码；帧数/PTS 与输入符合约定；长时音画漂移在预先约定范围内；慢输出不会无限占用内存或阻塞协议；正常退出可读文件；横竖屏/重连有明确分段或重建行为。延迟、CPU 和内存指标在同设备、同流、同输出模式下对比，不预先承诺优于 GStreamer。
