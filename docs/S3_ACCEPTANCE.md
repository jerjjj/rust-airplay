# S3 验收材料

对应 [MIGRATION_PLAN.md](MIGRATION_PLAN.md) §3 的 S3 行。S3 的退出标准是：**可发现；真实配对 / SETUP / RECORD / FLUSH / TEARDOWN；错误连接有界**。本文件逐条对照，并列出证据与**仍未验收的部分**。

## 1. 退出标准逐条对照

| 退出标准 | 结论 | 证据 |
|---|---|---|
| **可发现** | ✅ 已达 | `discovery.rs` 广播 `_raop._tcp` / `_airplay._tcp`，TXT 字段逐条对齐上游 `lib/dns_sd/dns_sd.c:238-360` 与 `lib/dnssdint.h:32-49`（含 `pk` = 身份公钥、`sf`/`pw` 随访问控制变化）；单元测试固定每个字段并断言"不多写字段"；本机系统 Bonjour 浏览器 `dns-sd -B _raop._tcp local` 独立确认可见；**iPad 实际发现并连接**（见 §3） |
| **真实配对** | ✅ 已达（PIN 与现代缓存路径均实测） | 真机日志显示 PIN 三步（`/pair-pin-start` → `/pair-setup-pin` 体长 86/347/116）与缓存快路径（`/pair-verify` 68+68）；legacy `/pair-setup` + `/pair-verify` 有单测与 socket 用例；注册表（C4）使二次连接走快路径 |
| **SETUP** | ✅ 已达 | 真机 `SETUP` 体长 755 与 204 的二进制 plist 均答 200；密钥派生（FairPlay 解密 + 已验证会话的 `SHA512(aeskey‖ecdh)[..16]` 混合）有 socket 端到端用例；Digest 口令门（451/401/单次 nonce）有并发用例 |
| **RECORD / FLUSH / TEARDOWN** | 🟡 部分 | 真机实测 `RECORD`（200）与 `TEARDOWN`（200，42 字节 plist）；`FLUSH` 未被真机触发，仅由 `control_spec` 覆盖 envelope 行为；三者都**尚未**作用于媒体（属 S4） |
| **错误连接有界** | ✅ 已达 | 头三项上限 20/64/1024 与 64 KiB 头上限、`Transfer-Encoding`/重复 `Content-Length` 拒绝、plist 深度 32/对象 4096/64 KiB、`MAX_CONNECTIONS` 排队语义、PIN 预算、口令失败预算、属性测试"任何输入不 panic"（含深 bplist 回归护栏） |

结论：**S3 的协议与访问控制目标达成；"媒体语义"部分明确留给 S4**，因此 RECORD/FLUSH/TEARDOWN 标为部分。

## 2. 能力声明（避免误读）

- `RouteHandlers::new(identity)`：`/info`、`/fp-setup`、`/feedback`、`SETUP`（无配对会话时只做 FairPlay 解密，不做 ecdh 混合）；**配对路由一律 501**。
- `RouteHandlers::with_pairing(identity, state)`：在上一行基础上提供 `/pair-pin-start`、`/pair-setup-pin`、`/pair-setup`、`/pair-verify`；`/info` 的 `pk` 与配对身份的 `pk` 强制同源。
- 库**不做**的事：不碰文件系统（身份种子与客户端名单都在 CLI 侧，[ADR-0002](ADR-0002-pairing-identity.md)）、不链接媒体库（[ADR-0003](ADR-0003-media-backend.md)）、不做授权判定（`with_client_registered` 是调用方的谓词，`SessionAuth` 是证据不是权限）。
- 未绑定端口时 `SETUP` 响应**不写** `dataPort`/`controlPort`/`timingPort`——宁可少声明，也不声明没有的服务。

## 3. 真机证据（iPad，2026-10-05）

两份日志：仓库内 [`session.log`](../session.log)（首次会话，带 470 重试）与带毫秒时间戳的会话（输出见提交 `e87bf45`）。

### 3.1 完整请求序列（首次会话，`session.log`）

```
POST /pair-verify    68B  → 200      （新连接，先探测配对）
GET  /info           70B  → 200
POST /pair-pin-start      → 200      （生成一次性 PIN）
POST /pair-setup-pin 86B  → 200      （第 1 步）
POST /pair-setup-pin 347B → 470 !    （第 2 步：proof 与当前 PIN 不符）
-- 客户端换新连接重试 --
POST /pair-verify    68B  → 200
GET  /info           70B  → 200
POST /pair-pin-start      → 200
POST /pair-setup-pin 86B  → 200
POST /pair-setup-pin 347B → 200      （用户输入新 PIN 后通过）
POST /pair-setup-pin 116B → 200      （第 3 步）
POST /pair-verify    68B  → 200      （握手）
POST /pair-verify    68B  → 200      （签名）
POST /fp-setup       16B  → 200
POST /fp-setup       164B → 200
SETUP  rtsp://192.168.1.7/<id>  755B → 200
GET  /info                → 200
RECORD rtsp://192.168.1.7/<id>   → 200
SETUP  rtsp://192.168.1.7/<id>  204B → 200
POST /feedback  ×4        → 200      （每约 2 秒一次）
TEARDOWN rtsp://192.168.1.7/<id> 42B → 200
```

