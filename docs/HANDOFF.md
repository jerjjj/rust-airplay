# 交接说明（新会话从这里开始）

> 目的：让下一个会话（或另一位协作者）不必重新推导任何东西即可接手。**先读本文件，再读 [MIGRATION_PLAN.md](MIGRATION_PLAN.md)（全项目计划）与 [S4_WORK_PLAN.md](S4_WORK_PLAN.md)（当前阶段执行计划）。**

## 0. 一句话状态

协议核心已完成到 **S3 收口**（发现、控制面、配对、SETUP 密钥派生、Digest 口令；**真机 iPad 已实测发现与配对**），**S4 媒体栈进行中**：音频收包/解密/重排序/控制通道/端口绑定已完成并接线，**镜像通道差最后一段（CTR 规则）**，渲染后端（GStreamer）尚未开始——**目前仍不出声、不出画**。

## 1. 接手第一步：跑这三条命令

```powershell
cargo test --workspace --all-targets                 # 普通套件，最近一次 209 通过
pwsh -File tools/oracle/run.ps1                      # 上游 C 差分，7 条必须全通过
cargo clippy --workspace --all-targets -- -D warnings
$env:RUSTDOCFLAGS='-D warnings'; cargo doc --workspace --no-deps
```

四门禁是全绿基线（`fmt` / `test` / `clippy -D warnings` / `doc -D warnings`）。若某一条红了，先修它再动新代码。

## 2. 直接接手的工作：S4-1c-2 镜像 CTR 解密规则

**位置**：`crates/airplay-core/src/media/mirror.rs`

**规则（已从 `upstream/UxPlay/lib/mirror_buffer.c` 的 `mirror_buffer_decrypt` 逐条读出，不要重新猜）**：

1. 若上一个包留下 `nextDecryptCount` 个密钥流字节，则用它们异或本包开头：`output[i] = input[i] ^ og[(16 - nextDecryptCount) + i]`；
2. `encryptlen = ((inputLen - nextDecryptCount) / 16) * 16`，先 `aes_ctr_start_fresh_block`（**每包重启计数器**），再解密这 `encryptlen` 个字节；
3. `restlen = (inputLen - nextDecryptCount) % 16`；若 `restlen > 0`：把尾部 `restlen` 字节放进 16 字节缓冲（其余补零）**整块**解密，只用前 `restlen` 字节，**该块剩余的 `16 - restlen` 个字节就是留给下一个包的密钥流**；`nextDecryptCount = 0` 重置于下包；
4. 重键（`mirror_buffer_init_aes`）时 `nextDecryptCount = 0`——新流从密钥流起点开始。

**现状与坑**：
- 当前 `StreamCipher` 是"单条连续密钥流"模型，**与上游不符，且没有接到任何流上**（文档里已写明）。要**替换**它，不要扩展。
- 我（上一个会话）尝试替换后有两处断言失败且**未定位**：
  - `carried()` 在解密 40 字节包后返回 0（期望 8）——实现骨架看着与规则一一对应，需要先复现再动手；
  - 另一条失败是**我的用例写错**（断言"解密全零缓冲等于全零"——零明文异或密钥流当然不是零）。已被回退，不在树里。
- **验证方式必须换**：不要用"两包等于一段"（那是错误模型的性质）。正确做法是**独立重算密钥流**（用 `crypto::Aes128CtrState` 直接从 IV 生成 N 块密钥流）后**逐字节比对**，并用"40 字节包接 24 字节包"这种**非整块长度**的固定向量覆盖携带逻辑。
- 之后还需：SPS/PPS 前缀拼装（`raop_rtp_mirror.c:420-450`：未加密的 codec 数据包要在下一个加密视频包前拼上）、把镜像链路接到 `MediaSockets::accept_mirror` 上、与上游做 oracle 差分。

## 3. 已完成（不要在细节上重做）

