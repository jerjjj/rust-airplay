# S3 移植工作计划与缺陷修复批次

本文件把 [MIGRATION_PLAN.md](MIGRATION_PLAN.md) 的 S3（发现、网络控制服务、plist、会话生命周期和访问控制）拆成可独立验收的工作包，并把项目分析阶段发现的缺陷纳入同一批次。阶段级权威仍是 MIGRATION_PLAN.md，代码约定仍是 [ARCHITECTURE.md](ARCHITECTURE.md)；本文件只描述执行顺序、出口标准和当前状态。

启动日期：2026-09-20。基线：`FDH2/UxPlay@57ea83411d5f7e0b38c5841987439340543f025c`。上游参考树位于 `upstream/UxPlay`（被根仓库忽略，不进入产品链接）。

## 0. 上游基线核对（2026-10-05 复核）

上游已前进到 `origin/master = 3dbf7ceee65932154e85a2f83963d53520a799fa`（2026-10-05，自基线的 `57ea8341` 起 22 个提交）。逐文件判定结果：

| 上游改动 | 我们对应的实现 | 判定 |
|---|---|---|
| `lib/httpd.c`：新增 `timeout_limit = 60` 的"死连接"清理；把连接建立时的 8 字节 peek 阻塞循环改成按连接的 `staging[8]` 累积 | `server/mod.rs`（`idle_timeout`）、`server/connection.rs`（分帧循环）、`server/mod.rs` 的"差异"小节 | **受影响**：空闲超时不再是"上游没有、我们自定"，已把默认值对齐为 60 s 并按硬超时执行（上游只在 socket 再次可读时检查该年龄，且移除连接后漏了 `continue`，两处都不照搬）。peek 重建只影响我们尚未实现的反向连接（PTTH）识别 |
| `lib/http_request.c`：`on_url`/`on_header_field`/`on_header_value`/`on_body` 的 buffer 增长与初始化修复 | `wire.rs`（有界解析）、`control.rs`（`consumed`/`Content-Length`/分帧） | **不受影响**：改动是 llhttp 回调的内存管理（realloc 累积、初始化），没有新增或改变请求长度上限、`Content-Length` 语义或错误处理；我们复刻的可观测行为不变 |
| `lib/raop_handlers.h`：只改 `raop_handler_set_parameter`（volume/progress 的 `conn->raop_rtp` 判空与 `free`） | 尚未移植（属 W5c/S4） | **不受影响**：`/info`、`/fp-setup`、`/pair-*`、`/feedback`、`OPTIONS`/`RECORD`/`FLUSH`/`TEARDOWN` 处理函数一字未改 |
| `lib/raop.c`（空格 + 新增 `get_codec_string`）、`lib/raop.h`（HLS 编解码枚举、`get_custom_profile` 回调、playlist 辅助声明） | 控制面与配对无关 | **不受影响**：属 HLS/视频（S4）范围 |
| `lib/dns_sd/dns_sd.c`（+62）、`lib/dnssd.h`（+12）：Windows 支持与哑 DNS_SD 回调 | W6 尚未开始 | **纳入 W6 输入**：确定 TXT 字段与跨平台回调形状时要按新的 `dnssd.h` 核对 |
| `lib/raop_ntp.c`/`.h`（+57）：`sync_delay`、允许 `recv_time_kernel = NULL`、清理 | `ntp.rs`（只有解析原语） | **纳入 S4 输入**：`raop_ntp` 的同步语义变了，移植 NTP 会话时要按新签名与行为 |
| `lib/airplay_video.c`、`raop_rtp_mirror.c`、`uxplay.cpp`（+355：音量一致性、playlist 过滤、Windows 支持、内存泄漏修复） | 未移植（S4/CLI） | **不受影响**（本轮范围外），但 CLI 规划要参考新的 `uxplay.cpp` 选项 |
| `lib/pairing.c`、`lib/srp.c`、`lib/crypto.c` | W5b 已移植 | **未改动**：上游至今没有修掉"第 3 步不校验第 2 步、用全零会话密钥完成配对"的缺陷，我们对齐到 `3dbf7cee` 后仍然选择偏离（见 W5b 小节） |

结论：S3 已移植的可观测行为在上游 `3dbf7cee` 下**只有空闲超时一处需要对齐**（已改）；其余差异都落在尚未移植的区域，已登记为 W6/S4 的输入。

**引用约定与失效审计**：本仓库的行号引用有两种形式——老引用写成 `upstream/UxPlay/lib/x.c:1-2`（按基线 `57ea8341` 解释），新引用一律写成 `upstream/UxPlay@<hash>:lib/x.c:1-2`。本次逐文件审计了哪些老引用在 `3dbf7cee` 下仍然成立：`httpd.c` 的 8 处已全部改为带 hash 的新行号；`raop.c` 的改动只发生在第 94 行（空格）与第 885 行之后（新增 `get_codec_string`），因此我们引用的 `raop.c` 行号（均 < 885）仍成立；`raop_handlers.h` 的唯一 hunk 在第 1141 行之后，我们引用的行号（均 < 1141）仍成立；`pairing.c`、`srp.c`、`crypto.c`、`http_response.c`、`global.h`、`dnssdint.h`、`playfair/*` 未改动；`http_handlers.h` 有改动但我们只按文件名引用、不带行号。带行号的老引用因此**不需要批量重写**，但新增引用必须带 hash。

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
- 实测：fmt / clippy / doc 通过；`cargo test --workspace --all-targets` = **58 passed / 0 failed**，另有 `tests/control_spec.rs` 的 **15 passed** 与 `tests/plist_spec.rs` 的 **5 passed**（缺陷批次前只有 30 个内联单元测试，没有集成测试）。

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
| D9 | 测试覆盖缺口（GCM/X25519/sha512_derive、mirror 粘包、wire 边界、control 边界） | 上述用例已补；`control.rs` 的头部/体上限、`Ignored` 语义、默认 200/501/431、`Public` 与响应头集合、`to_bytes` 的 `Content-Length` 规则已由单元测试与 `tests/control_spec.rs` 覆盖；**仍缺** `pairing.rs` 签名加解密往返与 `verify_pair_signature` | 部分完成 |
| D10 | `missing_docs = "warn"` 被 9 个模块的 `#![allow(missing_docs)]` 关闭 | `control.rs` 已随 W3 去掉 allow 并补齐全部公共 API 文档；其余 8 个模块（`crypto.rs`、`fairplay.rs`、`mirror.rs`、`pairing.rs`、`ntp.rs`、`wire.rs`、`srp.rs`、`rtp.rs`）待逐模块删除 allow 并补齐文档 | 部分完成 |
| D11 | `control.rs` 与上游 wire 行为不一致（写死 `SESSION_ID`、`Public` 头、缺 `Audio-Jack-Status`/`Server`、无 501） | 全部闭合：`Public`/`Server`/`Audio-Jack-Status`/501/上游默认 200 随 W1 落地；`X-Apple-Session-ID`、可选 `CSeq`、免 `ANNOUNCE`、协议回显、`Audio-Latency`、`Connection: close` 随 W3 落地；固定 `SESSION_ID` 已删除 | 已完成 |
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
- 状态：**已完成**（`c07cb2b`）。交付：`control.rs` 新增 `Route`/`RequestFacts`/`Dispatch`/`dispatch()`（逐条对应上游路由，含 `raop.c:270` 的 HLS 规则）与 `SERVER_HEADER_VALUE`/`AUDIO_JACK_STATUS`/`AUDIO_JACK_STATUS_RECORD`/`PUBLIC_METHODS`；未移植路由与未知 RTSP 方法返回 501，未匹配 URL 返回上游默认的 200 空响应；`tests/control_spec.rs` 用 7 个用例固定 40 余条路由判定与响应头契约。
- 核对后新增的上游事实：`GLOBAL_VERSION` 是 `220.68`（`upstream/UxPlay/lib/global.h:22`），所以 `Server` 头保持该 wire 身份；上游**没有** ANNOUNCE 路由（整个 `lib/` 无匹配），因此 `ANNOUNCE` 是本移植的扩展且不进 `Public`；`RECORD` 用 `connected; type=analog` 并附 `Audio-Latency`（`raop_handlers.h:1225-1226`，`Audio-Latency` 的计算与默认值随 W3 落地）；`ControlResponse::to_bytes` 当时写死 `RTSP/1.0`（W3 已改为回显请求协议）。

