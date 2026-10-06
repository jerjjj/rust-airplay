# 交接说明（新会话从这里开始）

> 目的：让下一个会话（或另一位协作者）不必重新推导任何东西即可接手。**先读本文件，再读 [MIGRATION_PLAN.md](MIGRATION_PLAN.md)（全项目计划）与 [S4_WORK_PLAN.md](S4_WORK_PLAN.md)（当前阶段执行计划）。**

## 0. 一句话状态

协议核心已完成到 **S3 收口**（发现、控制面、配对、SETUP 密钥派生、Digest 口令；**真机 iPad 已实测发现与配对**），**S4 进行中**：音频收包/解密/重排序/控制通道/端口绑定已完成并接线，**镜像通道整条已完成（CTR 规则 / SPS+PPS 拼装 / 接线，与上游 C 差分通过）**，**渲染边界与 GStreamer 后端已落地**（接口 + 子进程接入，六个集成测试对真 GStreamer 通过，但**尚未接进 CLI**）——**目前仍不出声、不出画**。

## 1. 接手第一步：跑这三条命令

```powershell
cargo test --workspace --all-targets                 # 普通套件，最近一次 209 通过
pwsh -File tools/oracle/run.ps1                      # 上游 C 差分，现在 8 条（含镜像）必须全通过
cargo clippy --workspace --all-targets -- -D warnings
$env:RUSTDOCFLAGS='-D warnings'; cargo doc --workspace --no-deps
```

四门禁是全绿基线（`fmt` / `test` / `clippy -D warnings` / `doc -D warnings`）。若某一条红了，先修它再动新代码。

**GStreamer 后端不在默认门禁里**（独立 workspace），要另外跑：

```powershell
cd crates/airplay-renderer-gst
cargo clippy --all-targets -- -D warnings
$env:PATH = 'C:\msys64\ucrt64\bin;' + $env:PATH
cargo test -- --ignored --test-threads=1            # 6 条，需要真 GStreamer 运行时
```

## 2. 直接接手的工作

### 2a. 把渲染后端接进 CLI（S4-2 的最后一段）

接口在 `crates/airplay-renderer`（`Renderer`/`AudioStream`/`VideoStream`，零依赖），后端在 `crates/airplay-renderer-gst`（子进程驱动 `gst-launch-1.0`）。缺的是 CLI 用**一个 feature** 把后端接上（默认构建不带它，这样没有 GStreamer 的机器仍然全绿），以及把 `media::audio` 的 `AudioFrame`、`media::mirror` 的 `VideoFrame` 送进渲染器。真机出声出画要在那之后测。

### 2b. S4-1b-3 时钟算术（有意未做，见第 7 节）

`media/timing.rs` 目前只有报文层（请求/应答/拒绝非法包）。时钟算术我试做后**回退**了：`raop_ntp_adjust_remote_timestamp_offset` 的"固定偏移"与 `sync_offset` 的漂移偏移之间的语义没有对到可验证的程度，落一个错的时钟信号比不落更糟。诚实说明、可复现的失败点与建议的下一步都在第 7 节。**不要凭直觉补一个"看起来对"的公式**：先把上游的语义读准，再写一个手工算好的固定向量。

### 2c. S4-3 / S4-4

见 [S4_WORK_PLAN.md](S4_WORK_PLAN.md)：RECORD/FLUSH/TEARDOWN 的媒体语义、volume/progress 真正作用到渲染层、SETUP 多流语义、HLS `playbin`；以及抓包回放与同硬件对比。

## 3. 已完成（不要在细节上重做）