- **S0–S2**：仓库/许可/CI 配置、协议原语、密码链与 PlayFair，均与上游 C 有 oracle 差分。
- **S3（已收口，见 [S3_ACCEPTANCE.md](S3_ACCEPTANCE.md)）**：plist（有界）、控制面路由与状态机、配对（PIN 三步 / legacy / pair-verify）、SETUP 密钥派生（FairPlay 解密 + `SHA512(aeskey‖ecdh)[..16]` 混合）、Digest 401（单次 nonce、跨连接失败计数）、`SET_PARAMETER`/`GET_PARAMETER`（缺 `Content-Type` 答 **451**）、发现广告（TXT 逐字段对齐上游）、`/info` 与广告同源、客户端注册表。**真机 iPad 实测**：完整请求序列与握手 63 ms 时序已归档。
- **S4 已完成**（14 个提交）：
  - `media/audio.rs`：RTP 音频包解析（上游布局、`00 68 34 00` 标记）、有界重排序窗口（512、缺口记录、远跳重开、序号回绕）、载荷解密（**只解密 `len/16*16` 对齐前缀**、**每包重置 IV**）；
  - `media/control.rs`：控制通道（解析 `0x56` 重发/`0x54` 同步；构造 8 字节重传请求 `0x80 0xd5`；重发音频走同一路径**真正补缺口**）；
  - `media/timing.rs`：时序报文层（32 字节请求 `0x80 0xd2 0x00 0x07` + 偏移 24 时间戳；应答三值偏移 8/16/24）；
  - `media/ports.rs`：**三个 UDP 端口 + 镜像 TCP 监听**、收包循环（`select!` + 帧回调）、分包读取（含**短读**处理）；
  - `media/mirror.rs`：镜像密钥派生（`AirPlayStreamKey`/`AirPlayStreamIV` + **十进制**连接号 + 音频密钥 → SHA-512 取前 16 字节）、128 字节包头与类型表、NAL 长度前缀→Annex-B 起始码就地改写与校验；
  - **CLI 接线**：先绑端口再广告 → `SetupPorts` 声明真实端口 → 收到带密钥的 SETUP 后启动收包循环。**真机现在会真的发包**（日志出现 `*** media session keyed, audio can arrive ***`）。

## 4. S4 剩余路线

1. **S4-1c-2** 镜像 CTR 规则 + SPS/PPS 拼装 + 接线（见上）。
2. **S4-1b-3** 时钟算术：读 `raop_ntp.c` 的 `t1..t4` 段、`raop_ntp_adjust_remote_timestamp_offset`、重复/伪造包拒绝、`sync_delay`、`recv_time_kernel` 可能为 NULL。
3. **S4-2 GStreamer 渲染**（见 [ADR-0003](ADR-0003-media-backend.md)）：定义最小渲染接口（`open/push/flush/close/set_volume`），新建 `crates/airplay-renderer-gst`，**并定案该 crate 如何不污染默认门禁**（feature 或独立成员）。**这一步之后才会出声出画。**
4. **S4-3** RECORD/FLUSH/TEARDOWN 的媒体语义、volume/progress 作用到渲染层、SETUP 多流语义、HLS `playbin`。
5. **S4-4** 抓包回放 + 同硬件对比（首帧/延迟/A-V 偏差/丢帧/CPU/内存）、长时播放与断线恢复。
6. 之后 S5–S8（见 MIGRATION_PLAN §3）。

## 5. 环境与网络事实（本机）