### W2 依赖与网络运行时选型（阻塞 W3 及以后）

- 目的：S3 会引入本项目第一批非密码依赖，先决策再动手。
- 需要决策的点与建议：
  1. **plist**：上游直接用 libplist（`#include "plist/plist.h"`，见 `raop_rtp_mirror.c:35`），同时用二进制与 XML 两种格式（`raop.c:370`、`raop.c:518`、`raop_handlers.h:72`/`246`/`302`/`347`/`384`/`426`、`http_handlers.h:95`/`201`/`297`/`449`/`515`/`706`）。**建议**引入 `plist` crate（MIT，需确认 MSRV 与 GPL 兼容），自研只覆盖需要的节点类型作为后备。
  2. **DNS-SD**：上游 `dnssd.c`/`dnssd.h`/`dnssdint.h` 基于 Bonjour/libdns_sd。**建议**引入 `mdns-sd`（纯 Rust，便于三平台 CI），而不是绑定系统 Bonjour/Avahi。
  3. **网络运行时**：**建议** `tokio`（`rt-multi-thread` + `net`），因为 S3/S4 同时存在 TCP 控制连接、镜像 TCP、音频 UDP 与 NTP 定时器；按「单会话所有权 + 有界队列」用 `mpsc`（有界）+ task 落所有权。备选方案是 `std::net` + 自管非阻塞/事件循环 + `socket2`（需要 `SO_REUSEADDR`、`TCP_NODELAY` 等上游设置）。
  4. 依赖许可必须与 GPL-3.0-or-later 兼容，并在 THIRD_PARTY_NOTICES.md 记录。
- 出口：`docs/ADR-0001-network-stack.md` 记录选型、被否方案与理由、MSRV/许可核对结果；`Cargo.toml` 依赖落地（或明确不引入）；任何新依赖都要有实际用途，不得为未来功能预置。
- 状态：**已完成**（[ADR-0001](ADR-0001-network-stack.md)）。决策：tokio（`rt-multi-thread`/`net`/`time`/`sync`/`io-util`/`macros`）+ `plist` + `mdns-sd`；依赖在首个使用它的工作包落地（W4 `plist`、W5 `tokio`、W6 `mdns-sd`），不提前写入 `Cargo.toml`；FFmpeg 推迟到 S4 前单独决策。证据：tokio 1.53.1 / MIT / MSRV 1.71；plist 1.8.0 / MIT / MSRV 1.81（1.10.1 要 1.88，`resolver = "3"` 的 MSRV 感知解析自动回退）；mdns-sd 0.21.4 / Apache-2.0 OR MIT / MSRV 1.71；socket2 0.6.5 作为降级路径。

### W3 控制面重构：无状态路由 + 有状态会话

- 目的：把 `control.rs` 从「状态键控的 `advance()`」改成「无状态 URL 路由 + 有状态会话对象」，否则接入 `/info`、`/pair-setup` 时必须重写。
- 内容（W1 已完成：路由判定、`Public`、`Server`、`Audio-Jack-Status`、501 与上游默认 200）：
  - `X-Apple-Session-ID` 取代写死的 `Session` 常量（`upstream/UxPlay/lib/raop.c:268-297`）。
  - `CSeq` 改为可选：缺省时不回 `CSeq`/`Audio-Jack-Status`（`raop.c:338`、`raop.c:490-492`），并按 `raop.c:263-270` 处理「无 CSeq 且未启用 HLS」的请求。
  - 新连接可直接 `SETUP`，取消对 `ANNOUNCE` 的依赖（上游没有该路由）；`OPTIONS` 不再因状态被拒。
  - `ControlResponse` 保存请求协议并原样回显（HTTP/1.1 对端目前会收到 `RTSP/1.0`）。
  - `GET_PARAMETER`/`SET_PARAMETER` 有明确行为；`RECORD` 补 `Audio-Latency`（依赖音频延迟配置）。
  - 明确 `ControlError` → 状态码映射（未实现方法 501，畸形请求 400，未知 URL 404；证据 `http_handlers.h:657`/`:891`/`:910`/`:926`/`:974`）。
  - `tests/control_spec.rs` 顶部列出的「已知分歧」必须在 W3 结束后逐条改成正向断言。
- 出口：W1 的规格化测试全绿；状态迁移语义与上游一致（OPTIONS 不因状态被拒）；`Err`→状态码映射有测试。
- 依赖：W1。写作用域：`crates/airplay-core/src/control.rs`（独占）。
- 状态：**已完成**（本次提交）。交付与决策：
  - `ControlSession::handle(&mut self, input) -> Result<ControlOutcome, ControlError>`，`ControlOutcome::{Respond, Ignored}` 显式表达上游「什么都不发」的两条路径（`raop.c:247-250` 不可用的 `CSeq`、`raop.c:263-266` 无 `CSeq` 且未启用 HLS 且不是 BLE 信标响应）；`raop.c:257-260` 的 BLE 规则与 `raop.c:270` 的 HLS 规则分别是 `RequestFacts::is_ble_beacon_response` 与 `is_hls_request`。
  - 固定 `SESSION_ID` 与 `Session` 头已删除（上游从不发送也不读取该头）；连接在首次见到 `X-Apple-Session-ID` 时绑定，之后不同值返回 `ControlError::SessionMismatch`（上游 `raop.c:400-402` 用 `assert`，会 abort，移植改为 400）。
  - `CSeq` 改为可选且按上游归一化（非负整数，回写十进制）；缺省时不回 `CSeq`/`Audio-Jack-Status`。
  - `ControlResponse` 新增 `protocol` 字段并原样回显请求协议（`http_response.c:71-91`）；`to_bytes` 逐字移植 `http_response.c:131-170`：有 body 才写 `Content-Length`，无 body 但有 `Content-Type` 时写 `Content-Length: 0`，否则完全不写。
  - 状态改为单向宽松的 `observe()`：`ControlState` 仅供参考，任何请求都不会因状态被拒（上游路由层无状态门）；`ANNOUNCE` 仍是扩展。新增 `AUDIO_SAMPLE_RATE`/`DEFAULT_AUDIO_DELAY_MICROS`/`MAX_AUDIO_DELAY_MICROS`/`audio_latency_samples()`，`RECORD` 附 `Audio-Latency`（默认 250 ms → `11025`）与 analog 状态，`TEARDOWN` 附 `Connection: close`。
  - `ControlError` → 状态码：头部超限 431（裸响应，`raop.c:232-238`）、`Parse`/`BodyTooLarge`/`SessionMismatch` 400；所有拒绝都是裸状态行，不回显任何请求字段。
  - 分歧说明：`GET_PARAMETER`/`SET_PARAMETER`（上游 451/200+`text/parameters`）与其余未移植路由仍答 501；未匹配 URL 仍回上游默认 200 空响应（上游只在具体 handler 内返回 404，如 `http_handlers.h:910`/`:926`/`:974`）；`SETUP`/`RECORD`/`FLUSH`/`TEARDOWN` 只实现 RTSP 信封，plist/密钥/RTP 端口处理归 W5。
  - `tests/control_spec.rs` 重写为 15 个正向断言（路由表 40 余条、无 `CSeq` 的静默丢弃、BLE/HLS 分支、会话绑定与不匹配、协议回显、`Content-Length` 规则、`Audio-Latency`、`Connection: close`、431/400 裸响应），顶部「已知分歧」清单已删除；`control.rs` 的 `#![allow(missing_docs)]` 同时去掉（D10 的第一个模块），本次测试 58 单元 + 15 + 5 集成。

