# ADR-0002：配对身份密钥的来源与配对状态归属

- 状态：**已接受**（2026-09-20，S3 工作包 W5b）
- 决策者：本项目主理人，依据技术方案评审团（架构／安全／验收）的评审与独立向量核验
- 相关：`docs/ADR-0001-network-stack.md`、`docs/S3_WORK_PLAN.md`、`crates/airplay-core/src/server/pairing.rs`

## 背景

AirPlay 的现代配对流程（`/pair-setup-pin` 与 `/pair-verify`）需要三样长期或共享的东西：

1. **长期 Ed25519 身份密钥**：客户端在第 1 步用缓存下来的接收端公钥认证接收端，`/info` 的 `pk`、`/pair-setup` 的返回值和 `/pair-setup-pin` 第 3 步加密的内容都是这把密钥的公钥（上游 `upstream/UxPlay/lib/raop.c:659-674` 只读一处 `pk`）。
2. **PIN 策略**：上游把配置的 PIN 存成 `pin + 10000` 哨兵（`uxplay.cpp:1654`），随机 PIN 存 `raop->pin`，`use_pin` 决定 `/pair-verify` 的分支（`raop.c:627-628`、`:764-766`；`raop_handlers.h:469-477`）。
3. **每连接的临时材料**：SRP 上下文、proof 校验后的会话密钥、X25519 临时密钥对（上游 `raop_conn_t::session`，`raop.c:151`）。

上游的密钥来源是「文件或从 device_id 派生」：`pairing_init_generate(device_id, keyfile, …)`，keyfile 默认为空串时走 `Ed25519(seed = SHA512("UxPlay-Persistent-Not-Secure-Public-Key" ‖ device_id))`（`crypto.c:38`、`:351-386`）。

## 决策

1. **库不读写文件，长期身份由调用方以 32 字节 Ed25519 种子注入**（`PairingState::new` / `with_random`）。持久化（文件路径、权限、原子写）属于后续配置层，不属于核心库。
2. **同一 device_id 必须始终配同一身份密钥**，且公钥必须与广告的 `pk` 同源：`RouteHandlers::with_pairing` 用 `PairingState` 的公钥覆盖 `DeviceIdentity::public_key`，因此 `/info`、`/pair-setup` 与第 3 步永远是同一把密钥。接收端换钥的后果只有一个：客户端必须重新配对（已写成测试与文档，未在真机验证）。
3. **不实现 device_id 派生密钥路径**，也不把它作为默认。device_id 是 MAC 地址，会在 mDNS/TXT 广播，任何人可据此推出私钥并伪造接收端签名。若将来为兼容旧 UxPlay 安装而要复刻，必须放在配置层、显式命名（例如 `from_device_id_insecure`）并默认关闭。
4. **共享状态在 `PairingState`，每连接状态在 `PairingSession`**：PIN、失败计数、身份密钥、注册谓词属于接收端；SRP、会话密钥、X25519 临时密钥属于连接。加锁只保护状态读写，绝不跨调用方回调持锁；PIN 通过 `pending_pin()` 读取而不是回调。
5. **随机源是唯一注入点**（`RandomSource`），只读不落盘的随机性来源为 `getrandom 0.2.15`（`Cargo.lock` 中已作为传递依赖存在，提升为直接依赖不新增任何包）；测试注入确定性实现。
6. **不引入「是否已配对」这一接收端授权位**。上游没有它：注册表在应用层（`uxplay.cpp:2635-2665`），库只提供 `check_register` 谓词；本移植把它做成可选的 `with_client_registered`，未注入时与上游一致地放行，注入即表示调用方要求注册表。

## 后果

- 调用方必须自己保证身份密钥的持久性与机密性（最小权限、原子写）；否则每次重启都要重新配对，而默认配置下客户端可能表现为「接收端不可用」（真机行为未验证）。
- 库不引入路径、权限与平台差异，核心仍可脱离网络与设备测试（`docs/ARCHITECTURE.md`）。
- 配对不再把「无 PIN」当作可用配置：`PinMode::Disabled` 时 `/pair-setup-pin` 一律 `470`，这是**有意偏离上游**的隐式 `"0000"` 口令（理由与全部偏离见 `server/pairing.rs` 的模块文档与 `docs/S3_WORK_PLAN.md` 的 W5b 小节）。
- 新增直接依赖 `getrandom 0.2.15`（MIT OR Apache-2.0），已记入 `THIRD_PARTY_NOTICES.md`。

## 被否方案

| 方案 | 否决理由 |
|---|---|
| 库内读写密钥文件（照搬上游） | 把路径、权限、原子写与平台差异拖进核心；将来改成注入式是破坏性变更；违反「核心可脱离设备测试」 |
| 进程内临时身份密钥 | 每次重启都要重新配对；默认配置下客户端会静默失败，用户无法诊断。只允许测试使用 |
| 复刻 device_id 派生密钥 | device_id 公开可推导私钥，等于放弃接收端认证 |
| 把「已配对客户端列表」放进库 | 上游把它放在应用层；放进库会让「配对成功」被误当授权位，而 `/pair-verify` 成功并不授予访问权（真正的门是 SETUP 的 Digest 401 与 DNS-SD 策略位） |
| 共享状态不用锁、照搬上游的数据竞争 | 上游无锁是缺陷；`std::sync::Mutex` 且中毒后恢复（`PoisonError::into_inner`）可在不 panic 的前提下工作 |
