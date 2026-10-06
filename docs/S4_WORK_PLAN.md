# S4 工作计划：媒体会话与 GStreamer 渲染

依据 [MIGRATION_PLAN.md](MIGRATION_PLAN.md) 的 S4 行与 [ADR-0003](ADR-0003-media-backend.md)（先实现上游的 GStreamer 路径，为 FFmpeg 保留替换点）。工作包按依赖排序，每个包以四门禁 + 本地提交交付，验收判据写在包内。

## S4 目标与出口

- 目标：**真实设备可用**——iPhone/iPad 能投屏出画面、能出声，行为对照上游 `renderers/`。
- 出口（对应 MIGRATION_PLAN S4 行）：指定设备 H.264 + AAC-ELD 与 ALAC 播放、长时播放、断线恢复；同硬件对照数据（首帧、延迟、同步偏差、丢帧、CPU/内存），**不写没有实测依据的性能承诺**。
- 明确不做：AirPlay 2 的多客户端/多房间、AirPlay 2 音频（ADR-0003 与上游基线一致）。

## 约束（继承 S3）

1. 协议核心（`crates/airplay-core`）**不链接 GStreamer/FFmpeg**；渲染层独立 crate，由 CLI 注入（ADR-0003）。
2. 生产代码不得因**外部输入**触发 panic；长度运算必须 checked；先校验再分配。
3. 解析与解密不越权：解密只发生在会话密钥已按 A 组契约交付之后。
4. 能力声明只覆盖已实现功能；环境问题（插件缺失、sink 不可用）如实归因，不算移植缺陷。

## 工作包

### S4-0 开放项收尾（不依赖媒体，可立即做）

- **S3 的 W8 验收材料**（[S3_WORK_PLAN.md](S3_WORK_PLAN.md) §8 的 E）：退出标准逐条对照；能力声明区分 `RouteHandlers::new`（配对 501）与 `with_pairing`；ADR 索引与引用一致性复核；README 状态更新。
- **真机请求序列归档**：iPad 已完成"被发现 + 进入配对"的真机验证，但请求序列尚未落档——需要 CLI 的 `--log <文件>`（同样不含密钥）或人工粘贴窗口内容，然后写进验收文档。
- **工程化**：`deny.toml` + cargo-deny（许可白名单与 THIRD_PARTY_NOTICES 对齐、advisory、重复依赖）；配置远程仓库后 CI 的 3 OS + MSRV + oracle 三个 job 才真正运行（当前一律标注"仅本地验证"）。

### S4-1 媒体会话骨架（核心内，无媒体依赖）

**首个切片（下一次开工从这里开始，逐个切片提交）**