### W4 plist 编解码

- 目的：解析上游客户端实际发送的二进制/XML plist，并构造 `/info`、SETUP 响应。
- 内容：`crates/airplay-core/src/plist.rs` 提供 `Plist`（先查大小上限再解析、类型化读取返回 `PlistError` 而不是 panic、`encode_binary`/`encode_xml`）与 `dictionary()` 构造辅助；依赖 `plist 1.8.0`（`default-features = false`，可选 `serde` 不参与构建），已在 `lib.rs` 注册 `pub mod plist`。
- 出口（已满足）：`/info` 请求（`qualifier` 字符串数组）、SETUP 请求（`ekey`/`eiv`/`deviceID`/`model`/`name`/`streams[type=96|110]`）与 SETUP 响应（`timingPort`/`eventPort`/`streams`）都有夹具验证。夹具由 `tools/plist_fixtures.py` 用 CPython `plistlib` 生成（`crates/airplay-core/tests/fixtures/*.bplist`），因此解码方向对的是**独立实现**；反向由 `cargo test -- --nocapture` 把本 crate 的编码结果写到 `$CARGO_TARGET_TMPDIR`，再用 `python tools/plist_fixtures.py --verify` 交给 `plistlib` 读回，两边值一致。**没有真实抓包，全部夹具是合成样本**；`streamConnectionID` 故意取 `> i64::MAX` 的值以固定无符号 64 位路径。
- 依赖：W2。写作用域：`crates/airplay-core/src/plist.rs`（新建）、`crates/airplay-core/tests/plist_spec.rs` + `tests/fixtures/`、`tools/plist_fixtures.py`。

### W5 网络控制服务

- 目的：把 W3/W4 接到真实 socket 上，完成配对流程的服务端部分。
- 内容：TCP 监听与连接上限；RTSP/HTTP 请求循环（复用 `wire.rs` 的 `consumed` 语义处理流水线与半包）；`/info`、`/fp-setup`、`/pair-pin-start`、`/pair-setup-pin`、`/pair-setup`、`/pair-verify`、`POST /feedback`；错误连接有界（请求头/体上限、空闲超时、连接数上限）。
- 出口：本机可用 `curl`/测试客户端走通 `OPTIONS` → `/info` → `/fp-setup` → `/pair-setup` 的部分流程；超限请求被拒绝且进程不 panic。
- 依赖：W3 + W4。写作用域：`crates/airplay-core/src/server/`（新建）。
- 状态：**拆分为 W5a（已完成）与 W5b（待做）**。
  - **W5a 已完成**（`322e01b`）：`crates/airplay-core/src/server/` 三个模块——`mod.rs`（`ServerConfig`/`Server::bind`/`Server::run`，accept 前先取连接许可，满了就在 backlog 等待，对应 `httpd.c:389-402`）、`connection.rs`（按 `control::request_length` 分帧的连接循环，未完成请求不能刷新空闲超时，无法成帧的请求答一次后关闭）、`handlers.rs`（`DeviceIdentity` + 上游默认值，`/feedback`、`/info`、`/fp-setup` 三个路由）。同时 `control.rs` 增加 `RouteHandler`/`RouteReply`/`RouteRequest`/`NoHandler` 与 `ControlSession::handle_with`（信封与前置条件仍集中在一处），`request_length` 负责帧长。`tests/server_spec.rs` 用 9 个真实 socket 用例覆盖 /info 往返、同一次写入的两个请求（**该用例发现并修掉了「缓冲区里已有完整请求却仍阻塞在 read」的真实缺陷**）、分片请求、80 KiB 头的 431、畸形请求的 400、TEARDOWN 的 `Connection: close`、feedback 回调与 `max_connections=1` 的连接上限。验收：67 单元 + 15 + 5 + 9 集成全绿，fmt/clippy/doc 全绿。
  - **W5b 已完成**（本次提交）：四条配对路由落地。新增 `crates/airplay-core/src/server/pairing.rs`（`PairingState`＝接收端共享的长期身份、PIN 策略、失败计数、注册谓词、随机源；`PairingSession`＝每连接的 `Initial → Setup → Handshake → Finished` 状态机与三步 `/pair-setup-pin`、legacy `/pair-setup`、两步 `/pair-verify`）。写作用域按评审结论放宽到：`crypto.rs`（新增 `aes128_gcm16_encrypt`、`ed25519_public_key`）、`pairing.rs`（**修掉 `/pair-verify` 第二步的 CTR 偏移**：客户端签名用密钥流第 2 段 64 字节，原实现从 0 开始；把 `pair_setup_gcm_iv` 换成显式 `pair_setup_gcm_nonces`（解密 +1／加密 +2）并修正文档，清除 D10 里该模块的 `missing_docs` allow）、`srp.rs`（新增 `AppleSrpServer::server_public()` 与仅测试用的确定性客户端 `test_client`）、`control.rs`（`RouteReply::closing` 与 `ControlResponse::close_after_reply`，对应上游 `http_response_set_disconnect`）、`server/`、`tests/server_spec.rs`。
  - 评审（技术方案评审团：架构／安全／验收）给出 1 项 Critical 与 4 项阻断项，主理人逐条独立核对上游原文后处理：
    - **Critical（上游缺陷，未照搬）**：上游第 3 步不校验第 2 步是否通过，GCM 密钥取自 `calloc` 出的**全零**会话密钥（`pairing.c:513`、`:587`、`:603`），任意对端 2 个请求即可在不知道 PIN 的情况下让接收端装上攻击者公钥。本移植把会话密钥建模为 `Option`（仅在 proof 校验通过后写入），第 3 步在没有它时答 `470`，并有单测（断言 `client_public_key` 未被写入、连接仍可正常配对）与真实 socket 用例。
    - **阻断项**：乱序/越界（上游 `pairing.c:560`、`:603` 空指针解引用；`raop_handlers.h:366`、`:408-409` 越界读）→ 全部精确校验长度并答 `470`；CTR 偏移（见上）；缺 GCM 加密原语（见上）；无有效 PIN 时必须拒绝（上游隐式 `"0000"`）→ 本移植 `PinMode::Disabled` 与「未生成一次性 PIN」一律 `470`。
    - **有意偏离清单**（每条都有测试与模块文档）：一次性 PIN 仅在配对成功后消费、重复 `/pair-pin-start` 不轮换已显示的 PIN、`MAX_PIN_FAILURES = 5` 次错误后作废（上游无失败计数、且任意对端可作废 PIN）、未生成 PIN 时不回退到 `"0000"`、`/pair-setup-pin` 只接受二进制 plist（上游 `plist_from_bin` 同义）、长度为 0 的共享密钥拒绝、每连接状态不匹配答 `470` 而非崩溃。
    - **未采纳的一条建议**：安全角色建议把上游「默认 200 空响应」的所有分支统一改成 `470`。主理人不采纳——`/pair-verify` 在未 setup 且未配 PIN 时的 200 空是客户端回退到 legacy `/pair-setup` 的**协议信号**，`method != "pin"` 与 `/pair-setup` 长度不符同理；改成 470 会破坏真实客户端的回退路径。仅在上游本身返回 470 的分支与上述安全阻断点上使用 470。
  - 验收：**83 单元 + 15 control_spec + 5 plist_spec + 15 server_spec = 118 全绿**；fmt/clippy/doc 全绿。配对向量的独立性：`pairing.rs` 的 CTR/GCM 向量由验收角色用纯 Python 独立实现生成，主理人再用 Node `crypto` 复算一致后才写入测试（仓库代码不参与生成）；`/pair-verify` 第二步的偏移错误由此可被测试报红。真机项（客户端是否带 `CSeq`、三阶段 plist 真实形状、`+1/+2` nonce 与 64 字节偏移是否被真实 iOS 接受、重启换钥后的表现）**全部未验证**，已列入 S3 验收的未测试清单。
  - W5b 遗留的未验证/待定项（不得在 S3 验收中当作已通过）：① SRP 服务端公钥 `B` 用最小编码（与上游 `srp.c:730` 一致），若真实客户端按 RFC5054 左填充到 256 字节，则约 1/256 的概率会因 `M` 不一致而失败——需要真机或抓包确认；② 一次性 PIN 在「配对成功才消费」的语义下，两个都知道 PIN 的对端可在第一个成功前的窗口内各自完成配对（已在模块文档记录，未加预留机制）；③ `plist` crate 对深嵌套 bplist 的递归深度未验证（< 64 KiB 的深嵌套输入是否会导致栈溢出需要单独测）；④ 空闲超时与「用户读 PIN 再输入」的交互：默认已按上游新加的 `timeout_limit = 60`（`upstream/UxPlay@3dbf7cee:lib/httpd.c:377`）对齐为 60 s，超时后需重新 `/pair-pin-start`（PIN 语义已保证不因此失效）；上游只在 socket 再次可读时才检查该年龄，本移植按硬超时执行（更强的同义实现）。