- Windows x64，Rust stable（`stable-x86_64-pc-windows-msvc`），MSRV 1.85（CI 有独立 job，**CI 从未运行**——无远程仓库）。
- **iPad 可用**：已能发现接收端、走完 PIN 配对与缓存配对快路径、发 SETUP/RECORD/TEARDOWN。
- 接收端起法：`cargo run -p airplay-cli -- --name "Rust AirPlay Test" --port 47000 --log session.log`（`--port 0` 自动选端口；`--log` 追加请求轨迹，**行内含毫秒时间戳**，不含密钥）。
- **端口注意**：本机 Hyper-V 保留 TCP 6935–7034，默认 7000 不可绑定（`os error 10013`）；换端口即可。
- 防火墙已有 `rust-airplay` 入站 Allow 规则。
- `mdns-sd` 与系统 Apple Bonjour **可共存**（UDP 5353），已实测。
- **GStreamer 前提**：本机 MSYS2 UCRT64 有 GStreamer 1.28.3，但**与默认 Rust 工具链 ABI 来源不同**；S4-2 前必须选定一致 ABI 并验证链接，不能凭 pkg-config 成功就认为可构建。AirPlay AAC-ELD 样本**尚未验证**。
- 身份与客户端名单：`%LOCALAPPDATA%\rust-airplay\identity.seed` 与 `...\clients`（库不碰文件系统，属 CLI 配置层）。

## 6. 约定与纪律（必须遵守）

- **四门禁**全绿才提交：`cargo fmt --all -- --check`、`cargo test --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`。
- **一个工作包一个本地提交**，提交信息说明**为什么**；不要在提交信息里写未实测的数字。
- **不猜上游**：任何协议细节必须先读 `upstream/UxPlay`（本地检出，基线 `57ea8341`，差异核对基准 `3dbf7cee`；引用写 `upstream/UxPlay@<hash>`，有测试强制哈希合法）。读不到就先记为"待读"而不是实现一个像样的猜测。
- **生产代码不得因外部输入 panic**；长度运算 checked；先校验再分配；解析/解密不得越权。
- **能力声明只覆盖已实现功能**；未实测就写"未测试"。
- 新增依赖需记入 `THIRD_PARTY_NOTICES.md` 与相应 ADR。

## 7. 诚实清单（未验收）

- 媒体：**没有任何音频或画面被播放过**（S4-2 未做）。
- 真机：RTP 是否真的到达 data 口**尚未验证**（端口已声明、循环已启动，但没在 iPad 会话里确认过帧计数）。
- 镜像：CTR 规则未实现、SPS/PPS 未拼装、未接线。
- 平台：macOS/Linux/BSD/ARM 未构建；`mdns-sd` 仅 Windows 实测；oracle 的 `build.ps1` Linux 分支未验证；CI 从未运行。
- 时钟：时序报文层完成，**时钟算术未做**（同步偏差无法计算）。
- 工程：无 `deny.toml`/cargo-deny；无 fuzz 常态化（已有属性测试与深 bplist 回归护栏）。

## 8. 我已踩过的坑（省你时间）

1. **Windows 沙箱 ACL**：本工作区在受限模式下曾无法被授权（`grantWrite(...)` 失败），需要完全访问模式；`C:\Windows\System32\dns-sd.exe` 可用于独立确认 mDNS 广告。
2. **不要用脚本盲替换改代码**：我因此毁过一次 `main.rs`（`println!` 替换命中错误位置），也曾在 `discovery.rs` 误删过测试头。**先读准区域，再用编辑工具**。
3. **重排序窗口语义**："第一帧定起点"——我两次把用例写成"先到的高序号包会被挂起"，实际它反而定起点。
4. **NTP 时间基**：`ntp::unix_nanos_to_timestamp` 收 **Unix** 纳秒（自己加 NTP 纪元）；且 32 位小数**无法逐纳秒精确**，往返断言要允许 ≤2 ns。
5. **`Plist` 的 `get_*` 返回 `Result`**（不是 `Option`）；`rtp::parse` 是**自由函数**；`ParseError` 在 `wire` 模块。
6. **测试文件的服务端夹具**：`server_spec.rs` 里的 `TestServer.address` 是**字段**不是方法。
7. **文档链接**：`cargo doc -D warnings` 会因为指向未引入类型的 intra-doc 链接而失败（提交前跑 doc 门禁能当场发现）。
8. **`git commit -m` 里带引号/反斜杠**会被 shell 解析坏——写 `target/commit-*.txt` 用 `-F` 提交。
