# S3 移植工作计划与缺陷修复批次

本文件把 [MIGRATION_PLAN.md](MIGRATION_PLAN.md) 的 S3（发现、网络控制服务、plist、会话生命周期和访问控制）拆成可独立验收的工作包，并把项目分析阶段发现的缺陷纳入同一批次。阶段级权威仍是 MIGRATION_PLAN.md，代码约定仍是 [ARCHITECTURE.md](ARCHITECTURE.md)；本文件只描述执行顺序、出口标准和当前状态。

启动日期：2026-09-20。基线：`FDH2/UxPlay@57ea83411d5f7e0b38c5841987439340543f025c`。上游参考树位于 `upstream/UxPlay`（被根仓库忽略，不进入产品链接）。

## 1. 基线快照

- 工作区：`crates/airplay-core` + `crates/airplay-cli`；`resolver = "3"`、edition 2024、MSRV 1.85、GPL-3.0-or-later、`publish = false`。
- 依赖（airplay-core，12 个）：aes、aes-gcm、cbc、cipher、ctr、ed25519-dalek、md-5、num-bigint、num-traits、sha1、sha2、x25519-dalek。`Cargo.lock` 共 58 包。
- 上游 `upstream/UxPlay/lib` 共 49 个条目（含 `CMakeLists.txt`）。S3 相关的上游实现：`raop.c`、`raop_handlers.h`、`http_handlers.h`、`http_request.c`、`http_response.c`、`httpd.c`、`netutils.c`、`dnssd.c`、`pairing.c`、`sockets.h`、`threads.h`、`compat.c`。
- 每个工作包完成时必须通过：
  ```powershell
  cargo fmt --all -- --check
  cargo test --workspace --all-targets
  cargo clippy --workspace --all-targets -- -D warnings
  $env:RUSTDOCFLAGS='-D warnings'; cargo doc --workspace --no-deps
  ```
- 本轮（缺陷批次）完成后的实测：fmt / clippy / doc 通过，`cargo test --workspace --all-targets` = **43 passed / 0 failed**（缺陷批次前为 30）。

## 2. 约束（继承，不得绕过）

| 来源 | 约束 |
|---|---|
| ARCHITECTURE.md | 自有核心禁 unsafe；外部输入不得触发 unwrap/expect/断言；长度运算检查溢出、先验证再分配、限制可配置；解析不顺带解密或渲染；增量 decoder 明确 consumed/incomplete/EOF/fatal，且错误后不猜下一帧起点；单会话所有权 + 有界消息队列，避免全局大锁；公共接口写用途/约束/错误文档；不为未来功能堆空 crate 或抽象 |
| MIGRATION_PLAN.md | 能力发布只能声明已实现功能；CLI 对等必须保留 UxPlay 的单横杠多字符选项（S6）；本地 Git 提交已授权，推送远程/发布/新增能力需单独授权 |
| THIRD_PARTY_NOTICES.md | 每个 Rust 翻译保留适用上游 SPDX 声明；数据表与实现分别标注来源 |

## 3. 缺陷修复工作包

状态为本次批次结束时的实际状态。