### W5c SETUP 与会话密钥（待做，安全评审已给出开工前置）

- 范围：SETUP 的 plist 流参数解析与会话建立——`streams[]`、`ekey`/`eiv` 解密、`timingPort`/`eventPort`、`ecdh_secret → aeskey = SHA256(aeskey16‖ecdh_secret32)[0..16]`（`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:826-860`）、密码 Digest 401（`:650-748`）、注册表落地、`SET_PARAMETER` 的新语义（`:1141-1168`）。
- **开工前置（安全评审的阻断项，必须先满足再写处理器）**：
  - **A. 会话材料交付**：只有完成 `/pair-verify` 第二步（`status == Finished`）的连接才允许把 ECDH 共享秘密/客户端身份交付给 SETUP；交付接口必须是消费型（`take_*`），重复交付失败；不得跨连接复用；**`/pair-verify` 成功不等于授权**（授权门是 SETUP 的 Digest 401）；交接面要有显式认证状态枚举（未认证 / 仅 legacy / PIN 已验证 / 已注册），SETUP 按它分支。
  - **SETUP 已完成（本次）**：`Route::Setup` 从 `is_ported()` 移出，改由 `RouteHandlers::setup` 服务（`control.rs` 的 `Dispatch::NotImplemented` 分支现在只在 handler **真的**服务了该路由时才推进生命周期状态，501 不再推进）。处理器按上游 `raop_handlers.h:785-1060`：要求二进制 plist（否则 400，上游会带着空请求继续，这是有意收窄）、`eiv` 恰好 16 字节、`ekey` 恰好 72 字节（上游不查长度）、经 `FairPlaySession::decrypt` 解出 16 字节音频密钥、若本连接已完成 `/pair-verify` 则按 `SHA512(aeskey‖ecdh_secret)[..16]` 混合（`raop_handlers.h:848-854`，注意上游用的是 SHA-512 而非 SHA-256）、`streams` ≤4 且 `type` 仅 96/110、未知 type 按上游"switch 无 default"不回应；响应为二进制 plist，含 `timingPort`/`eventPort` 与每个 stream 的 `dataPort`(+`controlPort`)，**未绑定的端口不写**（上游写它已绑定的端口值；S4 之前我们没有端口，写占位值等于声明不存在的能力）。`SetupPorts` 由调用方配置，`setup_callback` 把 `SetupRequest{audio_key, audio_iv, streams, client_identity, authentication}` 交给会话层（对应上游把密钥直接交给 `raop_rtp_init`）。**仍未做**：Digest 401 分支（`:650-748`）、NTP/RTP/镜像会话（`:910-929`，属 S4）、以及 ecdh 混合路径的端到端用例（目前只有交付契约的单测，混合分支缺一个"已 verify + SETUP"的 socket 用例）。
  - **Digest 401 已完成（本次）**：口令路径按上游 `:649-748` 移植，并落实安全评审的 B 组要求——`PairingState::with_password`（接收端级配置）、**挑战发生在解析 plist 之前**（上游先解析再挑战，这是有意的加固）、`Authorization: Digest` 由只读解析器 `pairing::parse_digest_authorization` 处理（上游用 `strstr` 加写 NUL，缺字段时空指针算术）、`digest_auth_response` 复用既有实现、响应值用 `pairing::constant_time_eq` 常量时间比较、**nonce 由本移植自己的随机源生成 16 字节转 32 位 hex**（上游 `pairing_session_make_nonce` 哈希的是 `ed_ours` 指针，熵来自堆地址）、nonce 单次使用（无论是否匹配都作废）、认证状态按连接粘滞、失败计数在共享状态上且有 `MAX_PASSWORD_FAILURES` 上限。socket 用例覆盖：无凭据 → 401 + `Digest realm="raop", nonce="…"`（32 字符）→ 用同一组字段算出的摘要 → 200 → 同连接再发不再挑战 → 另一连接用错口令 → 401 且共享失败计数为 1。
  - **ecdh 混合路径的端到端用例已完成（本次）**：FairPlay 握手 → legacy `/pair-setup` → `/pair-verify` 两步（客户端签名用密钥流第 2 段）→ SETUP，断言回调收到的 `audio_key == SHA512(fairplay_key‖ecdh_secret)[..16]`、`authentication == Verified`、`client_identity` 为客户端公钥；**同一连接第二次 SETUP 只能得到未混合的 FairPlay 密钥**（材料一次性交付，A 组契约在 socket 层的证据）。
  - **W5c 仍未做（1 项）**：`SET_PARAMETER`/`GET_PARAMETER` 的 `text/parameters` 语义（`raop_handlers.h:1141-1168`，volume/progress；上游在无 `raop_rtp` 时走 `audio_set_volume`）。它本质是媒体动作，与 S4 一起做更合适；在此之前这两条路由仍答 501。NTP/RTP/镜像会话（`:910-929`）同样属 S4。
  - **B. Digest 401**：**不照搬**上游 `pairing_session_make_nonce`（`pairing.c:199-215` 把 `ed_ours` **指针**当 32 字节密钥做 SHA-512，熵来自堆地址/URL/时间戳），改用 `OsRandom` 生成 32 字节并 hex；保留上游的 nonce 比对（`:714`）且 nonce 一次性；`Authorization` 缺失/畸形一律 401（上游 `:701-702` 在缺 `nonce="` 时是空指针算术 + 越界读）；口令比较常量时间、长度先校验；失败计数跨连接共享且有上限；认证状态按连接粘滞；**401 必须发生在解析 plist 之前**。
  - **C. `ekey`/`eiv` 与流参数**：`ekey` 恰好 72 字节、`eiv` 恰好 16 字节，读前即验（上游 `:786`/`:798` 的 `memcpy` 不查来源长度）；复用 `fairplay.rs` 的 `Result` 且 `HandshakeRequired` fail-closed；`streams` ≤ 4、元素必须是 dict、`type` 仅接受 96/110；`timingPort` 必须 `1..=65535`（不复现上游的静默截断）；`eventPort` 只回接收端自己的值，绝不回显客户端提供的端口；响应中的端口必须是**实际绑定**的端口，未绑定则不声明该字段。
  - **D. plist 攻击面**：深度与对象数上限已实现（[`MAX_PLIST_DEPTH`]＝32、`MAX_PLIST_OBJECTS`＝4096，见 `plist.rs`）；SETUP 必须走 `ControlSession::handle_with` 的 64 KiB 体上限，不得绕过；解析失败不得留下半初始化会话；重复键语义需文档化并有用例。