| 切片 | 内容 | 上游阅读顺序 | 验收判据 |
| 切片 | 内容 | 上游阅读顺序 | 验收判据 |
|---|---|---|---|
| **S4-1a** 包解析与有界重排序 | ✅ **已完成**（`src/media/audio.rs`）：`parse_audio_packet` 按上游布局读 RTP 头（payload type 96、<12 字节忽略、`00 68 34 00` 无数据标记识别、错误版本/类型拒绝）；`ReorderWindow` 有界（默认 512）按序交付、重复丢弃、缺口跨越并记录丢失（丢失表按容量封顶）、远跳视为新流起点并交付该帧、序号回绕正确 | `lib/raop_rtp.c:558-668` 收包/缓冲路径 + `lib/raop_buffer.c` | 12 条单测全部通过：乱序/重复/丢包/远跳/回绕/容量上限/过去帧丢弃/标记识别/非法包拒绝 |
| **S4-1a-2** 载荷解密 | 用 `SETUP` 交付的 `audio_key`/`audio_iv` 解密载荷：**先核对上游 AES-CBC 覆盖范围与 IV 用法再实现**（不猜），之后 `AudioFrame.payload` 才是明文 | `lib/raop_rtp.c` 的解密调用点 + `lib/crypto.c` | 固定向量 + 与 oracle 差分解密结果一致 |
| **S4-1b** 控制通道 | 🟡 **重传已完成**（`src/media/control.rs`）：解析控制包（类型 = `packet[1] & !0x80`；`0x56` 重发音频、音频从偏移 4 开始；`0x54` 同步包含 `sync_rtp`/`remote_ntp`/`next_rtp`）、按上游 8 字节格式构造重传请求（`0x80 0xd5`、控制序号、起始序号、数量）、`AudioSession::receive_control` 把重发音频走与数据口相同的路径（**补上窗口等待的缺口**而不是当重复丢弃）。**NTP 时序服务仍待做** | `lib/raop_rtp.c:203-233`（请求）+ 控制分支（`0x56`/`0x54`）+ `lib/raop_ntp.c`(1027) 待读 | 6 条控制通道测试 + 2 条会话测试；重发确实补缺口；同步包不报错且不交付；时钟偏差计算（待 S4-1b-2） |
| **S4-1b-2** NTP 时序服务 | 🟡 **报文层已完成**（`src/media/timing.rs`）：按上游 `raop_ntp.c:578-700` 构造 32 字节请求（`0x80 0xd2 0x00 0x07`，偏移 24 放本机 NTP 时间戳，复用 `ntp::unix_nanos_to_timestamp`）并读回应答的三个 32 位值（偏移 8/16/24 = t1/t2/t3，类型 `0x53`）。**时钟算术仍未做**：上游用 `raop_ntp_adjust_remote_timestamp_offset` 调整远端值、拒绝重复与伪造包、并叠加 `sync_delay`，这些必须通读 `raop_ntp.c`(1027 行) 才能忠实实现——不猜 | `lib/raop_ntp.c`（待通读） | 3 条报文层测试（请求字节与上游一致、时间戳往返在格式精度内、请求/短包/错类型被拒） |
| **S4-1b-3** 时钟同步算术 | 偏差与延迟计算、`sync_delay`、重复/伪造包拒绝、`recv_time_kernel` 可能为 NULL 的分支 | `lib/raop_ntp.c` 的 `t1..t4` 段落 | 偏差固定向量 + 与上游对照 |
| **S4-1c** 镜像通道 | 🟡 **已完成**：密钥派生（`AirPlayStreamKey`/`AirPlayStreamIV` + 十进制连接号 + 音频密钥，SHA-512 取前 16 字节）、128 字节包头解析与类型表（`raop_rtp_mirror.c:298-352`，含"哪两类是加密的"）、TCP 监听与分包读取（含短读）、NAL 长度前缀→Annex-B 起始码就地改写与校验（`:453-474`）。**未完成**：视频载荷的 CTR 解密规则 | `lib/raop_rtp_mirror.c` + `lib/mirror_buffer.c` | 已有 11 条镜像测试 |
| **S4-1c-2** 镜像 CTR 解密规则 | ✅ **已完成**（`src/media/mirror.rs`）：`MirrorDecryptor` 按 `mirror_buffer_decrypt` 的规则解密——每包把计数器移到块边界（`aes_ctr_start_fresh_block`，即上一包尾块之后的那一块）、包头的 `carried` 字节用上一包尾块未用的密钥流、尾部按整块零填充解密并留下 `16-rest` 字节给下一包、计数器按本包用掉的**含尾块在内**的块数前进；密钥流由本模块从 IV 显式生成（位置是规则的一部分，不能依赖 CTR 实现自己保存的状态）。错误的 `StreamCipher`（"单条连续密钥流"模型）已删除。另有 `CodecConfig`：未加密的 SPS+PPS 包按上游规则**拼到下一个时间戳相同的视频包前面**；`MirrorSession` 把整条镜像流变成 `VideoFrame`，CLI 现在声明并接受镜像端口。**验证**：`tools/oracle` 新增 `mirror` 模式——用上游自己的 `mirror_buffer_decrypt` 解密全零包，打印它**读到的密钥流位置**，并做上游自身的往返校验；`tests/upstream_oracle.rs` 用同一批向量逐字节比对本移植。另有 23 条单测（密钥流生成器比对 crate 自己的 CTR、40+24 固定向量、非整块长度序列的收发往返、计数器重启、重键、拒绝越界包、codec 拼装四条、会话层四条） | `lib/mirror_buffer.c` 的 `mirror_buffer_decrypt` + `lib/raop_rtp_mirror.c:420-450` | ✅ 固定向量 + 非整块长度（40→24、33、47、1、64、5、128）+ 与上游 oracle 差分全部通过 |
| **S4-1d** 端口与收包 | 🟡 **套接字层已完成**（`src/media/ports.rs`）：`MediaSockets::bind` 绑定 timing/control/data 三个 UDP 端口（上游 `raop_rtp.c:236-244` 同样是这三个），`receive_audio`/`receive_control`/`receive_timing` 驱动收包；socket 级测试证明**真 UDP 上**乱序被正确重排序、重发在控制口补上缺口、时序口把报文交给调用方。**仍待**：① 收包循环的编排（tokio task + 帧回调和背压）；② 镜像 TCP 监听（随 S4-1c）；③ 把绑定端口写进 `SetupPorts` 并接进 CLI（之后真机 SETUP 才声明端口） | `lib/raop_rtp.c:236-244` + `lib/raop_handlers.h` 的 SETUP 分支 | 已有 4 条 socket 测试；接线后需真机确认 SETUP 响应含端口且真有包进来 |

