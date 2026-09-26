# rust-airplay

将 [FDH2/UxPlay](https://github.com/FDH2/UxPlay) 的应用、协议、配对、密码处理和外围功能迁移到 Rust，保留 GStreamer 和必要的系统接口。目标是固定上游基线的功能对等，不是新增完整 AirPlay 2 能力。

**当前是分阶段开发中的协议核心，尚不能被 iPhone 连接或播放媒体。** 原版构建、密码链、实机互操作和媒体后端的完成状态以 [迁移计划](docs/MIGRATION_PLAN.md) 为准。

- [分阶段计划与验收](docs/MIGRATION_PLAN.md)
- [S3 工作计划与缺陷修复批次](docs/S3_WORK_PLAN.md)
- [ADR-0001：S3 网络、plist 与 DNS-SD 选型](docs/ADR-0001-network-stack.md)
- [架构与代码约定](docs/ARCHITECTURE.md)
- [源码可行性研究](research/uxplay-rust-feasibility.zh-CN.md)
- [上游基线及归属](THIRD_PARTY_NOTICES.md)

需要 Rust 1.85 或更新版本。协议核心阶段不依赖 GStreamer SDK；后续媒体后端会作为独立组件接入。

```powershell
cargo test --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p airplay-cli -- --help
```

所有里程碑以可审查 Git 提交交付；未通过实机测试的功能不会标注为“兼容完成”。当前尚未配置远程仓库，提交保存在本地。

项目代码按 GPL-3.0-or-later 发布。上游原有代码、数据表和文档的归属及适用许可需要继续保留，详见 notices。