- **安全评审的裁决记录**（2026-10-05，独立安全评审，本轮唯一一份）：
  - 上游「第 3 步不校验第 2 步」的 Critical、以及无 PIN、长度不校验、注册谓词可绕过、全零共享密钥等偏离**全部成立**；本轮新增两项偏离：**一次连接只开一次 `/pair-verify` 密钥交换**（上游 `pairing.c:162-180` 只拒绝 FINISHED）、**每连接最多 `MAX_SETUP_PIN_ATTEMPTS` 个 SRP 上下文**（上游无界，每个 step 1 一次模幂）。
  - **不采纳**「legacy `/pair-setup` 不得开启 `pair-verify`」：客户端就是先 `/pair-setup` 再 `/pair-verify`，禁掉会破坏 legacy 流程；改为在交付层强制 A1（未完成 verify 不交付材料），授权仍由 Digest 401 承担。
  - **不成立**「`client_public_key` 一名两用（X25519/Ed25519）」：两处写入都是 Ed25519（step 3 的 PIN 证明密钥、M1 声明的密钥），X25519 在 `ecdh_client_public`。但 W5c 必须区分二者**信任级别**：M1 里声明的密钥未经验证，只有 step 3 的密钥经过了 PIN 证明。
  - **接受为残余风险**：`num-bigint` 的模幂非常量时间（上游用 OpenSSL），以每连接 SRP 上限 + 连接上限 + 空闲超时缓解，本轮不更换 bignum；已做 `Drop` 尽力清零（身份种子、会话密钥、ECDH 共享秘密）。
  - **W5c 硬性验收项**：密钥材料（PIN、会话密钥、`ecdh_secret`、混合后的 `aeskey`）**永不进日志/响应体/错误文本**——上游恰恰把这些打印出来（`raop_handlers.h:273`、`:803-812`、`:844-846`），本移植必须显式偏离。

### W6 DNS-SD 广告与访问控制

- 目的：让服务可被发现（`_airplay._tcp`、`_raop._tcp`），并对 TXT 能力位与访问控制有明确实现。
- 出口：本机 `dns-sd -B`/`avahi-browse` 可查询到实例；TXT 字段与上游 `dnssd.c` 对齐；访问控制（允许/拒绝、`Active-Remote`/`DACP-ID` 传递）有测试。
- 依赖：W2。写作用域：`crates/airplay-core/src/discovery.rs`（新建）。
- 契约约束（W5c/W6 必须一致，否则客户端会选错配对流程）：TXT 的 `pk` 必须等于 `PairingState::identity_public()`；`sf`/feature bit 27 必须与 `PinMode` 一致（配了 PIN 才声明现代配对可用）；`with_client_registered` 必须接到真实注册表（上游 `uxplay.cpp` 的 `registered_keys`/`-reg`），不能只留谓词。
- 选型旁证（2026-10-05 上游核对）：上游给系统 Bonjour/Avahi 路径新增的 API 自述「必须由应用 service 其 socket，否则 daemon 侧队列无界增长」，且 Windows 需要哑回调才能链接；`mdns-sd` 不需要 fd 服务、不依赖系统 daemon、无平台回调泄漏 —— 这**支持** [ADR-0001](ADR-0001-network-stack.md) 的既有选择，W6 仍须在本机（含 Windows）记录 `mdns-sd` 的真实可用性。
- **状态（2026-10-05）：广告已完成，访问控制待做。**
  - `discovery.rs`：`Advertisement::new(identity, port, protection)` 逐字段复刻上游 `lib/dns_sd/dns_sd.c:238-360` 与 `lib/dnssdint.h:32-49` —— RAOP 记录（`txtvers=1`、`ch=2`、`cn=0,1,2,3`、`da=true`、`et=0,3,5`、`vv=2`、`ft`＝`features` 的 `0x%X,0x%X` 形式、`am`＝model、`md=0,1,2`、`rhd=5.6.0.0`、`pw`/`sf`（无认证 `0x4`、口令 `0x84`、PIN `0x8c`）、`sr=44100`、`ss=16`、`sv=false`、`tp=UDP`、`vs`＝sourceVersion、`vn=65537`、`pk`＝身份公钥；实例名 `AABBCCDDEEFF@Name`）与 AirPlay 记录（`deviceid`、`features`、`pw`、`flags=0x4`、`model`、`pk`、`pi`、`srcvers`、`vv`）。`Advertiser::register`/`unregister` 用 `mdns-sd` 注册两条记录，`enable_addr_auto` 让 responder 自行填地址；单元测试固定了每个字段的期望值（含"不得多写字段"的计数断言）。
  - **本机实测**：CLI 在 `0.0.0.0:47000` 起服务并注册成功；系统 Bonjour 浏览器 `dns-sd -B _raop._tcp local` 在多个接口上看到 `7A341A4E7FA3@Rust AirPlay Test` —— **`mdns-sd` 与 Apple Bonjour 服务可在同一台机器上共存**（两者都用 UDP 5353），这条 Windows 可用性记录补上了第 167 行的要求；入站防火墙规则（`rust-airplay` Allow ×2）已存在。
  - CLI（`crates/airplay-cli`）现在是可运行入口：`--name/--port/--pin [digits]/--no-pin/--password/--device-id/--seed/--help`；身份种子持久化在 `%LOCALAPPDATA%\rust-airplay\identity.seed`（库仍不碰文件系统，符合 ADR-0002），设备 ID 由种子派生（首字节为本地管理地址），PIN 出现时打印到控制台。
  - **Windows 端口注意**：`--port 7000` 在本机失败（`os error 10013`），因为 Hyper-V 保留了 TCP 6935–7034；端口可配置，广告用实际绑定端口，`--port 0` 自动挑空闲端口。
  - **C2 已完成**：`Advertisement::txt_records()` 把两条记录编码成 RFC 6763 线格式（长度前缀 `key=value`，按 key 排序保证稳定，单个条目超过 255 字节则**省略**——上游 `TXTRecordSetValue` 同样拒绝）。CLI 用**同一个 `Advertisement` 值**既填 `DeviceIdentity::txt_airplay`/`txt_raop`（`/info?txtAirPlay` 的回答）又做 mDNS 注册，因此"广告说一套、`/info` 说另一套"在结构上不可能发生；单元测试用解码器做往返断言证明这一点。
  - **W6 剩余**：① **C4 已完成**：CLI 侧注册表 `%LOCALAPPDATA%\rust-airplay\clients`（每行一个 64 位 hex 公钥；非键行跳过而不是让接收端起不来），启动时载入并接到 `PairingState::with_client_registered`；会话中若出现**已验证的客户端公钥**（`SetupRequest.client_identity`）就记住并落盘，下次运行即可为该客户端回答。**诚实限制**：公钥只有在会话建立时才可见（上游在 PIN 第 3 步注册），因此"只配过对、从未建立会话"的客户端仍不在名单里；匹配失败时按上游默认放行，这是 `with_client_registered` 的既定语义。② **C3 已完成**：`RequestTrace` 增加 `active_remote`/`dacp_id`（来自请求头，`upstream/UxPlay/lib/raop.c:342-351` 只是把它们记在连接上交给调用方），文档写明"它们标明请求来自哪个遥控客户端，**不构成任何授权**"（头是连接方自己写的）；CLI 轨迹把它们显示为 `remote=<active>/<dacp>`；socket 用例断言两者确实到达调用方。③ `sf`/bit 27 与 `PinMode` 的映射已实现，待真机确认客户端实际选择哪条配对流程。

### W7 测试基建：集成层、上游差分 oracle、fuzz