- **S0–S2**：仓库/许可/CI 配置、协议原语、密码链与 PlayFair，均与上游 C 有 oracle 差分。
- **S3（已收口，见 [S3_ACCEPTANCE.md](S3_ACCEPTANCE.md)）**：plist（有界）、控制面路由与状态机、配对（PIN 三步 / legacy / pair-verify）、SETUP 密钥派生（FairPlay 解密 + `SHA512(aeskey‖ecdh)[..16]` 混合）、Digest 401（单次 nonce、跨连接失败计数）、`SET_PARAMETER`/`GET_PARAMETER`（缺 `Content-Type` 答 **451**）、发现广告（TXT 逐字段对齐上游）、`/info` 与广告同源、客户端注册表。**真机 iPad 实测**：完整请求序列与握手 63 ms 时序已归档。
- **S4 已完成**（16 个提交）：
  - `media/audio.rs`：RTP 音频包解析（上游布局、`00 68 34 00` 标记）、有界重排序窗口（512、缺口记录、远跳重开、序号回绕）、载荷解密（**只解密 `len/16*16` 对齐前缀**、**每包重置 IV**）；
  - `media/control.rs`：控制通道（解析 `0x56` 重发/`0x54` 同步；构造 8 字节重传请求 `0x80 0xd5`；重发音频走同一路径**真正补缺口**）；
  - `media/timing.rs`：时序报文层（32 字节请求 `0x80 0xd2 0x00 0x07` + 偏移 24 时间戳；应答三值偏移 8/16/24，`t2`/`t3` 是 **64 位** Q32.32）；
  - `media/ports.rs`：**三个 UDP 端口 + 镜像 TCP 监听**、收包循环（`select!` + 帧回调）、分包读取（含**短读**处理）、`MirrorStream::serve`；
  - `media/mirror.rs`：镜像密钥派生、128 字节包头与类型表、NAL 长度前缀→Annex-B 起始码就地改写与校验、**`MirrorDecryptor`（上游 CTR 规则，见下）**、`CodecConfig`（SPS/PPS 拼装规则）、`MirrorSession`（整条流→`VideoFrame`）、`stream_keys`；
  - **CLI 接线**：先绑端口再广告 → `SetupPorts` 声明真实端口（含镜像口）→ 收到带密钥的 SETUP 后启动音频收包循环与镜像 accept 循环。**真机现在会真的发包**（日志出现 `*** media session keyed, audio can arrive ***`）。
  - `crates/airplay-renderer`：渲染边界（零依赖、`NullRenderer`，默认 workspace 成员）；
  - `crates/airplay-renderer-gst`：GStreamer 后端（**独立 workspace**，子进程接入），6 条集成测试对真运行时通过。

### 镜像 CTR 规则（S4-1c-2，已与上游差分对齐）

`MirrorDecryptor` 按 `mirror_buffer_decrypt` 逐条实现，**密钥流由本模块从 IV 显式生成**（位置是规则的一部分，不能依赖 CTR 实现自己保存的状态）：

1. 每包把计数器移到块边界（`aes_ctr_start_fresh_block`），即**上一包尾块之后的那一块**；
2. 包头的 `carried` 字节用上一包尾块**末尾未用**的密钥流异或；
3. 尾部按**整块零填充**解密，本包只用前 `rest` 字节，留下 `16-rest` 字节给下一包；
4. 计数器按本包用掉的块数前进，**含尾块在内**（所以两包等长会读到同一批块）。

**验证方式**（这正是上一个会话失败的地方，别再退回旧办法）：`tools/oracle` 的 `mirror` 模式用上游自己的 `mirror_buffer_decrypt` 解密**全零包**，打印它**读到的密钥流位置**（全零异或密钥流就是密钥流），并做上游自身的往返校验；`tests/upstream_oracle.rs` 用同一批向量逐字节比对本移植。上一版错在两处：carry 取错了尾块里 16 字节的偏移，且计数器少前进一块。

## 4. S4 剩余路线

1. **S4-2 收尾**：CLI 接线（feature 隔离）+ 真机出声出画（见 §2a）。
2. **S4-1b-3** 时钟算术（见 §2b 与 §7）。
3. **S4-3** RECORD/FLUSH/TEARDOWN 的媒体语义、volume/progress 作用到渲染层、SETUP 多流语义、HLS `playbin`。
4. **S4-4** 抓包回放 + 同硬件对比（首帧/延迟/A-V 偏差/丢帧/CPU/内存）、长时播放与断线恢复。
5. 之后 S5–S8（见 MIGRATION_PLAN §3）。

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

- 媒体：**没有任何音频或画面被播放过**——渲染后端已落地并对着真 GStreamer 通过集成测试，但**还没接进 CLI**（S4-2 最后一段）。
- 真机：RTP 是否真的到达 data 口**尚未验证**（端口已声明、循环已启动，但没在 iPad 会话里确认过帧计数）；镜像口同理。
- 时钟：时序报文层完成，**时钟算术未做且我试做后回退**（见第 8 节第 9 条）。
- 渲染：`set_volume` 在 GStreamer 后端只记在内存里，**没有作用到运行中的管线**（要改管线属性，S4-3 做）；`flush` 是空操作（上游的语义也没读透）。
- 平台：macOS/Linux/BSD/ARM 未构建；`mdns-sd` 仅 Windows 实测；oracle 的 `build.ps1` Linux 分支未验证；CI 从未运行。
- **GStreamer 后端仅在 Windows + MSYS2/UCRT64 GStreamer 1.28.3 上测过**；macOS/Linux 的分支（库名、sink 名）没有实测。
- 工程：无 `deny.toml`/cargo-deny；无 fuzz 常态化（已有属性测试与深 bplist 回归护栏）。

