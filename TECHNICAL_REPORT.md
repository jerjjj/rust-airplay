# Java AirPlay Server — 技术实现深度分析报告

> **目的**: 为 `rust-airplay` 项目提供完整的协议级参考，覆盖 AirPlay 镜像协议的全部技术细节。
>
> **来源**: [serezhka/java-airplay](https://github.com/serezhka/java-airplay) v1.0.7
>
> **生成日期**: 2025-07-18

---

## 目录

1. [协议概览](#1-协议概览)
2. [mDNS 服务发现 (Bonjour)](#2-mdns-服务发现-bonjour)
3. [配对流程 (Pairing)](#3-配对流程-pairing)
4. [FairPlay DRM 密钥交换](#4-fairplay-drm-密钥交换)
5. [RTSP 控制协议](#5-rtsp-控制协议)
6. [视频流处理](#6-视频流处理)
7. [音频流处理](#7-音频流处理)
8. [HLS / YouTube 播放模式](#8-hls--youtube-播放模式)
9. [密码学实现细节](#9-密码学实现细节)
10. [网络架构](#10-网络架构)
11. [Rust 实现建议](#11-rust-实现建议)

---

## 1. 协议概览

Apple AirPlay 镜像协议允许 iOS/macOS 设备将屏幕内容无线投射到接收端。整个协议栈分为三层：

```
┌──────────────────────────────────────────────────┐
│  应用层: RTSP (控制) + binary plist (序列化)       │
├──────────────────────────────────────────────────┤
│  安全层: EdDSA + Curve25519 ECDH (配对)           │
│          FairPlay + OmgHax (密钥交换)             │
│          AES-CTR (视频) / AES-CBC (音频) (流加密)  │
├──────────────────────────────────────────────────┤
│  传输层: TCP (控制+视频) + UDP (音频+音频控制)      │
│  发现层: mDNS (Bonjour)                           │
└──────────────────────────────────────────────────┘
```

### 完整协议时序

```
iOS 设备                         java-airplay 服务端
   │                                    │
   │  ① mDNS 查询 _airplay._tcp.local   │
   │◄───────────────────────────────────│  (注册服务)
   │                                    │
   │  ② RTSP GET /info                  │
   │───────────────────────────────────▶│  返回设备能力
   │                                    │
   │  ③ POST /pair-setup                │
   │───────────────────────────────────▶│  交换 EdDSA 公钥
   │                                    │
   │  ④ POST /pair-verify (第1次)       │
   │───────────────────────────────────▶│  ECDH 公钥 + EdDSA 公钥
   │◄───────────────────────────────────│  ECDH 公钥 + 加密签名
   │                                    │
   │  ⑤ POST /pair-verify (第2次)       │
   │───────────────────────────────────▶│  加密签名 (验证)
   │◄───────────────────────────────────│  OK
   │                                    │
   │  ⑥ POST /fp-setup (3次往返)        │
   │───────────────────────────────────▶│  FairPlay 密钥协商
   │                                    │
   │  ⑦ RTSP SETUP (发送加密密钥)        │
   │───────────────────────────────────▶│  ekey + eiv
   │                                    │
   │  ⑧ RTSP SETUP (协商流端口)          │
   │───────────────────────────────────▶│  streamConnectionID
   │◄───────────────────────────────────│  dataPort + controlPort
   │                                    │
   │  ⑨ TCP 视频流 (端口=dataPort)       │
   │═══════════════════════════════════▶│  AES-CTR 加密 H.264
   │                                    │
   │  ⑩ UDP 音频流 (端口=dataPort)       │
   │═══════════════════════════════════▶│  AES-CBC 加密 ALAC/AAC-ELD
   │                                    │
   │  ⑪ RTSP TEARDOWN                   │
   │───────────────────────────────────▶│  断开流
```

---

## 2. mDNS 服务发现 (Bonjour)

### 2.1 注册的服务

服务端使用 JmDNS 库注册两个 mDNS 服务：

#### AirPlay 视频服务 `_airplay._tcp.local`

```
服务名: {serverName}@_airplay._tcp.local
端口:   airTunesPort (与控制端口相同, 随机绑定)
TXT 记录:
```

| 键 | 值 | 说明 |
|---|---|---|
| `deviceid` | MAC 地址 (XX:XX:XX:XX:XX:XX) | 设备唯一标识 |
| `features` | `0x5A7FFFF7,0x1E` | 功能位掩码 |
| `srcvers` | `220.68` | 源版本 (伪装 Apple TV) |
| `flags` | `0x44` | 标志位 |
| `vv` | `2` | 协议版本 |
| `model` | `AppleTV3,2C` | 设备型号 |
| `rhd` | `5.6.0.0` | 远程硬件版本 |
| `pw` | `false` | 是否需要密码 |
| `pk` | `f3769a660475...` | 公钥哈希 (固定值, 不重要) |
| `rmodel` | `PC1.0` | 远程型号 |
| `rrv` | `1.01` | 接收端版本 |
| `rsv` | `1.00` | 接收端软件版本 |
| `pcversion` | `1715` | 协议兼容版本 |

#### AirTunes 音频服务 `_raop._tcp.local`

```
服务名: {MAC无冒号}@{serverName}@_raop._tcp.local
端口:   与 AirPlay 相同
TXT 记录:
```

| 键 | 值 | 说明 |
|---|---|---|
| `ch` | `2` | 声道数 |
| `cn` | `1,3` | 编码 |
| `da` | `true` | 数字音频 |
| `et` | `0,3,5` | 加密类型 |
| `ek` | `1` | 加密密钥 |
| `ft` | `0x5A7FFFF7,0x1E` | 功能 (同 AirPlay) |
| `sr` | `44100` | 采样率 |
| `ss` | `16` | 采样位深 |
| `tp` | `UDP` | 传输协议 |
| `txtvers` | `1` | TXT 版本 |

### 2.2 关键 features 位掩码 `0x5A7FFFF7`

此掩码控制 iOS 设备的行为选择：

```
0x5A7FFFF7 = 0101 1010 0111 1111 1111 1111 1111 0111

Bit  0: 支持视频
Bit  1: 支持照片
Bit  2: 支持音频
Bit  4: 支持屏幕镜像
Bit  5: 支持音频冗余
Bit  7: 支持 HLS
Bit  8: 支持加密
Bit 27: 支持 FairPlay 2
Bit 29: 支持 FairPlay 1
...etc
```

第二个 features `0x1E` 控制其他扩展功能。

---

## 3. 配对流程 (Pairing)

配对使用 **EdDSA (Ed25519)** + **Curve25519 ECDH** 实现双向认证和密钥协商。

### 3.1 密码学组件

| 算法 | 用途 |
|---|---|
| **Ed25519** | 设备签名与验证 |
| **Curve25519 ECDH** | 密钥协商 (生成共享密钥) |
| **SHA-512** | 密钥派生 (KDF) |
| **AES-CTR-128** | 签名加密传输 |

### 3.2 预置密钥

服务端在启动时生成 EdDSA 密钥对，公钥在 `/pair-setup` 中提供给客户端。

### 3.3 协议流程

#### 步骤 1: `POST /pair-setup` — 获取服务端 EdDSA 公钥

```
请求:  (空 body)
响应: 32 bytes EdDSA 公钥 (Ed25519 的 A 值)
```

#### 步骤 2: `POST /pair-verify` (第 1 次) — ECDH 密钥交换

```
请求格式 (68 bytes):
┌────────┬──────────────────────────────────────────┐
│ Offset │ 内容                                      │
├────────┼──────────────────────────────────────────┤
│  0     │ flag = 0x01 (4 bytes, 大端: 0x00000001)   │
│  4     │ 客户端 Curve25519 公钥 (32 bytes)          │
│ 36     │ 客户端 EdDSA 公钥 (32 bytes)               │
└────────┴──────────────────────────────────────────┘

响应格式 (96 bytes):
┌────────┬──────────────────────────────────────────┐
│ Offset │ 内容                                      │
├────────┼──────────────────────────────────────────┤
│  0     │ 服务端 Curve25519 公钥 (32 bytes)          │
│ 32     │ 加密的 EdDSA 签名 (64 bytes)               │
└────────┴──────────────────────────────────────────┘
```

**密钥派生过程 (双方)**:

```
1. sharedSecret = Curve25519.agree(对端公钥, 己方私钥)  // 32 bytes

2. sha512 = SHA-512()
   sha512.update("Pair-Verify-AES-Key")
   sha512.update(sharedSecret)
   aesKey = sha512.digest()[0..16]    // 前 16 bytes

3. sha512.reset()
   sha512.update("Pair-Verify-AES-IV")
   sha512.update(sharedSecret)
   aesIV = sha512.digest()[0..16]     // 前 16 bytes

4. cipher = AES-CTR-128(aesKey, aesIV)
```

**服务端构建签名数据**:

```
dataToSign = 己方Curve25519公钥 || 对方Curve25519公钥  // 64 bytes
signature  = EdDSA.sign(dataToSign, 己方EdDSA私钥)   // 64 bytes
encryptedSig = cipher.encrypt(signature)              // 64 bytes
```

#### 步骤 3: `POST /pair-verify` (第 2 次) — 签名验证

```
请求格式 (68 bytes):
┌────────┬──────────────────────────────────────────┐
│ Offset │ 内容                                      │
├────────┼──────────────────────────────────────────┤
│  0     │ flag = 0x00 (4 bytes)                     │
│  4     │ 客户端加密 EdDSA 签名 (64 bytes)            │
└────────┴──────────────────────────────────────────┘

响应: 空 body (仅状态码)
```

**服务端验证**:

```
1. 用相同的 cipher 解密客户端的 encryptedSig → signature (64 bytes)
2. sigMessage = 对方Curve25519公钥 || 己方Curve25519公钥  // 64 bytes
3. EdDSA.verify(sigMessage, signature, 对方EdDSA公钥)
4. 成功 → pairVerified = true
```

### 3.4 Rust 实现关键点

- `ed25519-dalek` 提供 EdDSA
- `x25519-dalek` 提供 Curve25519 (注意: 这是 DH 函数, 不是 Ed25519 签名)
- `aes` + `ctr` crate 提供 AES-CTR
- `sha2` crate 提供 SHA-512

```rust
// Curve25519 DH 公钥 ≠ Ed25519 签名公钥!
// 它们是两种不同的密钥对, 虽然底层曲线相同 (Curve25519)
let ecdh_secret = x25519_private.diffie_hellman(&peer_ecdh_public);
let signature   = ed25519_private.sign(&data_to_sign);
```

---

## 4. FairPlay DRM 密钥交换

FairPlay 是 Apple 的数字版权管理方案。AirPlay 镜像使用 FairPlay 保护视频/音频流。

### 4.1 协议阶段

FairPlay 分为 3 次消息交换, 每次都是 12 字节头 + 可变数据:

```
FairPlay 消息格式:
┌────────┬──────────────────────────────────────────┐
│ Offset │ 内容                                      │
├────────┼──────────────────────────────────────────┤
│  0-3   │ 魔法数字 "FPLY" (0x46 0x50 0x4C 0x59)     │
│  4     │ 协议版本 (固定 0x03)                       │
│  5     │ 消息类型 (0x01=setup1, 0x02=setup2, ...)   │
│  6     │ 保留 (通常 0x01)                           │
│  7     │ 方向/标志                                  │
│  8-11  │ 保留 (0x00000000)                          │
│ 12-13  │ payload 长度 (大端, 不含头)                 │
│ 14+    │ payload                                    │
└────────┴──────────────────────────────────────────┘
```

### 4.2 三次往返

#### 第 1 次: Client → Server (16 bytes total)

```
Header: FPLY 03 01 01 00 00 00 00 00 04
Payload: 02 00 00 BB
```

服务端根据 `payload[2]` (即 `mode`) 选择预置的 `replyMessage` 之一进行响应:

- **mode = 0**: 142 bytes 硬编码响应
- **mode = 1**: 142 bytes 硬编码响应
- **mode = 2**: 142 bytes 硬编码响应
- **mode = 3**: 142 bytes 硬编码响应

所有 4 个 `replyMessage` 都是预置的常量数组 (见 `FairPlay.java`)。

#### 第 2 次: Client → Server (164 bytes total)

```
Header: FPLY 03 01 03 00 00 00 00 00 98
Payload: 152 bytes (含加密的 key material)
```

服务端:
1. 将 164 bytes 全部消息保存为 `keyMsg[0..164]`
2. 返回 `"FPLY" + 03 01 04 00 00 00 00 00 14 + payload[144..164]` (即最后 20 bytes)

#### 第 3 次: (隐含) AES 密钥解密

服务端收到 RTSP SETUP 中的 `ekey` 后, 调用 `OmgHax.decryptAesKey()` 从 `keyMsg` + `ekey` 中提取真正的 AES 密钥。

### 4.3 OmgHax 解密算法

`OmgHax.decryptAesKey(keyMsg, cipherText, keyOut)`:

```
1. chunk1 = cipherText[16..]
2. chunk2 = cipherText[56..]
3. blockIn = [0u8; 16]

4. generate_session_key(default_sap, keyMsg, sapKey)
   └── 内部调用 decryptMessage() + ModifiedMD5 + SapHash 等

5. generate_key_schedule(sapKey, key_schedule)
   └── 生成 11 轮密钥表

6. z_xor(chunk2, blockIn, 1)      // XOR with z_key constant
7. cycle(blockIn, key_schedule)    // 10 轮自定义分组密码
8. keyOut[i] = blockIn[i] ^ chunk1[i]  // 提取 AES 密钥
9. x_xor(keyOut, keyOut, 1)        // XOR with x_key constant
10. z_xor(keyOut, keyOut, 1)       // XOR with z_key constant
```

**核心 `cycle()` 函数** 是一个 10 轮的分组密码:
- 每轮包含 S-Box 查表 (`table_s5` ~ `table_s8`) + 置换 (`permute_block_1` / `permute_block_2`)
- 轮密钥来自 `key_schedule[10-round]`
- 最后一轮不执行置换, 而是与 `key_schedule[0]` 异或

**关键常量** (在 `OmgHaxConst.java` 中):
- `table_s1` ~ `table_s10`: 大查找表 (每个 256xN bytes)
- `z_key`, `x_key`, `t_key`: 16 bytes 固定 XOR 密钥
- `message_key[4][144]`: 消息解密密钥表 (4 种模式)
- `message_iv[4][16]`: 消息解密 IV (4 种模式)
- `default_sap`: 320 bytes 默认 SAP
- `static_source_1` (17 bytes), `static_source_2` (47 bytes): 静态源数据
- `initial_session_key` (16 bytes): 初始会话密钥
- `index_mangle[11]`: 轮索引混淆值

### 4.4 Rust 实现策略

OmgHax 是逆向工程产物, 最直接的方案是：
1. **将常量表导出为 Rust 的 `[u8; N]` 数组** (从 Java 的 `table_s1` ~ `table_s10` 复制)
2. **逐行翻译** `cycle()`, `permute_block_1()`, `permute_block_2()` 等函数
3. 或使用 `include_bytes!` 从二进制 blob 加载

---

## 5. RTSP 控制协议

### 5.1 数据序列化: Apple Binary Property List (bplist)

所有 RTSP 消息体使用 Apple 的 **binary plist** 格式。Java 使用 `dd-plist` 库。Rust 可使用 `plist` crate。

### 5.2 GET /info — 设备能力协商

响应内容 (binary plist):

```xml
<dict>
    <key>audioFormats</key>
    <array>
        <dict>
            <key>audioInputFormats</key><integer>67108860</integer>
            <key>audioOutputFormats</key><integer>67108860</integer>
            <key>type</key><integer>100</integer>
        </dict>
        <dict>
            <key>audioInputFormats</key><integer>67108860</integer>
            <key>audioOutputFormats</key><integer>67108860</integer>
            <key>type</key><integer>101</integer>
        </dict>
    </array>
    <key>audioLatencies</key>
    <array>
        <dict><key>audioType</key><string>default</string>
              <key>inputLatencyMicros</key><false/>
              <key>type</key><integer>100</integer></dict>
        <dict><key>audioType</key><string>default</string>
              <key>inputLatencyMicros</key><false/>
              <key>type</key><integer>101</integer></dict>
    </array>
    <key>displays</key>
    <array>
        <dict>
            <key>features</key><integer>14</integer>
            <key>height</key><integer>{config.height}</integer>
            <key>heightPhysical</key><false/>
            <key>heightPixels</key><integer>{config.height}</integer>
            <key>maxFPS</key><integer>{config.fps}</integer>
            <key>overscanned</key><false/>
            <key>refreshRate</key><integer>60</integer>
            <key>rotation</key><false/>
            <key>uuid</key><string>e5f7a68d-7b0f-4305-984b-974f677a150b</string>
            <key>width</key><integer>{config.width}</integer>
            <key>widthPhysical</key><false/>
            <key>widthPixels</key><integer>{config.width}</integer>
        </dict>
    </array>
    <key>features</key><integer>130367356919</integer>
    <key>keepAliveSendStatsAsBody</key><integer>1</integer>
    <key>model</key><string>AppleTV3,2</string>
    <key>name</key><string>Apple TV</string>
    <key>pi</key><string>b08f5a79-db29-4384-b456-a4784d9e6055</string>
    <key>sourceVersion</key><string>220.68</string>
    <key>statusFlags</key><integer>68</integer>
    <key>vv</key><integer>2</integer>
</dict>
```

**关键点:**
- `features = 130367356919` (0x1E5A7FFFF7) — 当 features 较大时, iOS 走 HTTP fp-setup 路径; 较小时 (`119`) 走 RTSP fp-setup 路径
- `audioFormats = 67108860` (0x3FFFDFC) — 支持的音频格式位掩码

### 5.3 RTSP SETUP — 流协商

这是最关键的控制消息, 有**两种形式**:

#### 形式 1: 发送加密密钥 (Client → Server)

```xml
<dict>
    <key>ekey</key>
    <data> <!-- 72 bytes FairPlay 加密的 AES 密钥 + 元数据 --> </data>
    <key>eiv</key>
    <data> <!-- base64 编码的 IV, 如 "91IdM6RTh4keicMei2GfQA==" --> </data>
</dict>
```

`ekey` 的 72 bytes 结构:
```
Offset  0-15:  FairPlay header (含 "FPLY" 魔法)
Offset 16-31:  ???
Offset 32-47:  加密的 AES 密钥 (16 bytes)
Offset 48-71:  ???
```

**解析**: `ekey[32..48]` 是 cipherText, `ekey[16..]` 是 chunk1, `ekey[56..]` 是 chunk2。通过 `OmgHax.decryptAesKey()` 解密得到真正的 16 bytes AES 密钥。

#### 形式 2: 协商流端口 (Client → Server, Server 返回端口)

```xml
<!-- 请求: 视频流 -->
<dict>
    <key>streams</key>
    <array>
        <dict>
            <key>type</key><integer>110</integer>          <!-- 110 = 视频 -->
            <key>streamConnectionID</key><integer>{id}</integer>
        </dict>
    </array>
</dict>

<!-- 请求: 音频流 -->
<dict>
    <key>streams</key>
    <array>
        <dict>
            <key>type</key><integer>96</integer>           <!-- 96 = 音频 -->
            <key>ct</key><integer>2</integer>              <!-- ALAC -->
            <key>audioFormat</key><integer>0x40000</integer>
            <key>spf</key><integer>352</integer>           <!-- samples per frame -->
        </dict>
    </array>
</dict>
```

**响应 (视频)**:
```xml
<dict>
    <key>streams</key>
    <array>
        <dict>
            <key>dataPort</key><integer>{videoServerPort}</integer>
            <key>type</key><integer>110</integer>
        </dict>
    </array>
    <key>eventPort</key><integer>{controlServerPort}</integer>
    <key>timingPort</key><integer>0</integer>
</dict>
```

**响应 (音频)**:
```xml
<dict>
    <key>streams</key>
    <array>
        <dict>
            <key>dataPort</key><integer>{audioServerPort}</integer>
            <key>type</key><integer>96</integer>
            <key>controlPort</key><integer>{audioControlPort}</integer>
        </dict>
    </array>
</dict>
```

### 5.4 流类型枚举

| Type | 含义 | 说明 |
|---|---|---|
| `96` | 音频流 | 需要 `ct` (compression type), `audioFormat`, `spf` |
| `110` | 视频流 | 需要 `streamConnectionID` |

### 5.5 音频压缩类型 (ct)

| Code | 名称 | 说明 |
|---|---|---|
| `1` | LPCM | 线性 PCM |
| `2` | ALAC | Apple Lossless |
| `4` | AAC | 标准 AAC |
| `8` | AAC_ELD | 增强低延迟 AAC |
| `32` | OPUS | Opus 编码 |

### 5.6 音频格式 (audioFormat) 位掩码

每个格式用一个唯一位表示, 组合了采样率×位深×声道:

| 格式 | Code | 含义 |
|---|---|---|
| PCM_44100_16_2 | 0x800 | 44.1kHz, 16bit, 立体声 |
| ALAC_44100_16_2 | 0x40000 | ALAC 44.1kHz, 16bit, 立体声 |
| ALAC_48000_16_2 | 0x100000 | ALAC 48kHz, 16bit, 立体声 |
| AAC_ELD_44100_2 | 0x1000000 | AAC-ELD 44.1kHz, 立体声 |
| AAC_ELD_48000_2 | 0x2000000 | AAC-ELD 48kHz, 立体声 |
| OPUS_48000_1 | 0x40000000 | Opus 48kHz, 单声道 |

---

## 6. 视频流处理

### 6.1 传输格式

视频通过 **TCP** 传输, 每个数据包结构为:

```
┌──────────┬──────────────────────────────────────────┐
│  Offset  │  内容                                      │
├──────────┼──────────────────────────────────────────┤
│   0-3    │  payloadSize (u32, little-endian)           │
│   4-5    │  payloadType (u16, little-endian, 取低8位)  │
│   6-7    │  payloadOption (未使用)                     │
│   8-15   │  timestamp (u64, little-endian, 未使用)     │
│  16-127  │  padding (填充到 128 bytes)                 │
├──────────┼──────────────────────────────────────────┤
│ 128+     │  payload (payloadSize bytes)               │
└──────────┴──────────────────────────────────────────┘

总包头: 固定 128 bytes (Netty ByteBuf 初始容量)
```

### 6.2 Payload 类型

| Type | 含义 | 处理方式 |
|---|---|---|
| `0` | 加密的 H.264 图像数据 (IDR/P/B 帧) | 解密 → 重组 NAL 单元 → 输出 |
| `1` | SPS/PPS 参数集 (未加密) | 解析 SPS/PPS → 重组为 Annex B → 输出 |
| 其他 | 未知 | 跳过 |

### 6.3 Payload 解密 (AES-CTR)

```java
// FairPlayVideoDecryptor.decrypt(video)
// 密钥派生:
sha512.reset(); sha512.update(aesKey); sha512.update(sharedSecret);
eaesKey = sha512.digest();  // 64 bytes

sha512.reset(); sha512.update("AirPlayStreamKey" + streamConnectionID);
sha512.update(eaesKey[0..16]); hash1 = sha512.digest();
decryptKey = hash1[0..16];

sha512.reset(); sha512.update("AirPlayStreamIV" + streamConnectionID);
sha512.update(eaesKey[0..16]); hash2 = sha512.digest();
decryptIV = hash2[0..16];

// AES-CTR 解密:
cipher = AES-CTR(decryptKey, decryptIV)
cipher.decrypt(video)
```

**CTR 溢出处理**: 解密后如果剩余字节不足 16 bytes, 用 `og` 缓冲暂存, 累积到下一个包的前 `nextDecryptCount` 字节进行 XOR。

### 6.4 NAL 单元重组

Type 0 payload 解密后包含一个或多个 NAL 单元, 每个以 4 bytes **大端** 长度前缀:

```
┌──────────┬──────────────┐
│ 4 bytes  │ NALU 数据     │
│ big-endian length       │
└──────────┴──────────────┘ ... 重复
```

重组步骤:
1. 遍历 payload
2. 读取 4 bytes 大端长度 → `naluSize`
3. 如果 `naluSize == 1`, 停止
4. 将 4 bytes 长度前缀替换为 **Annex B 起始码** `00 00 00 01`
5. 跳过 `naluSize` bytes 到下一个 NALU
6. 将 `00 00 00 01` 起始码的完整 Annex B 数据传给播放器

### 6.5 SPS/PPS 提取 (Type 1)

Type 1 payload 格式 (未加密):

```
Offset  0-1:  保留
Offset  2-3:  保留
Offset  4-5:  保留
Offset  6-7:  spsLen (u16, 大端)
Offset  8+:   SPS 数据 (spsLen bytes)
              ppsCount (1 byte, 跳过)
              ppsLen (u16, 大端)
              PPS 数据 (ppsLen bytes)
```

重组为 Annex B:
```
[00 00 00 01] + SPS + [00 00 00 01] + PPS
```

---

## 7. 音频流处理

### 7.1 传输格式

音频通过 **UDP** 传输, 每个数据包格式 (RTP-like):

```
┌────────┬──────────────────────────────────────────┐
│ Offset │ 内容                                      │
├────────┼──────────────────────────────────────────┤
│  0     │ flag (1 byte)                             │
│  1     │ type = header[1] & 0x7F (1 byte)          │
│  2-3   │ sequenceNumber (u16, 大端)                 │
│  4-7   │ timestamp (u32, 大端)                      │
│  8-11  │ SSRC (u32, 大端)                           │
├────────┼──────────────────────────────────────────┤
│ 12+    │ 加密的音频数据 (AES-CBC)                    │
└────────┴──────────────────────────────────────────┘

总包头: 12 bytes
```

### 7.2 Payload 解密 (AES-CBC)

与视频不同, 音频使用 **AES-CBC** 模式:

```java
// FairPlayAudioDecryptor
// 密钥派生 (简化):
sha512.reset(); sha512.update(aesKey); sha512.update(sharedSecret);
eaesKey = sha512.digest()[0..16];  // 仅取前 16 bytes

// 解密:
cipher = AES-CBC(eaesKey, eiv)  // eiv 来自 RTSP SETUP
cipher.decrypt(audio, audioLength / 16 * 16)  // 只解密 16 字节对齐部分
```

### 7.3 音频重排序缓冲

`AudioHandler` 使用 512 槽环形缓冲区处理 UDP 乱序:

```
buffer: [AudioPacket; 512]
prevSeqNum: 上次处理的序列号
packetsInBuffer: 缓冲中等待的包数

算法:
1. 收到 seq=N 的包 → buffer[N % 512] = packet
2. while dequeue():
   - 检查是否有 seq = prevSeqNum + 1 的包
   - 解密 → 输出 → prevSeqNum++
```

---

## 8. HLS / YouTube 播放模式

当 `features` 较大 (`130367356919`) 时, iOS 可能使用 HLS 模式而非镜像模式。此项目仅实现了 YouTube 的 HLS 代理。

### 8.1 连接升级 (HTTP Upgrade)

```
Client: POST /reverse
        Headers: Upgrade: PTTH/1.0
                 Connection: Upgrade
                 X-Apple-Purpose: event

Server: 101 Switching Protocols
        Headers: Upgrade: PTTH/1.0
                 Connection: Upgrade
```

升级后, 服务端移除 RTSP 编解码器, 添加 HTTP 编解码器, 建立反向连接通道。

### 8.2 YouTube 播放流程

```
1. Client → Server: POST /play
   Body: { "Content-Location": "mlhls://localhost/.../master.m3u8",
           "clientProcName": "YouTube" }

2. Server → Client (on reverse channel):
   POST /event
   Body: { "type": "unhandledURLRequest",
           "request": { "FCUP_Response_URL": "mlhls://localhost/.../master.m3u8" } }

3. Client → Server: POST /action
   Body: { "type": "unhandledURLResponse",
           "params": {
             "FCUP_Response_URL": "...",
             "FCUP_Response_Data": "<base64 编码的远端 m3u8 内容>"
           }}

4. Server: 将远端 m3u8 中的 URL 重写为本地 http://localhost:{port}/playlist?session={id}
   通过请求上下文返回给 Client
```

### 8.3 m3u8 URL 重写

- 远端 URL: `mlhls://localhost/itag/233/mediadata.m3u8`
- 本地 URL: `http://localhost:{port}/playlist/itag/233/mediadata.m3u8?session={sessionId}`

当 Client 请求本地 playlist URL 时, Server 逆向发出 FCUP 请求获取真实内容, 将响应中的远端 URL 全部替换为本地 URL 后返回。

---

## 9. 密码学实现细节

### 9.1 密钥层次结构

```
                    EdDSA 密钥对 (长期)
                         │
                    /pair-verify
                         │
              Curve25519 ECDH sharedSecret
                         │
                   SHA-512 KDF
                         │
              ┌──────────┼──────────┐
              │                     │
         AES-CTR key/IV        (不直接用)
      (加密 pair-verify 签名)
              │
         /fp-setup + /setup
              │
       FairPlay 密钥交换 (OmgHax)
              │
         aesKey (16 bytes)
              │
    ┌─────────┴─────────┐
    │                   │
  SHA-512 + sharedSecret  SHA-512 + sharedSecret
    │ ("AirPlayStream     │ (直接取前 16 bytes)
    │  Key" + connID)     │
    │                     │
  AES-CTR (视频)         AES-CBC + eiv (音频)
```

### 9.2 SHA-512 作为 KDF 的使用模式

整个协议大量使用 SHA-512 作为密钥派生函数:

```
模式 1: KDF(prefix, secret) → 取前 16 bytes 为 key/IV
  sha512.update(prefix_string)
  sha512.update(secret_bytes)
  result = sha512.digest()[0..16]

模式 2: KDF(key, secret) → 取前 16 bytes 为 key
  sha512.update(aesKey)        // 16 bytes
  sha512.update(sharedSecret)  // 32 bytes
  result = sha512.digest()[0..16]

模式 3: KDF(prefix + connID, key) → 取前 16 bytes
  sha512.update(prefix + streamConnectionID)
  sha512.update(eaesKey[0..16])
  result = sha512.digest()[0..16]
```

### 9.3 ModifiedMD5

`ModifiedMD5.modified_md5(base, sessionKey, md5)`:

在标准 MD5 基础上修改了:
- 初始 IV 值
- 每轮的常量
- 字节序处理

用于 `generate_session_key()` 中的 5 轮迭代。

### 9.4 SapHash

`SapHash.sap_hash(blockIn, keyOut)`:

1. 将 64 bytes 输入加载到 `buffer1[0..210]`
2. 840 轮置换 (类似 LFSR 的反馈移位)
3. 调用 `HandGarble.garble()` 执行大量混淆操作
4. 通过多层 XOR 累积输出 16 bytes 密钥

### 9.5 HandGarble

极其复杂的混淆函数, 操作 5 个 buffer (`buffer0` ~ `buffer4`), 执行数百步的字节操作, 包括:
- 查表置换 (`rol8`, `ror8`, `weird_rol8`)
- 算术运算 (加、减、乘、除、取模)
- 位运算 (AND, OR, XOR, NOT)
- 复杂的表达式组合

这是逆向工程最模糊的部分, 代码注释中充满 "I do not know why" 和 "FIXME"。

---

## 10. 网络架构

### 10.1 端口分配

```
ControlServer   : TCP, 随机端口 (通过 mDNS 广播)
  ├── RTSP 控制请求
  ├── HTTP 控制请求 (HLS 模式)
  └── 事件端口 (eventPort, 用于反向连接)

VideoServer     : TCP, 随机端口 (通过 RTSP SETUP 告知)
  └── 视频数据流

AudioServer     : UDP, 随机端口 (通过 RTSP SETUP 告知)
  └── 音频数据流

AudioControlServer: UDP, 随机端口 (通过 RTSP SETUP 告知)
  └── 音频控制 (同步、时间戳)
```

### 10.2 Netty Pipeline

**ControlServer**:
```
RtspDecoder → RtspEncoder → HttpObjectAggregator(64KB) → LoggingHandler → ControlHandler
```

**VideoServer**:
```
VideoDecoder → VideoHandler
```

**AudioServer**:
```
DatagramPacketDecoder(AudioDecoder) → AudioHandler
```

**AudioControlServer**:
```
AudioControlHandler
```

### 10.3 优雅的 Epoll/NIO 自动切换

```java
// 在 Linux 上自动使用 Epoll, 其他系统回退到 NIO
EventLoopGroup group = Epoll.isAvailable()
    ? new EpollEventLoopGroup()
    : new NioEventLoopGroup();
```

---

## 11. Rust 实现建议

### 11.1 推荐的 Crate 对照

| Java 组件 | Rust Crate | 说明 |
|---|---|---|
| Netty (TCP/UDP server) | `tokio` + `bytes` | 异步网络 |
| dd-plist | `plist` | Binary plist 解析 |
| EdDSA (Ed25519) | `ed25519-dalek` | 签名 |
| Curve25519 ECDH | `x25519-dalek` | 密钥协商 |
| AES-CTR/CBC | `aes` + `ctr` / `cbc` | 流加密 |
| SHA-512 | `sha2` | 哈希/KDF |
| JmDNS (mDNS) | `mdns-sd` 或 `zeroconf` | 服务发现 |
| GStreamer | `gstreamer-rs` | 播放 (可选) |
| Spring Boot | `axum` / `actix-web` | HTTP 服务 (HLS 代理) |
| Lombok | `derive_builder` / `getset` | 模板代码 |
| binary plist + m3u8 | `plist` + `m3u8-parser` (需自实现或移植) | 序列化 |

### 11.2 实现优先级

建议按以下顺序实现:

1. **Phase 1: 握手协议 (无加密)**
   - mDNS 注册
   - `GET /info` (返回硬编码的能力描述)
   - 验证 iOS 设备能否发现并连接

2. **Phase 2: 配对**
   - EdDSA keygen + `/pair-setup`
   - Curve25519 ECDH + `/pair-verify` (两阶段)
   - 单元测试: 用已知向量验证

3. **Phase 3: FairPlay + 流解密**
   - 移植 `OmgHaxConst` 常量表 (从 Java 源码提取)
   - 实现 `OmgHax.cycle()` 和 `generate_session_key()`
   - 实现 `FairPlayVideoDecryptor` 和 `FairPlayAudioDecryptor`
   - 从测试资源加载 known-good 加密视频包验证

4. **Phase 4: 视频/音频流**
   - TCP VideoServer + VideoDecoder
   - UDP AudioServer + AudioDecoder
   - NAL 单元重组
   - 音频重排序缓冲

5. **Phase 5: 播放器集成**
   - 将解码后的 H.264/ALAC/AAC-ELD 输出到 GStreamer pipeline
   - 或输出到文件 (用于调试)

6. **Phase 6: HLS/YouTube 支持 (可选)**
   - HTTP Upgrade (反向连接)
   - m3u8 URL 代理和重写

### 11.3 关键注意事项

1. **Endianness**: 视频包头使用 **little-endian**, NAL 单元长度使用 **big-endian**, 音频包头使用 **big-endian**。务必仔细处理。

2. **OmgHax 常量**: 10 个查找表中有几个超过 10KB (`table_s1` ~ `table_s10`)。建议直接从 Java 源码生成 Rust 文件:
   ```rust
   // 自动化脚本: 解析 Java 的 byte[] 数组 → Rust 的 [u8; N]
   pub const TABLE_S9: [u32; 1024] = [0x..., 0x..., ...];
   ```

3. **Curve25519 类型区分**: `ed25519_dalek::SigningKey` 和 `x25519_dalek::StaticSecret` 是不同类型, 需要显式转换。

4. **AES-CTR 溢出**: 视频 CTR 解密在包边界需要处理部分块 (不足 16 bytes 的残余), 参考 `FairPlayVideoDecryptor.og` 和 `nextDecryptCount` 逻辑。

5. **Session 管理**: 每个 iOS 设备连接创建独立 session, 包含独立的 `AirPlay` 实例 (独立的密钥状态机)。

6. **测试数据**: `java-airplay/server/src/test/resources/` 包含完整的协议交互二进制样本, 可用于 Rust 实现的集成测试。

---

## 附录 A: 关键常量速查

### FairPlay 消息头

```
FPLY = [0x46, 0x50, 0x4C, 0x59]
```

### mDNS Features 掩码

```
AirPlay:  features = "0x5A7FFFF7,0x1E"  (TXT 字符串)
AirTunes: ft       = "0x5A7FFFF7,0x1E"  (TXT 字符串)
```

### RTSP SETUP 流类型

```
96  = 音频流
110 = 视频流
```

### Audio Compression Type (ct)

```
1  = LPCM
2  = ALAC
4  = AAC
8  = AAC_ELD
32 = OPUS
```

### Video Payload Type

```
0 = 加密的图像数据 (IDR/P/B)
1 = SPS/PPS 参数集
```

---

## 附录 B: 参考文件索引

| 文件 | 内容 |
|---|---|
| `lib/.../AirPlay.java` | 顶层 API, 聚合配对+FairPlay+RTSP |
| `lib/.../Pairing.java` | EdDSA + Curve25519 配对 |
| `lib/.../FairPlay.java` | FairPlay 消息处理 |
| `lib/.../OmgHax.java` | 自定义解密算法 |
| `lib/.../OmgHaxConst.java` | 查找表常量 |
| `lib/.../FairPlayVideoDecryptor.java` | 视频 AES-CTR 解密 |
| `lib/.../FairPlayAudioDecryptor.java` | 音频 AES-CBC 解密 |
| `lib/.../RTSP.java` | RTSP SETUP/TEARDOWN 解析 |
| `lib/.../AirPlayBonjour.java` | mDNS 注册 |
| `server/.../ControlHandler.java` | 所有 RTSP/HTTP 端点 |
| `server/.../VideoHandler.java` | 视频 NAL 重组 |
| `server/.../AudioHandler.java` | 音频重排序缓冲 |
| `server/.../VideoDecoder.java` | 视频包解码 |
| `server/.../AudioDecoder.java` | 音频包解码 |
| `server/.../PropertyListUtil.java` | bplist 响应构建 |
| `server/.../Session.java` | 会话状态 |
| `client/.../App.java` | 客户端完整流程演示 |