| 编号 | 缺陷 | 处理 | 状态 |
|---|---|---|---|
| D1 | `aes128_gcm16_decrypt` 用 `Key/Nonce/Tag::from_slice`，长度不符即 panic | 改用 `from_exact_iter`，长度错误返回 `CryptoError::InvalidLength`；补 GCM 长度、往返、篡改用例 | 已完成 |
| D2 | `unix_nanos_to_timestamp` 把秒数 `as u32` 静默截断，2036 后回绕且无法报错 | 改为返回 `Result`，超过 NTP era 0 边界返回 `ParseError::Invalid("NTP timestamp outside era 0")`；新增 `NTP_ERA_SECONDS`；去掉 `saturating_*` 掩盖；补边界用例 | 已完成 |
| D3 | `playfair.rs` 生产路径 11 处 `unwrap()` 与 1 处 `expect()` | 全部改为定长索引读取 / `copy_from_slice` / `ok()?`；上游向量测试保持全绿 | 已完成 |
| D4 | `playfair.rs`、`playfair/hand_garble_generated.rs` 缺 SPDX 头 | 补 `LGPL-2.1-or-later`，并在 THIRD_PARTY_NOTICES.md 记录依据 | 已完成 |
| D5 | `parse_request_head` 对头部字节数无上限 | 新增 `DEFAULT_MAX_HEAD`（64 KiB）与 `parse_request_head_with_maximum`，超限返回 `TooLarge`；补 Incomplete/超限/坏 start line/坏 UTF-8/流水线用例 | 已完成 |
| D6 | `MirrorDecoder` 超限后不定位、错误可重复触发；每包 `drain` 造成 O(n²) | 增加显式 fatal（`poisoned`）语义与 `reset()`，改用偏移游标 + 每次 push 一次 drain；补粘包/半包/超限用例 | 已完成 |
| D7 | `AppleSrpServer::respond` 不校验客户端公钥长度 | 空值或超过 2048 位返回 `SrpError::InvalidLength`；补用例（零值仍为 `InvalidValue`） | 已完成 |
| D8 | `FairPlaySession::decrypt` 把 PlayFair 变换失败映射为 `InvalidMode(message[12])` | 新增 `FairPlayError::DecryptionFailed`，区分“变换拒绝握手消息”与“setup 模式非法”；补用例 | 已完成 |
| D9 | 测试覆盖缺口（GCM/X25519/sha512_derive、mirror 粘包、wire 边界） | 上述用例已补；**仍缺** `pairing.rs` 签名加解密往返与 `verify_pair_signature`、`control.rs` 的 `MAX_CONTROL_BODY` 超限/缺 Session 头/`to_bytes` 断言 | 部分完成 |
| D10 | `missing_docs = "warn"` 被 9 个模块的 `#![allow(missing_docs)]` 关闭 | 逐模块删除 allow 并补齐公共 API 文档 | 待做 |
| D11 | `control.rs` 与上游 wire 行为不一致（写死 `SESSION_ID`、`Public` 头、缺 `Audio-Jack-Status`/`Server`、无 501） | 并入 W3，因为要先实现方法集合才能正确声明能力 | 待做（W3） |
| D12 | 无 `tests/` 集成层、无上游差分 harness、无 fuzz/benches | 并入 W7 | 待做 |
| D13 | FFmpeg 是否作为新增外部原生依赖 | 需要用户决策（影响 S4 backend 边界） | 待决策 |
| D14 | 无远程仓库，CI 从未真正运行 | 需要用户决策：配置远程，或在文档中持续标注“仅本地验证” | 待决策 |

## 4. S3 工作包

### W1 上游控制面规格化测试（验收基准）

- 目的：在写网络层之前，把上游控制面的**外部可观测行为**固化成期望值，作为后续重构的回归基准。
- 上游证据（已核对）：
  - 路由表：`upstream/UxPlay/lib/raop.c:406-478`（RTSP 与 HTTP/1.1 的 method+URL 分派），`raop.c:443` 未知方法返回 `501 Not Implemented`；`raop_handlers.h:558` 同样返回 501。
  - 响应状态码：`raop.c:335` `200 OK`；`http_handlers.h:657`、`http_handlers.h:891` `400 Bad Request`；`http_handlers.h:910`、`http_handlers.h:926`、`http_handlers.h:974` `404 Not Found`。
  - 公共头：`raop_handlers.h:596` `Public: SETUP, RECORD, FLUSH, TEARDOWN, OPTIONS, GET_PARAMETER, SET_PARAMETER`；每条 RTSP 响应附 `Audio-Jack-Status: connected; type=digital`（RECORD 除外，`raop.c:338-340`）与 `Server: AirTunes/<GLOBAL_VERSION>`（`raop.c:489`）。
  - 会话标识：`raop.c:268`、`raop.c:297` 读取并保存客户端 `X-Apple-Session-ID`，`raop.c:400-402` 校验一致性。