**实现中被新单测抓出的两个自身缺陷**（记录在案以免重犯）：

1. 丢失表无上限：调用方若不取走丢失列表，一个持续丢包的客户端能让它无限增长 → 现按窗口容量封顶（保留最近的）。
2. "远跳视为新流"时**丢掉了触发重启的那一帧**，于是窗口永远等一个客户端已经发过的序号 → 现把该帧作为新流的首帧交付。

写作用域：`src/media/`（新建）、`src/rtp.rs`、`src/mirror.rs`、`src/ntp.rs`、`src/server/`（会话与 `SetupPorts` 接线）。**不**修改 `control.rs` 的既有路由语义。

写作用域：`src/rtp.rs`、`src/mirror.rs`、`src/ntp.rs`、`src/media/`（新建，若需要）、`src/server/`（会话与 `SetupPorts` 接线）。**不**修改 `control.rs` 的既有路由语义。

- 内容：NTP 时序服务与客户端同步（对照 `lib/raop_ntp.c`，注意 3dbf7cee 的 `sync_delay` 与 `recv_time_kernel` 可能为 NULL 的新签名）；音频 RTP 接收（UDP、AES-CBC 解密、序号/时间戳重排序、丢包检测）；重传请求（control 通道）；镜像 TCP（H.264 分片重组、由 `streamConnectionID` 派生的 AES-CTR 密钥与 IV、分片头解析）。
- **端口真实绑定**：时序口、音频 data/control、镜像 data 绑定后写进 `SetupPorts`——SETUP 响应从此声明真实端口（当前未绑定的字段一律不写）。
- 验收：造包/抓包回放测试证明解密与重组正确；乱序、重复、丢包、超长分片均有界且不 panic；无渲染器也能跑（帧可以丢弃或落盘）。写作用域：`src/rtp.rs`（已有基础）、`src/mirror.rs`、`src/ntp.rs`、`src/server/`（会话与新模块）。

### S4-2 渲染边界与 GStreamer 后端

- 内容：在**第一个 renderer 落地时**定义最小接口（`open(config)`、`push(frame, timestamp)`、`flush()`、`close()`、`set_volume`）——不预先创建抽象；新建 `crates/airplay-renderer-gst`，实现 audio（ALAC/AAC-ELD → 音频 sink）与 video（H.264 → video sink，`-avdec` 与 sink 选择照搬上游），行为对照 `upstream/UxPlay/renderers/`。
- **构建隔离形式在本包定案并记录**：renderer crate 不进默认 workspace `members`（或以 feature 隔离），确保没有 GStreamer SDK 的机器上 fmt/test/clippy 仍全绿。
- 验收：本机听到 iPhone 音频、iPad 镜像出画面；插件缺失时报明确错误而不是静默失败；接口与实现均有测试（接口用假 renderer，管线用本机设备手测并记录）。
- 写作用域：新 crate + `crates/airplay-cli`（后端选择）+ 核心的接口定义。

