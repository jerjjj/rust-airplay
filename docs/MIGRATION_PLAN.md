# 分阶段迁移计划（全项目）

> 本文件是**全项目计划的唯一权威**：阶段划分、退出标准、贯穿性工作流、依赖顺序、决策点与诚实清单都在这里；每个在进行中的阶段另有执行文档（见下表"执行文档"列），只写该阶段的工作包与判据，不再重复本文件的内容。

## 1. 目标与非目标

**目标**：把 UxPlay 的应用、协议、配对、密码处理（含 PlayFair）、渲染调度与 Python 外围逻辑全部改为 Rust；允许 GStreamer 与必要的系统接口。最终产品**不得链接遗留 UxPlay C 核心，也不得依赖原 Python beacon 脚本**。

**非目标**（明确不做，避免范围蔓延）：

- AirPlay 2 的多客户端、多房间、AirPlay 2 音频流；目标是**固定上游基线的功能对等**。
- 自创媒体路径：渲染后端按上游形态实现（[ADR-0003](ADR-0003-media-backend.md)），FFmpeg 作为**保留的替换点**而非当前任务。
- 在没有实测依据时宣称兼容或性能。

## 2. 基线与差异核对

- **移植基线**：`FDH2/UxPlay@57ea83411d5f7e0b38c5841987439340543f025c`（2026-09-17，1.74 Experimental）。
- **差异核对基准**：`3dbf7ceee65932154e85a2f83963d53520a799fa`（2026-10-05，自基线 22 个提交）。逐文件判定见 [S3_WORK_PLAN.md](S3_WORK_PLAN.md) §0。
- 引用约定：代码中的上游引用写 `upstream/UxPlay@<hash>`，哈希必须是上述两者之一的前缀——由 `crates/airplay-core/tests/reference_consistency.rs` 强制。
- 上游检出**不进仓库**（`.gitignore`），差分 oracle 在本地从它构建（`tools/oracle/`）。

## 3. 阶段总表

| 阶段 | 内容 | 退出标准 | 执行文档 | 状态 |
|---|---|---|---|---|
| **S0** | 仓库、许可、workspace、CI、上游来源记录 | 本地 fmt/test/clippy 通过，来源与阶段清晰 | — | ✅ 已完成 |
| **S1** | 有界协议基础：镜像帧、NAL、RTP、NTP 表示、HTTP/RTSP 消息 | 拆包/粘包、混合字节序、截断、超限、错误输入测试通过 | — | ✅ 已完成 |
| **S2** | 原版运行基线、密码原语、密钥派生、Apple SRP、PlayFair 移植 | 受控 C oracle 差分、合法向量、重新配对/换流测试通过 | — | ✅ 已完成（离线） |
| **S3** | 发现、控制面、plist、会话生命周期与访问控制 | 可发现；真实配对 / SETUP / RECORD / FLUSH / TEARDOWN；错误连接有界 | [S3_WORK_PLAN.md](S3_WORK_PLAN.md) | 🟡 主体完成，剩验收材料与真机证据归档 |
| **S4** | 镜像 TCP、音频 UDP、重传、NTP 同步、GStreamer 渲染后端 | 指定设备 H.264+AAC-ELD、ALAC、长时播放、断线恢复 | [S4_WORK_PLAN.md](S4_WORK_PLAN.md) | ⬜ 未开工（后端已决策） |
| **S5** | H.265、HLS/FCUP/URL 播放、语言/字幕、模式切换 | 与基线逐场景对照，正确控制播放与媒体切换 | 待建 | ⬜ 未开始 |
| **S6** | MP4、RTP 转发、封面/元数据、CLI/配置完整兼容 | 选项与行为矩阵完成，录制/导出可用 | 待建 | ⬜ 未开始 |
| **S7** | BLE、AWDL、Windows/Linux/macOS/BSD/ARM、部署 | 约定硬件与平台矩阵验收；正确标明上游已有的平台限制 | 待建 | ⬜ 未开始 |
| **S8** | 综合差分、fuzz、资源与性能、许可与发布 | 不含自有遗留 C/Python；真实设备与长时回归通过 | 待建 | ⬜ 未开始 |

### S3 剩余（收口清单）