- 出口：新增表驱动测试（`crates/airplay-core/tests/control_spec.rs`），逐条覆盖上述路由，未实现者必须产生显式的「未实现」结果（不能静默忽略或落到错误分支）；测试同时固定 `Public` 串与响应头集合。
- 依赖：无。写作用域：`crates/airplay-core/tests/`、`crates/airplay-core/src/control.rs`。

### W2 依赖与网络运行时选型（阻塞 W3 及以后）

- 目的：S3 会引入本项目第一批非密码依赖，先决策再动手。
- 需要决策的点与建议：
  1. **plist**：上游直接用 libplist（`#include "plist/plist.h"`，见 `raop_rtp_mirror.c:35`），同时用二进制与 XML 两种格式（`raop.c:370`、`raop.c:518`、`raop_handlers.h:72`/`246`/`302`/`347`/`384`/`426`、`http_handlers.h:95`/`201`/`297`/`449`/`515`/`706`）。**建议**引入 `plist` crate（MIT，需确认 MSRV 与 GPL 兼容），自研只覆盖需要的节点类型作为后备。
  2. **DNS-SD**：上游 `dnssd.c`/`dnssd.h`/`dnssdint.h` 基于 Bonjour/libdns_sd。**建议**引入 `mdns-sd`（纯 Rust，便于三平台 CI），而不是绑定系统 Bonjour/Avahi。
  3. **网络运行时**：**建议** `tokio`（`rt-multi-thread` + `net`），因为 S3/S4 同时存在 TCP 控制连接、镜像 TCP、音频 UDP 与 NTP 定时器；按「单会话所有权 + 有界队列」用 `mpsc`（有界）+ task 落所有权。备选方案是 `std::net` + 自管非阻塞/事件循环 + `socket2`（需要 `SO_REUSEADDR`、`TCP_NODELAY` 等上游设置）。
  4. 依赖许可必须与 GPL-3.0-or-later 兼容，并在 THIRD_PARTY_NOTICES.md 记录。
- 出口：`docs/ADR-0001-network-stack.md` 记录选型、被否方案与理由、MSRV/许可核对结果；`Cargo.toml` 依赖落地（或明确不引入）；任何新依赖都要有实际用途，不得为未来功能预置。

### W3 控制面重构：无状态路由 + 有状态会话

- 目的：把 `control.rs` 从「状态键控的 `advance()`」改成「无状态 URL 路由 + 有状态会话对象」，否则接入 `/info`、`/pair-setup` 时必须重写。
- 内容：
  - `head.target` 参与路由（当前 `wire.rs` 已解析但从未使用）。
  - 方法集合与 `Public` 头按 W1 的期望值修正；`GET_PARAMETER`/`SET_PARAMETER` 有明确行为。
  - `X-Apple-Session-ID` 取代写死的 `Session` 常量。
  - 定义 `ControlError`/`RouteError` → HTTP 状态码的映射（未实现方法 501，畸形请求 400，未知 URL 404），并写入文档与测试。
  - 每条 RTSP 响应补 `Audio-Jack-Status` 与 `Server` 头（按 W1 的例外规则）。
- 出口：W1 的规格化测试全绿；状态迁移语义与上游一致（OPTIONS 不因状态被拒）；`Err`→状态码映射有测试。
- 依赖：W1。写作用域：`crates/airplay-core/src/control.rs`（独占）。

### W4 plist 编解码

- 目的：解析上游客户端实际发送的二进制/XML plist，并构造 `/info`、SETUP 响应。
- 出口：`/info` 请求与 SETUP 请求可解析；`/info` 响应可构造；往返测试；如能取得真实抓包则加样本（无抓包时明确标注为合成样本）。
- 依赖：W2。写作用域：`crates/airplay-core/src/plist.rs`（新建）。

### W5 网络控制服务