## 8. 我已踩过的坑（省你时间）

1. **Windows 沙箱 ACL**：本工作区在受限模式下曾无法被授权（`grantWrite(...)` 失败），需要完全访问模式；确认 mDNS 广告可以用独立的 `C:\Windows\System32\dns-sd.exe`（是 `dns-sd`，不是 `dns-dns`）。
2. **不要用脚本盲替换改代码**：我因此毁过一次 `main.rs`（`println!` 替换命中错误位置），本次会话又用同样的方式毁过 `media/mirror.rs` 与 `media/timing.rs` 各一次（PowerShell 按行号切片 + 正则替换，插错位置导致重复模块/丢大括号）。**先读准区域，再用编辑工具；宁可整段重写一个函数，也不要用行号切片。**
3. **重排序窗口语义**："第一帧定起点"——我两次把用例写成"先到的高序号包会被挂起"，实际它反而定起点。
4. **NTP 时间基**：`ntp::unix_nanos_to_timestamp` 收 **Unix** 纳秒（自己加 NTP 纪元）；且 32 位小数**无法逐纳秒精确**，往返断言要允许 ≤2 ns。
5. **`Plist` 的 `get_*` 返回 `Result`**（不是 `Option`）；`rtp::parse` 是**自由函数**；`ParseError` 在 `wire` 模块。
6. **测试文件的服务端夹具**：`server_spec.rs` 里的 `TestServer.address` 是**字段**不是方法。
7. **文档链接**：`cargo doc -D warnings` 会因为指向未引入类型的 intra-doc 链接而失败（提交前跑 doc 门禁能当场发现）。
8. **`git commit -m` 里带引号/反斜杠**会被 shell 解析坏——写 `target/commit-*.txt` 用 `-F` 提交。
9. **时钟算术（S4-1b-3）我试做后回退了**，可复现的失败点：`sample()` 里两条腿 `t2-t1`、`t3-t4` 会把"客户端时钟差"消掉，如果固定偏移是按**第一次见到的远端时间戳**测的（上游 `raop_ntp_adjust_remote_timestamp_offset` 就是这么做的），那么当两端单向延迟对称时估计值恒为 0，与真实时钟差无关。我试了三种固定偏移的定标方式（用 t2 作锚、用 EPOCH 作锚、改采样公式为对差值取平均），都能让某个用例通过、却让另一个失败，说明我还没读懂上游的语义。**建议下一步**：把 `raop_ntp_thread` 的完整循环（含 `have_fixed_offset` 何时被置位、`data[]` 的 `t1_sent`/`t3_recv` 表如何被清理）与 `sync_offset` 的消费点（`raop_ntp_convert_remote_time` 的调用方）一起读，然后先写"上游在无延迟、有已知时钟差时算出什么"的手工向量，再实现。当前工作保存在 `git stash`（`wip: ntp clock arithmetic (unverified)`），可以 `git stash pop` 看当时的实现，但**不要直接用它**。
10. **子进程 stderr 用管道会阻塞**：GStreamer 出错时写得很多，没人读的管道会填满并把子进程卡在写操作上（本会话实测卡死）。`pipeline.rs` 因此把它重定向到文件；同理 `gst-launch` 的错误是**异步**报的，所以 `Pipeline::start` 会短暂 `settle()` 再检查它是否还活着。
11. **Windows 参数引号**：把整条管线描述作为一个参数传给 `gst-launch-1.0` 会被引号规则和 caps 里的逗号破坏（"erroneous pipeline: syntax error"）。`Pipeline::start` 用 `split_whitespace()` 逐个传参，`gst-launch` 会自己拼回去。
12. **嵌套 workspace 的 `target/` 不在根 `.gitignore` 覆盖范围**，第一次提交 GStreamer 后端时把 388 个构建产物提交了进去（已修复）。
