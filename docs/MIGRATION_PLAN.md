# 分阶段迁移计划

目标：UxPlay 自身应用、协议、配对、密码处理（包括 PlayFair）、渲染调度和 Python 外围逻辑全部改为 Rust；允许 GStreamer 和必要的系统接口。最终产品不得链接遗留 UxPlay C 核心或依赖原 Python beacon 脚本。

基线：`FDH2/UxPlay@57ea83411d5f7e0b38c5841987439340543f025c`（2026-09-17，1.74 Experimental）。范围和估算依据见研究报告。每阶段独立提交；发现新证据时更新本文件，未验收项目保持未完成。

## 里程碑

| 阶段 | 内容 | 退出标准 | 状态 |
|---|---|---|---|
| S0 | 仓库、许可、workspace、CI、上游来源记录 | 本地 fmt/test/clippy 通过，来源和阶段清晰 | 已完成 |
| S1 | 有界协议基础：镜像帧、NAL、RTP、NTP 表示、HTTP/RTSP 消息 | 拆包/粘包、混合字节序、截断、超限、错误输入测试通过 | 已完成 |
| S2 | 原版运行基线、密码原语、密钥派生、Apple SRP、PlayFair Rust 移植 | 受控 C oracle 差分、合法向量、重新配对/换流测试通过 | 已完成（离线） |
| S3 | 发现、网络控制服务、plist、会话生命周期和访问控制 | 可发现；真实配对/SETUP/RECORD/FLUSH/TEARDOWN；错误连接有界 | 进行中 |
| S4 | 镜像 TCP、音频 UDP、重传、NTP 同步、GStreamer backend | 指定设备 H.264+AAC-ELD、ALAC、长时播放、断线恢复 | 未开始 |
| S5 | H.265、HLS/FCUP/URL 播放、语言/字幕、模式切换 | 与基线逐场景对照，正确控制播放和媒体切换 | 未开始 |
| S6 | MP4、RTP 转发、封面/元数据、CLI/配置完整兼容 | 选项与行为矩阵完成，录制/导出可用 | 未开始 |
| S7 | BLE、AWDL、Windows/Linux/macOS/BSD/ARM、部署 | 约定硬件与平台矩阵验收；正确标明上游已有平台限制 | 未开始 |
| S8 | 综合差分、fuzz、资源与性能、许可和发布 | 不含自有遗留 C/Python；真实设备和长时回归通过 | 未开始 |

S0/S1 是本次启动工作的交付范围。S2 当前已完成标准密码原语、Apple 标签 KDF、Digest 认证、Apple session-key 扩展，以及 2048 位 SHA-1 Apple SRP 服务端 proof 计算；SRP 固定向量已与上游 C oracle 对齐。完整 PlayFair 数据表/变换、FairPlay `/fp-setup` 状态机、真实设备重新配对/换流仍未完成。S2 中原版基线验证必须先于声称协议互操作成功；S1/S2 的离线测试不等于实机验证。后续沿该计划继续，不需要反复确认已授权的代码修改与本地 Git 提交。提交远程、发布和新增能力单独按实际授权处理。

S3 的执行工作包、缺陷修复批次与依赖决策点见 [S3_WORK_PLAN.md](S3_WORK_PLAN.md)。

## 测试策略

1. 对确定性 wire primitives 使用手工构造的固定向量，覆盖边界与故障。
2. 对密码链使用固定上游 C 实现作测试 oracle；产品不链接该 oracle。先准备受控合法样本和随机种子，再做差分。
3. 在控制状态层记录多连接/乱序/重复请求场景；随机数、时间和端口通过接口注入。
4. 实机验收记录客户端型号、系统版本、服务端平台、媒体插件、网络、基线结果和 Rust 结果。
5. 同硬件同配置对比首帧、延迟、同步偏差、丢帧、CPU 与内存；不给没有实测依据的性能承诺。

每阶段必须运行 `cargo fmt --all -- --check`、`cargo test --workspace --all-targets` 和 `cargo clippy --workspace --all-targets -- -D warnings`。公开 API 要有文档，关键输入边界要有针对性测试。CI 结果和本地结果分开报告。

## 首轮实机矩阵（待取得设备信息后细化）

| 客户端/环境 | 主要场景 | 状态 |
|---|---|---|
| iPhone / iPad 当前系统 | 发现、PIN、镜像、横竖屏、锁屏恢复 | 未测试 |
| 一个历史 iOS/iPadOS 版本 | 配对和媒体兼容 | 未测试 |
| Apple Silicon / Intel Mac | 镜像、扩展屏幕、音频 | 未测试 |
| Windows x64 服务端 | 原生媒体依赖 ABI、安装、网络接口 | 未测试 |
| Linux x64 服务端 | 插件、桌面/无头模式 | 未测试 |
| macOS / ARM / BSD / Pi | 平台特性与硬件解码 | 未测试 |
| BLE 适配器 / BleuIO | 广播发现；媒体仍走 IP 网络 | 未测试 |

## 环境记录（2026-09-19）

- Rust 1.98.1，默认 `stable-x86_64-pc-windows-msvc`。
- 检测到 MSYS2 UCRT64 的 GStreamer 1.28.3 与 `avdec_aac`；只验证工具和插件可查询，未验证 AirPlay AAC-ELD 样本。
- 默认 Rust 与当前 GStreamer SDK 来源不同；S4 前选择一致 ABI 并验证链接，不能仅凭 pkg-config 成功就认为媒体 backend 可构建。
- 本机未提供真实设备会话或抓包。需要实机时记录明确的待测步骤，不伪造成功结果。