- 目的：落实 MIGRATION_PLAN 测试策略第 2–5 条。
- 内容：`crates/airplay-core/tests/` 集成层；受控上游 C oracle harness（仅测试构建，不链接进产品）；`proptest` 或 `cargo-fuzz` 目标覆盖 wire/mirror/plist 解析；控制层多连接/乱序/重复请求场景（随机数与时间注入）。
- 出口：oracle harness 可复现至少 SRP 与 PlayFair 的固定对照；fuzz 目标能跑满一轮无 panic。
- 依赖：可与 W3–W6 并行，但 W5 之后应覆盖服务器路径。
- 可行性更新（2026-10-05）：本机存在 `C:\msys64\ucrt64\bin\{gcc,clang,cmake,pkg-config}.exe`，上游 `CMakeLists.txt` 有 `if (WIN32)` 分支、`lib/` 的 POSIX 依赖很少，因此**C oracle 大概率可在本机构建**（待验证）。若成立，可一次结清：SRP `B` 最小编码的 255/256 分歧、PlayFair 差分、以及配对 CTR/GCM nonce 的上游对照——这些目前都只能靠"与独立实现一致"来支撑。plist 深度闸门（本轮新增）也应在此用 fuzz 覆盖。
- **状态（2026-10-05）：oracle 已建成并结清三项**。`tools/oracle/`（`oracle.c` + `build.ps1` + `README.md`）用 UCRT64 gcc 编译上游 `srp.c`/`crypto.c`/`pairing.c`/`fairplay_playfair.c`/`playfair/*`，仅作测试工具、不进入产品。对照结果（`cargo test -p airplay-core --test upstream_oracle -- --ignored` 3 项 + `--lib -- --ignored upstream_verifier` 1 项，全部通过）：
  - **CTR 偏移**：上游 AES-CTR 上下文产生 128 字节密钥流，服务端签名用第 1 段 64 字节、客户端签名在第 2 段 —— 与本移植逐字节一致（此前只有"独立实现一致"）。
  - **GCM nonce**：`Pair-Setup-AES-IV` 基准末字节 +1（请求）/ +2（回复），双向 `epk`/`authTag` 逐字节一致。
  - **PlayFair**：142 字节 setup 回复、32 字节 handshake 回复、72→16 字节密钥解封，全部逐字节一致。
  - **SRP 最小编码**：上游 verifier **接受**本移植算出的 proof（`authenticated = 1`），且 M2 与会话密钥一致；包括服务端公钥恰好 **255 字节**（首字节为 0）的那一档 —— 即"最小编码 vs 左填充"的分歧在本移植与上游之间**不存在**。仍待真机确认的是 Apple 客户端的行为（无设备/抓包）。
  - 过程中的一个上游调用约定：`srp_verifier_new` 会无条件写 `bytes_B`/`len_B`，传 NULL 会崩溃（上游的调用者必须给真实指针）；本移植的 `server_public()` 无此失败模式。

### W8 S3 验收与文档

- 出口：MIGRATION_PLAN S3 退出标准逐条对照（可发现 / 真实配对→SETUP→RECORD→FLUSH→TEARDOWN / 错误连接有界）；实机矩阵更新，无设备时明确写「未测试」；本文件状态表更新。

## 5. 波次与并行

| 波次 | 内容 | 说明 |
|---|---|---|
| 1 | D1–D8 | 本次完成，独立于 S3 设计决策 |
| 2 | D9 剩余、D10、W4、W6、W7 | W1/W2/W4/W3 已完成；D10 与 W6/W7 互相独立，可并行 |
| 3 | ~~W3~~ → ~~W5a~~ → ~~W5b~~ | W3、W5a、W5b 已完成；配对身份密钥来源见 [ADR-0002](ADR-0002-pairing-identity.md) |
| 4 | W6、W7、W8 | W6/W7 可并行，W8 收尾；D9 剩余（`pairing.rs` 签名往返已随 W5b 补齐，仍缺 `verify_pair_signature` 的独立向量）与 D10（其余 7 个模块的 `missing_docs`）可并入 |

写作用域：W4 新建 `plist.rs`，W5a 新建 `server/`，W6 新建 `discovery.rs`，W7 只碰 `tests/` 与 fuzz 目标。**W3 之后 `control.rs` 不再由任何工作包独占**：W5b 起它随需要改动（`RouteReply::closing`、`ControlResponse::close_after_reply`、`request_length`/`parse_first` 与 wire 上限的映射），W5b 还改动了 `crypto.rs`、`pairing.rs`、`srp.rs`；改动 `control.rs`/`wire.rs` 这类共享文件时必须在提交信息里写明影响面与对应回归用例。

## 6. 需要用户决策

1. ~~**FFmpeg**~~：**已定（2026-09-20）：推迟到 S4 开始前单独决策**，S3 不引入 FFmpeg。
2. ~~**新依赖**~~：**已定（2026-09-20）：接受 tokio + `plist` + `mdns-sd`**，见 [ADR-0001](ADR-0001-network-stack.md)；依赖按工作包逐个落地，不提前写入。
3. **远程仓库**：是否配置远程让 CI 真正运行；否则继续标注「仅本地验证」。**仍待你决策。**
4. **真机**：是否有可用 iPhone/iPad 用于 S3 配对验收；没有的话 S3 只能声明「本机可达」，不能声明互操作成功。**仍待你决策。**
5. ~~**配对身份密钥从哪来（W5b 前必须定）**~~：**已定（2026-09-20）：库不落盘，长期身份由调用方注入 32 字节 Ed25519 种子；禁止 device_id 派生密钥；公钥与 `/info` 同源**。理由与被否方案见 [ADR-0002](ADR-0002-pairing-identity.md)；随机源经 `RandomSource` 注入，默认实现为已提升为直接依赖的 `getrandom 0.2.15`（不新增包）。

## 7. 性能基线（2026-10-05）

方法与结果（可复现）：`crates/airplay-core/tests/server_spec.rs` 的 `measures_control_plane_throughput`（默认 `#[ignore]`，因为它是测量而不是断言）。单条 loopback 连接、把请求一次性流水线写入、服务端写回后读到 EOF：

```
cargo test --release -p airplay-core --test server_spec -- --ignored --nocapture
```

| 路由 | release（3 次） | debug |
|---|---|---|
| `OPTIONS`（无 body） | 74 460 / 69 305 / 77 185 req/s（13.2–14.4 µs/请求） | ~51 000–55 000 req/s |
| `GET /info`（二进制 plist，约 1 KiB 响应） | 31 250 / 30 060 / 32 616 req/s（30.1–33.3 µs/请求） | ~5 500 req/s |

结论与决策：

- **控制面不是瓶颈**。真实客户端在会话建立后每秒只发个位数控制请求，当前余量约三个数量级；`/info` 的成本主要在构造并编码那份 plist（14 个顶层键 + 嵌套数组）。
- 曾实现"请求头只解析一次（新增 `Frame`/`handle_frame`）+ 响应零分配（`write_into`）"的结构优化，release 三次测量**没有收益**（优化后 73.7k/31.3k req/s 对优化前 75.4k/32.5k，落在噪声内），因此按「不为未测量的需求增加 API 面」撤销，只保留测量用例本身。
- 真正的性能工作属于 **S4 媒体路径**：RTP 接收避免每包分配与拷贝、抖动缓冲的容量与水位、镜像/音频交给 GStreamer 而不是自己解码、以及避免在音频线程上做锁竞争与日志格式化。S3 结束时不对媒体性能做任何声明。

## 8. 后续实现计划（2026-10-05 盘点）

盘点结论：工程配置基本齐备（workspace lint `unsafe_code = forbid` / `missing_docs = warn` / clippy `all`、`rust-version = 1.85`、`publish = false`、`.gitattributes` 统一 LF、`README`/`LICENSE`/`THIRD_PARTY_NOTICES` 与 5 篇 `docs/`）。缺口 4 处：CI 从未运行（无远程）、无 `deny.toml`/cargo-deny（许可与漏洞检查）、无 fuzz/基准设施、CLI 仍是占位。按以下顺序推进，每项完成即一个本地提交并通过四门禁。

