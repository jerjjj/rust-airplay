# rust-airplay

将 [FDH2/UxPlay](https://github.com/FDH2/UxPlay) 的应用、协议、配对、密码处理和外围功能迁移到 Rust，保留 GStreamer 和必要的系统接口。目标是固定上游基线的功能对等，不是新增完整 AirPlay 2 能力。

**当前是分阶段开发中的协议核心：可被发现、可完成配对与控制面会话，但还不能播放媒体。** 客户端能查到本接收端、走完配对并发出 `SETUP`，但接收端尚未绑定媒体端口，所以不会出声。原版构建、密码链、实机互操作和媒体后端的完成状态以 [迁移计划](docs/MIGRATION_PLAN.md) 为准。

- [**交接说明（新会话从这里开始）**](docs/HANDOFF.md)
- [全项目分阶段计划与验收](docs/MIGRATION_PLAN.md)
- [S3 工作计划与缺陷修复批次](docs/S3_WORK_PLAN.md)
- [S3 验收材料（含真机请求序列与时序）](docs/S3_ACCEPTANCE.md)
- [S4 工作计划：媒体会话与 GStreamer 渲染](docs/S4_WORK_PLAN.md)
- [ADR-0001：S3 网络、plist 与 DNS-SD 选型](docs/ADR-0001-network-stack.md)
- [ADR-0002：配对身份与持久化边界](docs/ADR-0002-pairing-identity.md)
- [ADR-0003：媒体后端先 GStreamer、为 FFmpeg 保留替换点](docs/ADR-0003-media-backend.md)
- [架构与代码约定](docs/ARCHITECTURE.md)
- [源码可行性研究](research/uxplay-rust-feasibility.zh-CN.md)
- [上游基线及归属](THIRD_PARTY_NOTICES.md)

需要 Rust 1.85 或更新版本。

**默认构建不链接任何 C 库**：`cargo test --workspace --all-targets` 在没有 GStreamer 的机器上就能全绿，这是 [ADR-0003](docs/ADR-0003-media-backend.md) 的要求。真正播放的媒体后端是**独立 crate**（`crates/airplay-renderer-gst`，不在 workspace 成员里），它**构建期需要 GStreamer 开发库**：视频走 `gst-launch-1.0` 子进程，音频则链接 `libgstreamer`（raw AAC-ELD 没有同步字，只能一帧一 buffer 地喂，见 [ADR-0003 第 6 条](docs/ADR-0003-media-backend.md)）。

```powershell
cargo test --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

# 媒体后端自己的门禁（需要 GStreamer 开发库，见 crates/airplay-renderer-gst/README.md）
cargo test  --manifest-path crates/airplay-renderer-gst/Cargo.toml
cargo clippy --manifest-path crates/airplay-renderer-gst/Cargo.toml --all-targets -- -D warnings

# 起一个接收端（默认一次性 PIN；Windows 上 7000 可能被 Hyper-V 保留，见下）
cargo run -p airplay-cli -- --name "Rust AirPlay Test" --port 47000 --log session.log

# 出声出画要带 gst feature；**用 --release**，debug 下解码与解密慢一个数量级
cargo run --release -p airplay-cli --features gst -- --port 47000 --render gst

# 另一终端确认广告已发布（需系统 Bonjour；Linux 可用 avahi-browse -r _raop._tcp）
dns-sd -B _raop._tcp local
```

`--port 0` 会自动挑一个空闲端口，广告里用的是实际绑定端口。身份种子存放在 `%LOCALAPPDATA%\rust-airplay\identity.seed`，删除它等于换一个接收端身份，客户端需要重新配对；已验证过的客户端公钥记在 `%LOCALAPPDATA%\rust-airplay\clients`（每行一个，可手工清理以强制重新配对），两者都由 CLI 这一配置层持有——协议库本身不碰文件系统。Windows 上若 `--port` 报 `os error 10013`，是 Hyper-V/WSL 保留了该端口段（`netsh int ipv4 show excludedportrange protocol=tcp` 可查），换一个端口即可。

所有里程碑以可审查 Git 提交交付；未通过实机测试的功能不会标注为“兼容完成”。当前尚未配置远程仓库，提交保存在本地。

### 反复构建很慢时：先看接收端是不是还在跑

接收端是常驻进程，而 Windows 上**正在运行的可执行文件不能被覆盖**。如果上一次 `cargo run` 起的接收端还在（另一个窗口、或没有 Ctrl+C），下一次构建会在链接阶段失败：

```
error: linking with `link.exe` failed: exit code: 1104
LINK : fatal error LNK1104: 无法打开文件 "...\target\release\deps\rust_airplay.exe"
```

cargo 会把这次编译判为失败，**下一个命令再重做同样的编译与链接**——于是"每次都要等很久"，而且常常只看到一句被刷过去的错误。查一下有没有残留进程：

```powershell
Get-Process rust-airplay -ErrorAction SilentlyContinue | Select-Object Id, StartTime
```

长期跑接收端又同时改代码时，把二进制复制出来运行，构建就不会再去碰它：

```powershell
cargo build --release -p airplay-cli --features gst
Copy-Item target\release\rust-airplay.exe "$env:TEMP\airplay-receiver.exe" -Force
& "$env:TEMP\airplay-receiver.exe" --port 47000 --render gst
```

文件不被占用时，一次"改一行再跑"的成本是：**dev 约 2 秒，release 约 3 秒**（其中链接分别约 0.35 秒和 3 秒；release 的链接要处理整张 GStreamer + tokio 的依赖图）。日常改动用 `cargo test` / dev 构建迭代，只有要真机验收时才用 `--release`。

项目代码按 GPL-3.0-or-later 发布。上游原有代码、数据表和文档的归属及适用许可需要继续保留，详见 notices。

