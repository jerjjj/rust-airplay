# ADR-0001：S3 网络、plist 与 DNS-SD 依赖选型

- 状态：已接受（2026-09-20，由项目所有者确认）
- 范围：S3 及之后（S4–S6）的网络运行时、plist 编解码与 DNS-SD
- 关联：[MIGRATION_PLAN.md](MIGRATION_PLAN.md) S3、[S3_WORK_PLAN.md](S3_WORK_PLAN.md) W2、[ARCHITECTURE.md](ARCHITECTURE.md)

## 背景

S3 需要发现、网络控制服务、plist 编解码、会话生命周期和访问控制。上游对应实现依赖：

- **libplist**：`upstream/UxPlay/lib/raop_rtp_mirror.c:35` 直接 `#include "plist/plist.h"`；二进制与 XML 两种格式都在用（`raop.c:370`、`raop.c:518`、`raop_handlers.h:72`/`246`/`302`/`347`/`384`/`426`、`http_handlers.h:95`/`201`/`297`/`449`/`515`/`706`）。
- **Bonjour / libdns_sd**：`upstream/UxPlay/lib/dnssd.c`、`dnssd.h`、`dnssdint.h`。
- **socket 与线程**：`upstream/UxPlay/lib/httpd.c`、`http_request.c`、`http_response.c`、`netutils.c`、`sockets.h`、`threads.h`、`compat.c`。

这是本移植项目的第一批非密码依赖，必须先定选型再写代码。

## 决策

1. **网络运行时：tokio**，实际使用 `rt-multi-thread`、`net`、`time`、`sync`、`io-util`、`macros`。
2. **plist：`plist` crate**（落地时写 `default-features = false`，只用其 `Value` API；可选 `serde` 不进依赖图，`cargo tree -i serde` 无输出）。
3. **DNS-SD：`mdns-sd`**，纯 Rust 实现，不绑定系统 Bonjour/Avahi。
4. **依赖在首个使用它的工作包落地**，不在 W2 提前加入 `Cargo.toml`，以满足 ARCHITECTURE.md「不为未来功能堆空 crate 或抽象」。落地点：W4 `plist`，W5 `tokio`，W6 `mdns-sd`。
5. **FFmpeg 决策推迟**：不在 S3 引入 FFmpeg；是否作为新增外部原生依赖在 S4 开始前单独决策。

## 证据（`cargo info <crate> --registry crates-io`，2026-09-20）

| crate | 解析到的版本 | license | rust-version | 结论 |
|---|---|---|---|---|
| tokio | 1.53.1 | MIT | 1.71 | 满足 MSRV 1.85；许可与 GPL-3.0-or-later 兼容 |
| plist | 1.8.0（latest 1.10.1） | MIT | 1.81 | 1.10.1 的 `rust-version` 是 1.88.0，超过本仓库 MSRV，`resolver = "3"` 的 MSRV 感知解析自动回退到 1.8.0 |
| mdns-sd | 0.21.4 | Apache-2.0 OR MIT | 1.71.0 | 满足；默认 features 为 `async` + `logging`，不依赖任何 async runtime |
| socket2 | 0.6.5 | MIT OR Apache-2.0 | 1.70 | 备选路径用；注意 tokio 的 `net` feature 已传递依赖 socket2 |

MSRV 记录：workspace `rust-version = "1.85"`（本地工具链 1.98.1）。四个候选 crate 的上界都满足，但 plist 需要固定到 1.8.x 或等 MSRV 提升后再升级——升级 MSRV 时要重新评估。

## 理由

- **tokio 而非自管事件循环**：S3/S4 同时存在监听 TCP、RTSP/HTTP 控制连接、镜像 TCP、音频 UDP 与 NTP 定时器。ARCHITECTURE.md 要求「单会话所有权 + 有界消息队列，避免全局大锁」，这与「每连接一个 task + 有界 `mpsc`」直接对应；自己写非阻塞事件循环会引入更多必须审查的状态机代码，风险高于引入成熟 runtime。
- **`plist` 而非自研**：需要同时支持 binary 与 XML plist，且要构造 `/info` 响应；自研的边界情况（整数宽度、UID、日期、data 节点）是纯风险，没有兼容性收益。
- **`mdns-sd` 而非系统 Bonjour/Avahi**：纯 Rust 可在三平台 CI 中运行；绑定系统实现会让 Windows/Linux/macOS 构建与测试出现平台分支，也会把新的 C 依赖带进来。
- **许可**：MIT / Apache-2.0 与 GPL-3.0-or-later 兼容；仍需在 THIRD_PARTY_NOTICES.md 记录新增依赖。

## 被否方案

| 方案 | 否决理由 |
|---|---|
| `std::net` + 自管非阻塞/事件循环 + `socket2` | 需要自己实现可取消的连接状态机与定时器；在「禁止 unsafe、错误不得 panic」的约束下维护成本高。保留为降级路径：若 tokio 带来不可接受的依赖面或平台问题，改用 std + socket2（socket 选项如 `SO_REUSEADDR`、`TCP_NODELAY` 用 socket2 设置） |
| 绑定系统 Bonjour / Avahi（`libdns_sd`） | 平台分支、CI 复杂度、引入新的原生依赖 |
| 自研 plist 子集 | 只为「不新增依赖」付出兼容性风险；如 `plist` 出现 MSRV 阻塞，可作为 W4 的降级方案（只支持实际用到的节点类型） |
| FFmpeg 输出（F0–F4） | 本次不决策，推迟到 S4 前；见 `research/ffmpeg-output-feasibility.zh-CN.md` |

## 后果

- `Cargo.lock` 将随 W4/W5/W6 明显增长（tokio 会带入 mio、socket2、bytes、pin-project-lite 等）；每个工作包提交后要重新跑三条验收命令，并在提交信息里记录新增依赖。
- CI 的 MSRV 作业（1.85.0）必须在引入 `plist` 后仍通过；如果 `plist 1.8.0` 在 1.85 上失败，则改用自研子集（W4 的降级方案）。
- 依赖升级不再只看 latest：`resolver = "3"` 的 MSRV 感知解析会静默选择旧版本，评审时要用 `cargo tree` 确认实际解析结果。
- 2026-09-20 随 W4 用 `cargo metadata --locked` 审计了全部 71 个锁定包的 `rust-version`：没有任何包声明高于 1.85（恰好等于 1.85 的是 `base64ct 1.8.3`、`deranged 0.5.8`、`hashbrown 0.17.1`、`indexmap 2.14.2`、`zeroize 1.9.0`、`zeroize_derive 1.5.0`，以及本仓库两个 crate；其余 16 个包未声明）。本机只装了 stable/1.98.1，没有 1.85 工具链，因此 MSRV 的**编译**验证仍然只能由 CI 的 MSRV 作业完成。

## 待办

- [x] W4 落地 `plist` 时，在 THIRD_PARTY_NOTICES.md 记录版本与许可：`plist 1.8.0`（MIT），同时记录传递依赖 `quick-xml 0.38.4`、`indexmap 2.14.2`、`hashbrown 0.17.1`、`time 0.3.45`、`base64ct 1.8.3` 等（见 `Cargo.lock`）。
- [ ] W5 落地 `tokio` 时，记录实际启用的 feature 集合。
- [ ] W6 落地 `mdns-sd` 时，确认 TXT 记录构造能力覆盖上游 `dnssd.c` 的字段。
- [ ] S4 前完成 FFmpeg 决策并单独记录。
