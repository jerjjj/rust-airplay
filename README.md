# rust-airplay

将 [FDH2/UxPlay](https://github.com/FDH2/UxPlay) 的应用、协议、配对、密码处理和外围功能迁移到 Rust，保留 GStreamer 和必要的系统接口。目标是固定上游基线的功能对等，不是新增完整 AirPlay 2 能力。

**当前是分阶段开发中的协议核心：可被发现、可完成配对与控制面会话，但还不能播放媒体。** 客户端能查到本接收端、走完配对并发出 `SETUP`，但接收端尚未绑定媒体端口，所以不会出声。原版构建、密码链、实机互操作和媒体后端的完成状态以 [迁移计划](docs/MIGRATION_PLAN.md) 为准。

- [分阶段计划与验收](docs/MIGRATION_PLAN.md)
- [S3 工作计划与缺陷修复批次](docs/S3_WORK_PLAN.md)
- [ADR-0001：S3 网络、plist 与 DNS-SD 选型](docs/ADR-0001-network-stack.md)
- [ADR-0002：配对身份与持久化边界](docs/ADR-0002-pairing-identity.md)
- [ADR-0003：媒体后端先 GStreamer、为 FFmpeg 保留替换点](docs/ADR-0003-media-backend.md)
- [架构与代码约定](docs/ARCHITECTURE.md)
- [源码可行性研究](research/uxplay-rust-feasibility.zh-CN.md)
- [上游基线及归属](THIRD_PARTY_NOTICES.md)

需要 Rust 1.85 或更新版本。协议核心阶段不依赖 GStreamer SDK；后续媒体后端会作为独立组件接入。

```powershell
cargo test --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

# 起一个接收端（默认一次性 PIN；Windows 上 7000 可能被 Hyper-V 保留，见下）
cargo run -p airplay-cli -- --name "Rust AirPlay Test" --port 47000

# 另一终端确认广告已发布（需系统 Bonjour；Linux 可用 avahi-browse -r _raop._tcp）
dns-sd -B _raop._tcp local
```

`--port 0` 会自动挑一个空闲端口，广告里用的是实际绑定端口。身份种子存放在 `%LOCALAPPDATA%\rust-airplay\identity.seed`，删除它等于换一个接收端身份，客户端需要重新配对；已验证过的客户端公钥记在 `%LOCALAPPDATA%\rust-airplay\clients`（每行一个，可手工清理以强制重新配对），两者都由 CLI 这一配置层持有——协议库本身不碰文件系统。Windows 上若 `--port` 报 `os error 10013`，是 Hyper-V/WSL 保留了该端口段（`netsh int ipv4 show excludedportrange protocol=tcp` 可查），换一个端口即可。

所有里程碑以可审查 Git 提交交付；未通过实机测试的功能不会标注为“兼容完成”。当前尚未配置远程仓库，提交保存在本地。

项目代码按 GPL-3.0-or-later 发布。上游原有代码、数据表和文档的归属及适用许可需要继续保留，详见 notices。