### A. S3 收尾（1–2 个提交，无新依赖）
1. **✅ `SET_PARAMETER`/`GET_PARAMETER`（已完成）**：按上游 `raop_handlers.h:1069-1179` 移植——缺 `Content-Type` → **451 "Parameter not understood"**；`text/parameters` 的首行前缀匹配 `volume: `（严格解析 f32，非有限值忽略）与 `progress: a/b/c`（严格三段 u32），经 `SetParameter` 回调交给会话层（对应上游交给媒体层）；`image/jpeg`/`image/png`/`application/x-dmap-tagged` 属媒体层，答 200 但不上报（不假装存下了）；`GET_PARAMETER` 的 `volume` 回 `Content-Type: text/parameters` + `volume: {:.6}\r\n`（上游无媒体回调时的默认值 0.0，不编造音量）。socket 用例覆盖 451、volume、progress、封面忽略、非法值忽略与 GET 的响应体。
2. **✅ plist 重复键语义（已完成）**：`plist.rs` 模块文档写明"字典是 key→value 映射，重复键后者覆盖"，`plist_spec.rs` 加用例固定该行为，并记录将来若要拒绝此类 body 应在 `check_structure` 里计数。
3. **✅ 引用一致性（已完成）**：`tests/reference_consistency.rs` 扫描 `src/` 与 `tests/` 中所有 `upstream/UxPlay@<hash>`，断言哈希是固定基线（`57ea8341…`）或差分基线（`3dbf7cee`）的前缀（允许 git 短哈希），并有一条自测覆盖解析形式。**能抓的是"引用了未核对过的提交"，抓不到"整条引用漏了 hash"**——后者靠 W8 出口的人工复核。

### B. W7 测试基建（可与 C/D 并行）
1. **fuzz/proptest**：覆盖 `wire.rs`（头部三项上限、`Transfer-Encoding`、重复/带符号 `Content-Length`、分片与粘包）、`plist.rs`（深度 ≥33、对象数 >4096、畸形 bplist/XML）、`control.rs::request_length` 与 `server/connection.rs` 的分帧。本轮新增的全部解析约束正是它的目标。
2. **✅ oracle 纳入常规回归（已完成）**：新增 `tools/oracle/run.ps1` —— 若 oracle 未构建则先调用 `build.ps1`，然后设置 `AIRPLAY_UPSTREAM_ORACLE` 并运行 `cargo test --workspace --all-targets --no-fail-fast -- --ignored`，退出码即测试结果，可被流水线使用；支持 `-Rebuild` 与 `-Filter <名字>`。**这一步立刻暴露了一个真问题**：这些差分测试需要 `AIRPLAY_UPSTREAM_ORACLE` 环境变量指向 oracle，而之前没有任何地方设置它——也就是说"ignored 的差分"在常规操作下从来没被真正跑过。现在一条命令即可运行，本机实测 **7 条差分全部通过**（lib 内 3 条含 SRP 验证器接受本移植 proof、`upstream_oracle.rs` 3 条含 pair-verify 密钥流/pair-setup nonce 与 GCM/FairPlay、以及吞吐测量 1 条）。CI 里加了独立的 `oracle` job（ubuntu + `libssl-dev` + pwsh 调这两个脚本）；**诚实标注**：`build.ps1` 的 Linux 分支未在本机（Windows）验证过，CI 首次运行时需要确认，且该 job 只在本地存在 `upstream/UxPlay` 检出时才有意义。
3. **✅ 控制层场景（已完成）**：`server_spec.rs` 新增三条并发护栏 + 一条 SETUP 基准——
   - `concurrent_connections_pair_independently`：4 条连接并发走 legacy setup + `/pair-verify` 握手，断言**接收端临时公钥两两不同**（每条连接的状态互不串扰，这是并发下才看得见的性质）。
   - `password_failures_are_counted_across_connections`：6 条连接各错一次口令（最后一条用**正确**口令），全部 401，共享计数等于 5——证明上限跨连接生效，且过了上限连正确口令也拒绝（否则"上限"只是装饰）。
   - `surplus_connections_wait_for_a_slot`：占满 `MAX_CONNECTIONS` 后第 13 条连接**被排队而不是被丢弃**（300 ms 内不应答、套接字不被关闭），释放一个槽位后立刻拿到 200——钉住 `max_connections` 的排队语义（此前修过的 panic 就在这条路径上）。
4. **✅ 基准（已完成）**：新增 `measures_setup_throughput`（ignored 测量），走 FairPlay 握手后连发 300 次 SETUP。实测：**debug 2 130 req/s（469 µs），release 15 062 req/s（66.4 µs）**。对照 §7：OPTIONS release ≈74k、`/info` release ≈31k——SETUP 贵在 FairPlay 解密 + plist 解析 + plist 构造 + SHA-512，属预期量级，且它是**每会话一次**的操作，不在热路径上。

### C. W6 DNS-SD 与访问控制
1. 引入 `mdns-sd`（[ADR-0001](ADR-0001-network-stack.md)），广播 `_airplay._tcp`/`_raop._tcp`，TXT 与 `dnssd.c`/`dnssdint.h` 对齐；**契约**：`pk` == `PairingState::identity_public()`、`sf`/bit 27 与 `PinMode` 一致。
2. `/info` 的 `txtAirPlay`/`txtRAOP` 填真实记录（现在是空 data）。
3. `Active-Remote`/`DACP-ID` 传递（上游 `raop.c:342-351` 只导出，不做鉴权）。
4. **注册表接线**：`with_client_registered` + `SetupRequest.client_identity` → 真实名单（配置层负责持久化，见 [ADR-0002](ADR-0002-pairing-identity.md)）。
5. Windows 上 `mdns-sd` 可用性记录；不可用则按 ADR-0001 的降级路径决策。

### D. S4 前置（媒体栈）
1. **FFmpeg 决策**（用户）与 GStreamer 版本/插件矩阵确认。
2. **端口绑定**：NTP 时序口、音频 data/control、镜像 data 真实绑定后填进 `SetupPorts`，SETUP 响应从此声明真实端口（现在未绑定的字段一律不写）。
3. **媒体会话接口**：以 `SetupRequest`（audio key/IV/streams/identity/auth）为输入，定义 RECORD/FLUSH/TEARDOWN 的媒体语义、重传与 NTP 同步（上游 `raop_ntp.c` 的签名已变，开工前重读）。
4. **性能设施**：抓包回放 + 同硬件对比（MIGRATION_PLAN 测试策略 5）。

### E. W8 S3 验收与文档
1. 退出标准逐条对照；真机矩阵无设备时写"未测试"。
2. 能力声明区分 `RouteHandlers::new`（配对 501）与 `with_pairing`。
3. ADR 索引 + 引用一致性检查（见 A3）。
4. README 状态更新（当前：控制面 + 配对 + SETUP 密钥派生；无媒体）。

### F. 工程化（建议，低成本）
1. **`deny.toml` + cargo-deny**：许可白名单（与 `THIRD_PARTY_NOTICES.md` 对齐：MIT / Apache-2.0 / MIT OR Apache-2.0 / LGPL-2.1-or-later 的移植边界）、advisory、重复依赖；本地可跑，CI 恢复后纳入。
2. **远程仓库**（用户决策 3）：配置后 CI 的 3 OS + MSRV 1.85 job 才会真正运行；未配置前一律标注"仅本地验证"。
3. **CLI 占位**：`--help`/`--version` 目前由 `main` 直接输出，S6 做完整选项矩阵时统一。
4. 可选择补 `rust-toolchain.toml`；MSRV job 已覆盖 1.85，本地 stable 漂移风险可控。

