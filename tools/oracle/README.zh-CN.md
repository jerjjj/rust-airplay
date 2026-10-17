# 差分测试 oracle

一个小的 C 程序，链接上游源码并打印它们的中间值，从而让 Rust 实现可以与它所遵循的代码对比，而不是与关于那份代码的描述对比。

## 构建

需要 `upstream/UxPlay` 下提交 `57ea83411d5f7e0b38c5841987439340543f025c` 的检出，以及 MSYS2 UCRT64：

```powershell
pwsh -File tools/oracle/build.ps1
```

## 运行

```powershell
pwsh -File tools/oracle/run.ps1            # 缺失时先构建，然后比对
pwsh -File tools/oracle/run.ps1 -Rebuild   # 先重新构建
```

退出码即测试结果，可以据此在流水线中设卡。它等价于：

```powershell
$env:AIRPLAY_UPSTREAM_ORACLE = "$PWD/tools/oracle/oracle.exe"
cargo test -p rust-airplay --test upstream_oracle -- --ignored
```

## 检查什么

| 模式 | 打印内容 |
|---|---|
| `srp` | SRP-6a 的中间值 |
| `pairing` | `/pair-verify` 与 `/pair-setup` 的向量 |
| `fairplay` | FairPlay 变换 |
| `mirror` | 镜像解密器逐包从哪个位置取密钥流 |

画面不正确时先看 `mirror`：它打印计数器与密钥流位置，因此不一致能指出两条规则中哪一条错了，而不只是输出不同。

English: [README.md](README.md)。