结论：**三步配对体长 86/347/116、`/fp-setup` 16+164、两个 SETUP（755/204）、绝对 RTSP URL target、`User-Agent: AirPlay/980.77.1`、`Active-Remote`/`DACP-ID` 有真实值**——全部与此前的实现假设吻合。

### 3.2 时序（带时间戳会话）：握手 63 ms，"慢"在等媒体

| 相对时刻 | 请求 | 结果 |
|---|---|---|
| 57974 ms | `POST /pair-verify` | 200 |
| 57989 ms | `POST /pair-verify` | 200 |
| 57995 ms | `POST /fp-setup`(16) | 200 |
| 58004 ms | `POST /fp-setup`(164) | 200 |
| **58037 ms** | **`SETUP`(755)** | 200 |
| 58055 ms | `GET /info` | 200 |
| 58070 ms | `RECORD` | 200 |
| 58108 ms | `SETUP`(204) | 200 |
| 60218 / 62245 / 64242 / 66257 ms | `POST /feedback` ×4 | 200（间隔 ≈2.0 s） |

**控制面全程 63 ms（57974→58037）。** 之后客户端每 2 秒发 `/feedback` 拿 200，这是它在**等媒体**：本移植的 SETUP 响应没有媒体端口，音频/镜像会话起不来。用户报告的"连接很慢"由此得到解释——**不是控制面性能问题，而是 S4 未实现的必然表现**。

### 3.3 两个已确认的行为要点

1. **470 是设计行为**：第 2 步 proof 与接收端当前 PIN 不符（iPad 用了缓存的旧 PIN）时答 470，与上游一致；客户端随后自行换新连接重试。**运维提示**：`MAX_PIN_FAILURES = 5` 之后不再发新 PIN，若客户端反复用旧 PIN 会锁到上限，需重启接收端。
2. **缓存配对的快路径确实生效**：第二次会话没有 `/pair-pin-start` 与 `/pair-setup-pin`，直接 `pair-verify` 两次 + `fp-setup` 两次——注册表（`%LOCALAPPDATA%\rust-airplay\clients`）与已验证会话路径在工作。

## 4. 测试与差分证据

| 项 | 数量/结果 | 命令 |
|---|---|---|
| 普通套件 | **165 通过**（单元 105、control 15、plist 6、property 9、引用一致性 2、server 26、CLI 2） | `cargo test --workspace --all-targets` |
| 默认忽略 | 9（7 条上游差分 + 2 条测量） | 同上 |
| **上游差分** | **7/7 通过** | `pwsh -File tools/oracle/run.ps1` |
| 四门禁 | `fmt` / `test` / `clippy -D warnings` / `doc -D warnings` 全绿 | 见 [MIGRATION_PLAN.md](MIGRATION_PLAN.md) §4 |

差分内容：SRP 验证器接受本移植 proof、pair-verify 密钥流、pair-setup nonce 与 GCM、FairPlay setup/handshake，以及控制面吞吐与 SETUP 吞吐测量。

## 5. 真机矩阵

| 客户端/环境 | 场景 | 状态 |
|---|---|---|
| iPad 当前系统 | 发现、PIN 配对、缓存配对、SETUP、RECORD、TEARDOWN | **已实测**（§3） |
| iPad 当前系统 | 镜像画面、音频出声、横竖屏、锁屏恢复 | **未测试**（媒体属 S4） |
| 历史 iOS/iPadOS | 配对与媒体兼容 | 未测试 |
| Mac（Apple Silicon / Intel） | 镜像、扩展屏、音频 | 未测试 |
| Windows x64 服务端 | 控制面 + 发现 + 端口/防火墙 | **已实测**（本机） |
| Windows x64 服务端 | 媒体依赖 ABI | 未测试 |
| Linux x64 / macOS / ARM / BSD / Pi 服务端 | 构建、插件、硬件解码 | 未测试 |
| BLE 适配器 / BleuIO | 广播发现 | 未测试 |

## 6. 仍未验收（不得宣称完成）

1. **媒体**：镜像画面、音频、A/V 同步、长时播放、断线恢复——全部属 S4，未实现。
2. **FLUSH 的真机行为**：未被 iPad 触发，仅有 envelope 级测试。
3. **`GET_PARAMETER` 的 `progress` 查询**：按上游返回空体，未与真机确认。
4. **平台**：macOS/Linux/BSD/ARM 未构建；`mdns-sd` 仅 Windows 实测；oracle 的 `build.ps1` Linux 分支未验证；**CI 从未运行**（无远程仓库）。
5. **环境**：GStreamer 只验证过"工具与插件可查询"，未验证 AirPlay AAC-ELD 样本，也未验证与默认 Rust 工具链的 ABI 一致性——S4-2 前必须解决。
6. **首次会话日志无时间戳**，其"慢"由带时间戳的第二次会话解释；后续归档请一律带时间戳（CLI 现已默认如此）。

## 7. 结论

S3 的协议、配对、发现与访问控制目标**达成并有真机证据**；媒体相关语义（RECORD/FLUSH/TEARDOWN 的实际作用、端口声明）**明确移交 S4**。按项目约定，未实测的项目保持未完成标注。