### 依赖与决策点
- 顺序：A → (B ∥ C) → D → E；F1/F3 随时可做，F2 取决于用户。**A 已完成，B ∥ C 进行中。**
- **真机进展（2026-10-05）**：iPad 已发现本接收端并进入配对流程（接收端打印一次性 PIN，用户确认"可以连接"）。这是 S3 出口标准里"可发现 + 真实配对"的第一份真机证据；但**媒体未播放**（S4 未实现，SETUP 响应不声明媒体端口），也还没有请求轨迹，所以 iPad 每一步的具体请求内容尚未记录——B3/C 的收尾应把"真机请求序列"作为验收材料补齐。
- **待用户决策**：① 远程仓库/CI 是否启用；② 真机已确认有 iPad，S3 出口可含真机配对证据，"播放"仍归 S4；③ FFmpeg 取舍（S4 前）。

## 9. 进度记录

| 日期 | 工作包 | 提交 | 说明 |
|---|---|---|---|
| 2026-09-20 | D1–D8 | `3fc20c0` `95fbf37` `c9467ca` `323ce83` `7888a64` `5a97d8b` | 测试 30 → 43，fmt/test/clippy/doc 全绿 |
| 2026-09-20 | W1 | `c07cb2b` | 上游路由表 + 响应头契约 + `tests/control_spec.rs`（46 单元 + 7 集成） |
| 2026-09-20 | W2 | `f203204` | [ADR-0001](ADR-0001-network-stack.md)：tokio + plist + mdns-sd；FFmpeg 推迟到 S4 前 |
| 2026-09-20 | W4 | `8e72227` | `plist.rs` + `plistlib` 双向夹具（`tests/plist_spec.rs`、`tools/plist_fixtures.py`）；51 单元 + 7 + 5 集成；`plist 1.8.0` 落地 |
| 2026-09-20 | W3 | `833d8c4` | `control.rs` 无状态路由 + 有状态会话：`ControlOutcome`、`X-Apple-Session-ID` 绑定、可选 `CSeq`、协议回显、`Audio-Latency`、`Connection: close`、431/400 映射；`tests/control_spec.rs` 改 15 个正向断言；58 单元 + 15 + 5 集成 |
| 2026-09-20 | W5a | `322e01b` | `server/` 三层 + `RouteHandler` 委派 + `request_length`；`/info`、`/feedback`、`/fp-setup`；`tests/server_spec.rs` 9 个真实 socket 用例（含修掉「缓冲区已有完整请求仍阻塞 read」的缺陷）；67 单元 + 15 + 5 + 9 集成 |
| 2026-09-20 | W5b | `7466e03` `53d9362` | 配对路由 `/pair-pin-start`、`/pair-setup-pin`、`/pair-setup`、`/pair-verify`（`server/pairing.rs`）；修掉 `/pair-verify` 第二步 CTR 偏移、补 GCM 加密原语与 `AppleSrpServer::server_public()`、`RouteReply::closing`；拒绝上游「跳过 proof 校验即可完成配对的」Critical 缺陷；83 单元 + 15 + 5 + 15 集成 |
| 2026-10-05 | 阶段性验收 | `f55e633` `5dad9a4` `a07da76` 本次提交 | 上游核对到 `3dbf7cee`（§0）＋空闲超时对齐 60 s（§7 性能基线）；评审团**架构与验收两个角色产出、安全角色本轮被中止无产出（覆盖缺口见下）**，发现并修掉：`check_register` 谓词可被重发 `/pair-verify` 绕过（阻断，端到端复现）、缺上游三个头部上限（21 字段/65 字节字段名/1025 字节值应 431）、`Transfer-Encoding` 被忽略导致分帧错位、超长 protocol/version 被接受并回显、重复与带符号 `Content-Length` 被接受、`max_connections` 超上限时 `Server::bind` panic；89 单元 + 15 + 5 + 18 集成（另 1 个默认忽略的性能测量用例） |
| 2026-10-05 | 本轮覆盖缺口 | — | 安全角色（应用安全工程师）的验收分析未产出。它被交接的四项由主理人自行裁决并记入文档：注册拒绝改为粘滞（fail-closed）、分帧类偏离一律 fail-closed、`x25519_shared` 全零拒绝、W6 不复制系统 Bonjour 路径。**待办**：若要在 W5c（Digest 401、会话密钥接入）开工前补齐独立安全视角，需要重新召集该角色 |

提交与工作包对应：D1 `3fc20c0`；D2 `95fbf37`；D5 `c9467ca`；D6 `323ce83`；D3/D4 `7888a64`；D7/D8 `5a97d8b`。

## 10. 结构整理与 bug 审计（2026-10-05）

### 10.1 生产代码 panic 审计
扫描全部 `.rs`（排除 `#[cfg(test)]` 之后的测试代码）中的 `unwrap/expect/panic!/todo!/unimplemented!`：**协议库生产代码 0 处依赖外部输入的 panic 路径**。剩余的少数几处是"构造上不可能失败"的类型转换——`src/pairing.rs:37-85` 是对 32 字节哈希切出 `[u8;16]` 的 `try_into()`，`src/srp.rs:53` 是解析编译期常量模数——它们不随客户端输入变化，保留并在评审中记录。**已修复的真实脆弱点**：CLI 三处 `Mutex::lock().expect("lock")`（stdout 关闭会让回调 panic → 锁中毒 → 之后每条连接都 panic）改为 `unwrap_or_else(PoisonError::into_inner)`，与库内 `guard()` 的既有做法一致。

### 10.2 大文件拆分
测试模块从生产文件中抽出为同级 `tests.rs`（`foo.rs` + `foo/tests.rs`），生产文件不再被上千行测试淹没：

| 文件 | 拆分前 | 拆分后（生产） | 抽出的测试 |
|---|---|---|---|
| `src/server/pairing.rs` | 1742 | **985** | 757 → `src/server/pairing/tests.rs` |
| `src/control.rs` | 1330 | **985** | 345 → `src/control/tests.rs` |
| `src/server/handlers.rs` | 1038 | **926** | 112 → `src/server/handlers/tests.rs` |
| `src/playfair.rs` | 798 | **601** | 197 → `src/playfair/tests.rs` |
| `src/plist.rs` | 610 | **413** | 197 → `src/plist/tests.rs` |
| `src/discovery.rs` | 519 | **353** | 166 → `src/discovery/tests.rs` |

拆分是纯机械操作，安全性由**测试数量不变**证明（拆分前后均为 165 通过 / 9 忽略）。未拆的两类文件：`playfair_tables.rs`(6108) 与 `playfair/hand_garble_generated.rs`(873) 是**生成的数据表**，拆分只会破坏"生成物与生成器一一对应"；`tests/server_spec.rs`(1572) 是测试文件本身，拆分需要抽出共享夹具，留待需要时再做。

### 10.3 性能复核（release，观测缝落地之后）
| 路径 | 本次 | 此前基线 | 结论 |
|---|---|---|---|
| OPTIONS | 62 580 req/s（16.0 µs） | ≈74k | 单次运行波动（同机不同时刻，loopback 测量对后台负载敏感） |
| `GET /info` | 31 376 req/s（31.9 µs） | ≈31–32k | 无回归 |
| SETUP | 9 621 req/s（103.9 µs） | 15 062（66.4 µs） | 波动较大，见下 |

观察：这三条都是**微秒级、单连接、loopback** 测量，重复运行的离散度可达 ±30%（OPTIONS 62.6k–74k、SETUP 9.6k–15k），因此**只能用作量级参考，不能当作性能承诺**；这也解释了为什么早先"解析一次 + 零分配回复"的改动测出 73.7k vs 75.4k——落在噪声里，故当时已回退。**新增的观测缝没有可测量的开销**（`trace: None` 时只是一次 Option 检查），但上面的数字不足以证明这一点，真正的结论需要 S4 的抓包回放与同硬件对比设施（计划 §8 的 D4）。