### S4-3 会话生命周期与参数语义

- 内容：RECORD/FLUSH/TEARDOWN 的媒体语义（开始/暂停/恢复/结束）与媒体会话一致；`SET_PARAMETER` 的 volume/progress 真正作用到渲染层；`SETUP` 的多流/重复请求语义；HLS/URL 播放的 `playbin` 路径（S4-2 之后）。
- 验收：与上游逐场景对照（播放中暂停、拖动、切换、断开重连），控制面状态机与媒体会话不脱节。

### S4-4 性能与稳定性设施

- 内容：抓包回放（把真机流量存成用例）+ 同硬件对比（首帧、延迟、A/V 同步偏差、丢帧、CPU、内存）；长时播放与断线恢复用例。
- 验收：给出**可复现的测量方法与数据**；对照上游同配置数据；把结论写进计划而不是口口相传。

### S4-5 FFmpeg 后端（推迟，接口已预留）

- 只有当 S4-1/S4-2 完成后、且有明确收益目标（如更低延迟或更少依赖）时才做：新增同级 crate 实现同一接口，协议侧不改（ADR-0003 第 4 条）。

## 依赖关系与顺序

```
S4-0（可立即做，不依赖媒体）
  └─ 真机证据归档 ──┐
S4-1 媒体会话骨架 ───┼─→ S4-2 渲染边界 + GStreamer ─→ S4-3 生命周期/参数 ─→ S4-4 性能与稳定性
                     │                                        └─→ S4-5 FFmpeg（可选）
                     └─ 端口绑定写进 SetupPorts（S4-1 后 SETUP 才声明真实端口）
```

## 需要用户决策/输入

1. ~~**GStreamer 安装方式**~~（2026-10-06 已定）：用户问"发布到 GitHub 怎么选更好"，据此选了**运行期子进程接入**：默认 `cargo test`/`clippy` 在没有 GStreamer 的机器上全绿，贡献者不需要第二套 Rust 工具链，本机也不需要 MSVC 版 SDK。本机用 MSYS2/UCRT64 的 GStreamer 1.28.3（`C:\msys64\ucrt64\bin` 进 PATH）验证，六个集成测试通过；真机出声出画仍待做。
2. **对照设备**：是否只有一台 iPad（用于镜像与音频），有无第二台设备/播放器做对照。
3. **CI 远程**：是否配置远程仓库让三个 job 真正运行（否则继续标注"仅本地验证"）。
4. **S4-0 是否先做**：我建议先做 S4-0 的 W8 验收材料与真机证据归档，把 S3 正式收口，再进 S4-1。

## 进度记录

| 日期 | 事项 |
|---|---|
| 2026-10-05 | 计划建立；ADR-0003 定案（GStreamer 先行、FFmpeg 留替换点） |
| 2026-10-06 | S4-1c-2 完成：镜像 CTR 规则重写为 `MirrorDecryptor`（上游规则逐条），SPS/PPS 拼装，`MirrorSession` 接线；`tools/oracle` 新增 `mirror` 模式做上游差分 |
| 2026-10-06 | S4-2 接口与后端落地：`crates/airplay-renderer`（零依赖边界 + `NullRenderer`）、`crates/airplay-renderer-gst`（子进程接入，6 条集成测试对真 GStreamer 通过）；ADR-0003 第 5 条记录接入方式。真机出声出画与 CLI 接线未做 |
| 2026-10-06 | S4-1b-3（NTP 时钟算术）**未完成**：试做的工作被回退（`git stash`），原因见交接文档第 7 节——固定偏移与漂移偏移的语义没有与上游对齐到可验证的程度，宁可不落地也不落地一个错的时钟信号 |