- 目的：把 W3/W4 接到真实 socket 上，完成配对流程的服务端部分。
- 内容：TCP 监听与连接上限；RTSP/HTTP 请求循环（复用 `wire.rs` 的 `consumed` 语义处理流水线与半包）；`/info`、`/fp-setup`、`/pair-pin-start`、`/pair-setup-pin`、`/pair-setup`、`/pair-verify`、`POST /feedback`；错误连接有界（请求头/体上限、空闲超时、连接数上限）。
- 出口：本机可用 `curl`/测试客户端走通 `OPTIONS` → `/info` → `/fp-setup` → `/pair-setup` 的部分流程；超限请求被拒绝且进程不 panic。
- 依赖：W3 + W4。写作用域：`crates/airplay-core/src/server/`（新建）。

### W6 DNS-SD 广告与访问控制

- 目的：让服务可被发现（`_airplay._tcp`、`_raop._tcp`），并对 TXT 能力位与访问控制有明确实现。
- 出口：本机 `dns-sd -B`/`avahi-browse` 可查询到实例；TXT 字段与上游 `dnssd.c` 对齐；访问控制（允许/拒绝、`Active-Remote`/`DACP-ID` 传递）有测试。
- 依赖：W2。写作用域：`crates/airplay-core/src/discovery.rs`（新建）。

### W7 测试基建：集成层、上游差分 oracle、fuzz

- 目的：落实 MIGRATION_PLAN 测试策略第 2–5 条。
- 内容：`crates/airplay-core/tests/` 集成层；受控上游 C oracle harness（仅测试构建，不链接进产品）；`proptest` 或 `cargo-fuzz` 目标覆盖 wire/mirror/plist 解析；控制层多连接/乱序/重复请求场景（随机数与时间注入）。
- 出口：oracle harness 可复现至少 SRP 与 PlayFair 的固定对照；fuzz 目标能跑满一轮无 panic。
- 依赖：可与 W3–W6 并行，但 W5 之后应覆盖服务器路径。

### W8 S3 验收与文档

- 出口：MIGRATION_PLAN S3 退出标准逐条对照（可发现 / 真实配对→SETUP→RECORD→FLUSH→TEARDOWN / 错误连接有界）；实机矩阵更新，无设备时明确写「未测试」；本文件状态表更新。

## 5. 波次与并行

| 波次 | 内容 | 说明 |
|---|---|---|
| 1 | D1–D8 | 本次完成，独立于 S3 设计决策 |
| 2 | D9 剩余、D10、W1、W2 | 互相独立，可并行 |
| 3 | W3 → W4 → W5 | 严格顺序：W5 需要路由与 plist |
| 4 | W6、W7、W8 | W6/W7 可并行，W8 收尾 |

写作用域互不重叠：W3 独占 `control.rs`，W4 新建 `plist.rs`，W5 新建 `server/`，W6 新建 `discovery.rs`，W7 只碰 `tests/` 与 fuzz 目标。

## 6. 需要用户决策

1. **FFmpeg**：是否接受它作为新增外部原生依赖（影响 S4 播放后端边界，见 `research/ffmpeg-output-feasibility.zh-CN.md`）。
2. **新依赖**：是否接受 `plist` + `mdns-sd`，以及网络层选 `tokio` 还是 `std` + `socket2`（W2 会给出 ADR，但最终取舍需要你确认）。
3. **远程仓库**：是否配置远程让 CI 真正运行；否则继续标注「仅本地验证」。
4. **真机**：是否有可用 iPhone/iPad 用于 S3 配对验收；没有的话 S3 只能声明「本机可达」，不能声明互操作成功。

## 7. 进度记录

| 日期 | 工作包 | 提交 | 说明 |
|---|---|---|---|
| 2026-09-20 | D1–D8 | `3fc20c0` `95fbf37` `c9467ca` `323ce83` `7888a64` `5a97d8b` | 测试 30 → 43，fmt/test/clippy/doc 全绿 |

提交与工作包对应：D1 `3fc20c0`；D2 `95fbf37`；D5 `c9467ca`；D6 `323ce83`；D3/D4 `7888a64`；D7/D8 `5a97d8b`。
