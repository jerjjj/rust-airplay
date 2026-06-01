# Rust AirPlay 镜像接收器 — 开发状态

> 最后更新: 2026-06-01
> 状态: ✅ 视频画面可正常解密播放

## 一、已完成功能 ✅

### 1. mDNS (Bonjour) 服务注册
- 注册 3 个服务: `_airplay._tcp.local` / `_raop._tcp.local` / `_airplay-bds._tcp.local`
- TXT 记录完整: features=0x5A7FFFF7, flags=0x0, pk=...
- 使用 `mdns-sd` crate，调用 `.enable_addr_auto()` 自动绑定 IP

### 2. RTSP 控制协议
- 纯自建 TCP 服务器（非 HTTP/axum，因 AirPlay 用 RTSP 协议）
- 正确处理分帧（Content-Length 解析）
- 支持端点: GET /info, POST /pair-setup, POST /pair-verify, POST /fp-setup, SETUP, RECORD, GET_PARAMETER, SET_PARAMETER, POST /feedback, TEARDOWN, OPTIONS

### 3. 配对 (Pairing)
- Ed25519 密钥生成 + Curve25519 ECDH
- AES-CTR 加密签名（两轮验证），iPad 可成功验证
- 关键修复:
  - iPad flag 是小端序 `[01,00,00,00]`
  - Round 2 keystream 需跳过前 64 字节
  - Ed25519 签名高位兼容 `sig_bytes[63] &= 0x1F`

### 4. FairPlay DRM 握手
- 3 轮 fp-setup 协议，4 组硬编码回复消息
- keyMsg (164字节) 从 Round 2 提取并保存

### 5. 二进制查找表
- 从 java-airplay JAR 提取 table_s1~s10 (≈114KB)
- Python 脚本 `convert_tables.py` 转换为 Rust 数组
- 存放在 `src/fairplay/rust_tables.rs` (6000+ 行)

### 6. C 代码集成
- HandGarble (UxPlay C), ModifiedMD5, OmgHax 全部编译为静态库
- `build.rs` 使用 `cc` crate 编译

---

## 二、已修复问题 ✅

### 问题 1: C 源码与 UxPlay 不一致（核心阻塞 ✅ 已修复）
**根因**: `omg_hax.c`、`hand_garble.c`、`modified_md5_c.c` 三个 C 文件与上游 UxPlay 的 SHA256 不一致。`omg_hax.h` 头文件一致。
**修复**: 从 https://github.com/FDH2/UxPlay 下载正确版本替换。
**影响**: 修复后 OmgHax 产生正确的 AES key，视频解密正常。

### 问题 2: `DEFAULT_SAP` 转录错误 ✅ 已修复
**根因**: Rust 常量中 `0xAA,0xBB` 被误写为 `0xAB`，丢失 1 字节导致 81 字节偏移。
**修复**: 更正为 `0xAA, 0xBB, 0xE4, 0x0F, ...`。

### 问题 3: 视频解密器初始化时序 ✅ 已修复
**根因**: `streams` SETUP 先于 `ekey` 到达时，用硬编码测试值初始化视频解密器。
**修复**: 移除测试值回退，改为延迟初始化（ekey 和 streams 任一到达时检查对方是否就绪）。

### 问题 4: `streamConnectionID` PLIST 解析 ✅ 已修复
**根因**: 只尝试 `as_signed_integer()`，ID 存为 unsigned 时解析失败→默认 0→KDF 错误。
**修复**: 先尝试 `as_unsigned_integer()`，再回退 `as_signed_integer()`。

---

## 三、关键修复历史

1. iPad flag 是小端序 → `u32::from_le_bytes`
2. Round 2 keystream 偏移 (跳过前 64 字节)
3. Ed25519 签名高位清理 `sig_bytes[63] &= 0x1F`
4. `enable_addr_auto()` clone bug
5. axum → 自建 TCP (AirPlay 用 RTSP)
6. streamConnectionID 有符号→无符号 (Java `Long.toUnsignedString`)
7. RECORD 误当作 TEARDOWN 清除 session
8. `ctr::Ctr128BE` vs `Ctr128LE` 都测试过

---

## 四、项目结构

```
E:\Projects\rust-airplay\
├── Cargo.toml
├── build.rs                          # C 代码编译
├── convert_tables.py                 # JAR 表格提取
├── verify.py                         # Python 解密验证
├── src/
│   ├── main.rs                       # 入口、硬编码 key 支持
│   ├── config.rs                     # 1920x1080@60
│   ├── mdns.rs                       # mDNS 注册
│   ├── pairing.rs                    # EdDSA + Curve25519
│   ├── session.rs                    # 会话状态
│   ├── test_values.rs                # 测试密钥常量
│   ├── rtsp/
│   │   ├── handler.rs                # RTSP TCP 服务器
│   │   ├── plist.rs                  # Binary plist
│   │   └── types.rs                  # 常量
│   ├── fairplay/
│   │   ├── mod.rs                    # fp-setup 处理
│   │   ├── omghax.rs                 # Rust OmgHax
│   │   ├── omghax_const.rs           # 常量 + include! 表格
│   │   ├── omghax_test.rs            # 测试向量
│   │   ├── omghax_debug.rs           # 调试测试
│   │   ├── rust_tables.rs            # 114KB 表格
│   │   ├── playfair_ffi.rs           # C FFI 声明
│   │   ├── sap_hash.rs               # sap_hash (C→Rust→C)
│   │   ├── modified_md5.rs           # ModifiedMD5
│   │   ├── video_decrypt.rs          # AES-CTR 视频解密
│   │   ├── audio_decrypt.rs          # AES-CBC 音频解密
│   │   ├── hand_garble.c             # HandGarble C 实现
│   │   ├── omg_hax.c                 # OmgHax C 实现 (UxPlay)
│   │   ├── omg_hax.h                 # 表格头文件 (483KB)
│   │   ├── modified_md5_c.c          # ModifiedMD5 C 实现
│   │   ├── playfair.c                # playfair_decrypt C 实现
│   │   └── playfair.h                # playfair 头
│   ├── stream/
│   │   ├── video.rs                  # TCP 视频服务器 → video.h264
│   │   ├── audio.rs                  # UDP 音频 → audio.alac
│   │   └── nal.rs                    # NAL→Annex B 转换
│   └── bin/
│       └── verify.rs                 # 视频解密验证工具
└── encrypted_frame.bin               # (运行时生成) 加密帧
```

---

## 五、下一步计划

1. **在 Linux (WSL/实体机) 上编译 UxPlay C 代码**，排除 MSVC 兼容问题
2. **对比 Rust 表格 vs C 表格**，确认数据完全一致
3. **如果 C 在 WSL 也错**，说明表格提取有问题
4. **备选方案**: 用 JNI 直接调用 java-airplay 的 OmgHax
5. 修复后视频解密应能正常工作

## 六、参考资料

- [java-airplay (GitHub)](https://github.com/serezhka/java-airplay)
- [UxPlay (GitHub)](https://github.com/FDH2/UxPlay)
- TECHNICAL_REPORT.md — AirPlay 协议分析
