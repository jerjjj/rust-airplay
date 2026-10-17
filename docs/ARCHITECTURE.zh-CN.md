# 架构

English: [ARCHITECTURE.md](ARCHITECTURE.md)。

## crate 划分

```
rust-airplay-cli          命令行接收端
        |
        v
rust-airplay              库
  +-- receiver            会话：端口、发现、密钥、流
  +-- server              TCP 上的控制协议
  +-- media               音频、镜像、时序、套接字
  +-- discovery           mDNS 广播
  +-- pairing、srp、crypto、fairplay、plist、wire
  +-- render              渲染器接入的边界
        ^
        |
rust-airplay-gstreamer    渲染器
```

库不依赖本仓库内任何其他 crate；后端依赖库与 GStreamer；程序依赖库，以及可选的 backend。

## 会话流程

1. 客户端通过 mDNS 找到接收端，建立 TCP 连接。
2. `/info` 返回与广播相同的文本记录。
3. `/pair-verify` 识别已知客户端；`/pair-setup` 用 PIN 与新客户端配对。
4. `SETUP` 声明流。接收端应答实际绑定的端口，并派生会话所需的密钥。
5. `RECORD` 开始播放。
6. 媒体从时序、控制、数据三个套接字到达，镜像画面从 TCP 监听到达。帧被解密、排序后交给渲染器。
7. `TEARDOWN` 或连接关闭结束会话。

音频密钥在某次 `SETUP` 中到达；镜像流的标识在之后的另一次 `SETUP` 中到达且不带密钥。两者可能来自不同连接，因此密钥是**会话状态**而非连接状态。

## 渲染

帧以带时间戳的独立缓冲区跨过 trait 边界。会阻塞接收循环的后端会拖住循环关注的一切，因此每次写入都在自己的线程上进行，由有界队列供给。队列满时丢弃最旧的一帧，从而限制滞后。

GStreamer 后端的两种媒体投递方式不同：

- 视频是 `gst-launch-1.0` 子进程：H.264 带起始码，字节流自带帧边界。
- 音频是链接并推 `appsrc`：raw AAC-ELD 帧既无同步字也无采样率，一帧一 buffer 是唯一可行的投递方式。

这也是 GStreamer 只作为后端 crate 依赖、默认工作区不链接任何 C 库的原因。

## 发现

同时注册 `_raop._tcp` 与 `_airplay._tcp`，并带上当前客户端期望的文本记录。这些值不只影响外观：例如 feature 位图的第 27 位决定使用哪套客户端配对协议，因此对当前协议是清零的。测试逐字段钉住了这条记录。

## 身份

身份是 32 字节种子。设备标识由它派生，因此一个种子就是一个接收端，重启后仍是同一个。客户端密钥在会话携带过之后才被记住。

库不保存任何东西。让接收端能被认出的，是持久化种子与客户端名单的那个程序。