1. **验收材料（W8/E）**：退出标准逐条对照；能力声明区分 `RouteHandlers::new`（配对路由答 501）与 `with_pairing`；ADR 索引；README 状态。
2. **真机证据归档**：iPad 已实际发现接收端并进入 PIN 配对（用户确认可连接），但**请求序列尚未落档**——需要 CLI 的 `--log <文件>`（同样不含密钥）或人工粘贴，然后写进验收文档。
3. **已知微债**：`GET_PARAMETER` 只实现 `volume` 分支（`progress` 查询按上游返回空体）；日志/观测只有请求轨迹，没有分级日志。

### S5–S8 大纲（开工前细化）

- **S5**：H.265（上游的 codec 枚举与 `get_custom_profile`）、HLS/FCUP 与 URL 播放（`playbin` 路径）、`SET_PARAMETER` 的 progress 驱动、语言/字幕与模式切换。
- **S6**：MP4 录制/导出、RTP 转发、封面（`image/jpeg|png`）与 DMAP 元数据落到渲染层、CLI 选项矩阵与配置文件完整兼容。
- **S7**：BLE 发现（上游 beacon 的替代）、AWDL/P2P（macOS 特性）、Windows/Linux/macOS/BSD/ARM 的构建与部署矩阵，如实标注上游既有平台限制。
- **S8**：综合差分（对上游逐场景）、fuzz 常态化、资源与性能报告、许可与发布检查（含"无自有遗留 C/Python"的机械核对）。

## 4. 贯穿性工作流（每个阶段都要做）

1. **四门禁**：`cargo fmt --all -- --check`、`cargo test --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`。公开 API 必须有文档；关键输入边界必须有针对性测试。
2. **上游差分**：`pwsh -File tools/oracle/run.ps1`（自动构建 oracle、设置 `AIRPLAY_UPSTREAM_ORACLE`、跑全部 ignored 差分）。**产品不链接 oracle**。
3. **测试策略**：确定性 wire 原语用手工向量；密码链用上游 C 差分；控制层记录多连接/乱序/重复；属性测试守"任何输入都不 panic"与各类上限；随机数、时间、端口通过接口注入。
4. **真机矩阵**：记录客户端型号、系统版本、服务端平台、媒体插件、网络、基线结果与 Rust 结果；未测就写未测试，不伪造。
5. **依赖与许可**：新增依赖需在 [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md) 与相应 ADR 记录；GPL-3.0-or-later 兼容性核对；`cargo-deny` 待接入（S4-0）。
6. **文档与 ADR**：栈选择写 ADR（0001 网络栈、0002 配对身份、0003 媒体后端）；完成的工作写进所在阶段计划的进度记录。
7. **提交纪律**：一个工作包一个本地提交；提交信息说明**为什么**而不只是**改了什么**；未验收的项目保持未完成标注。

## 5. 依赖顺序

```
S0 ─ S1 ─ S2 ─┬─ S3（主体完成）─ S3 收口（S4-0）─┐
              │                                  ├─→ S4（媒体会话 → 渲染 → 生命周期 → 性能）─→ S5 ─→ S6 ─→ S7 ─→ S8
              └─ ADR-0003（GStreamer 先行/FFmpeg 留点）┘
```

- S4-1（媒体会话骨架）与 S3 收口可并行：前者不依赖验收材料。
- S4-2（GStreamer 渲染）依赖"选定一致的 SDK/ABI 并验证链接"这一步（见 §7 环境）。
- S5/S6 依赖 S4 的渲染层能稳定播放。

## 6. 决策点（需要用户输入）

| # | 决策 | 影响 | 现状 |
|---|---|---|---|
| 1 | 配置远程仓库、让 CI 真正运行（3 OS + MSRV + oracle 三个 job） | 跨平台与 MSRV 有证据；否则一律标注"仅本地验证" | 未决（工作流已就绪） |
| 2 | GStreamer 首个验收平台（Windows MSVC 运行时 / Linux / macOS） | 决定 S4-2 的首个验收环境与 ABI 选择 | 未决 |
| 3 | 对照设备（除现有 iPad 外是否有第二台客户端/播放器） | 决定真机矩阵能覆盖多少行 | 未决（iPad 可用于镜像/音频） |
| 4 | 是否先做 S4-0（我的建议：先收口 S3 并归档真机证据） | S3 何时正式关闭 | 未决 |

## 7. 已验收 vs 未验收（诚实清单）

**已由测试或实测支撑**：

- 协议原语、密码链与 SRP/PlayFair 与上游 C 的**7 条差分**（本机一条命令可复现）。
- 控制面、配对（PIN 三步、legacy、pair-verify 签名）、SETUP 密钥派生与 Digest 口令：165 个测试覆盖，含并发护栏（每连接独立密钥、口令失败跨连接计数、超额连接排队）。
- 发现广告：字段逐条对齐上游；**本机 iPad 已实际发现接收端并进入 PIN 配对流程**（用户确认可连接）；`mdns-sd` 与 Apple Bonjour 在本机共存实测通过。
- `/info` 与广告同源（同一 `Advertisement` 值）、引用一致性断言、属性测试（含深 bplist 拒绝）。

**未验收（不得宣称完成）**：

- **媒体**：镜像画面、音频出声、A/V 同步、长时播放、断线恢复——全部属 S4，未实现。
- **真机细节**：iPad 实际请求序列（是否带 `CSeq`、三阶段 plist 形状、`SETUP` 的 `streams` 取值）未归档；配对后的媒体失败点未记录。
- **平台**：macOS/Linux/BSD/ARM 未构建验证；`mdns-sd` 仅 Windows 实测；oracle 的 `build.ps1` Linux 分支未验证；CI 从未运行。
- **环境**：GStreamer 只验证过"工具与插件可查询"（见下），**未验证 AirPlay AAC-ELD 样本**，也未验证与默认 Rust 工具链的 ABI 一致性。
- **AirPlay 2**：不在范围（§1）。

## 8. 环境记录

- Rust 1.98.1，默认 `stable-x86_64-pc-windows-msvc`；MSRV 1.85（CI 有独立 job，本地未跑）。
- 检测到 MSYS2 UCRT64 的 GStreamer 1.28.3 与 `avdec_aac`；只验证了工具与插件可查询，**未验证 AirPlay AAC-ELD 样本**。
- 默认 Rust 与当前 GStreamer SDK 来源不同；**S4-2 前必须选定一致的 ABI 并验证链接**，不能仅凭 pkg-config 成功就认为媒体后端可构建。
- 本机保留的真实设备会话：iPad 发现 + 进入配对（请求序列未归档，见 §7）。
- 本机网络：LAN `192.168.1.x`，Apple Bonjour 服务在运行（与自研 responder 共存已实测）；Windows 防火墙已有 `rust-airplay` 入站规则；Hyper-V 保留 TCP 6935–7034（默认端口 7000 不可绑定，需换端口或 `--port 0`）。

## 9. 历史记录

- 2026-09-19：环境记录建立（首轮实机矩阵见下）。
- 2026-10-05：差异核对复核（22 个上游提交）；S3 主体完成；ADR-0003 定案；本文件重构为全项目计划（阶段总表加执行文档列、贯穿性工作流、依赖顺序、决策点、诚实清单）。
- 2026-10-05：S3 结构整理与 bug 审计（生产代码零外部输入 panic 路径；六个大文件的测试模块拆出为同级 `tests.rs`，测试数量不变）。

## 10. 首轮实机矩阵

| 客户端/环境 | 主要场景 | 状态 |
|---|---|---|
| iPhone / iPad 当前系统 | 发现、PIN、镜像、横竖屏、锁屏恢复 | **部分**：发现 + 进入 PIN 配对已实测；镜像未测试 |
| 一个历史 iOS/iPadOS 版本 | 配对和媒体兼容 | 未测试 |
| Apple Silicon / Intel Mac | 镜像、扩展屏幕、音频 | 未测试 |
| Windows x64 服务端 | 原生媒体依赖 ABI、安装、网络接口 | **部分**：控制面 + 发现已实测；媒体依赖 ABI 未验证 |
| Linux x64 服务端 | 插件、桌面/无头模式 | 未测试 |
| macOS / ARM / BSD / Pi | 平台特性与硬件解码 | 未测试 |
| BLE 适配器 / BleuIO | 广播发现；媒体仍走 IP 网络 | 未测试 |
