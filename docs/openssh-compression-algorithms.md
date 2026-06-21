# OpenSSH 压缩算法支持

## 支持的压缩算法

OpenSSH 支持以下两种压缩算法：

### 1. `none` — 无压缩

- 始终可用，不需要任何编译选项
- 类型常量：`COMP_NONE`（值为 `0`，定义于 `kex.h`）

### 2. `zlib@openssh.com` — Zlib 压缩（延迟启用）

- 需要编译时启用 `WITH_ZLIB` 宏（即链接 zlib 库）
- 类型常量：`COMP_DELAYED`（值为 `2`，定义于 `kex.h`）
- 压缩级别默认 **6**（范围 1–9）
- 使用 zlib 的 `deflateInit` / `inflateInit` 实现

## 关键特点：Delayed 模式

`zlib@openssh.com` 与传统 SSH 协议中的 `zlib` 不同，它采用**延迟启用**（Delayed）策略——压缩仅在**用户认证成功之后**才开始生效（参见 `packet.c` 和 `sshd-auth.c` 中的相关逻辑）。

这是为了防止认证前的压缩被利用进行类似 CRIME/BREACH 的侧信道攻击。

## 算法协商顺序

`compression_alg_list` 函数（定义于 `cipher.c`）控制协商提议顺序：

- **客户端默认**：`none,zlib@openssh.com`（优先不压缩）
- **服务端启用压缩时**：`zlib@openssh.com,none`（优先压缩）

## 配置方式

### 客户端（ssh_config）

```
Compression yes    # 启用 zlib@openssh.com（需 WITH_ZLIB）
Compression no     # 不压缩（默认）
```

客户端也可通过 `-C` 命令行选项启用压缩。

### 服务端（sshd_config）

```
Compression yes       # 等同于 delayed
Compression delayed   # 认证后启用压缩（需 WITH_ZLIB）
Compression no        # 不压缩
```

服务端默认值为 `COMP_DELAYED`（编译时启用 WITH_ZLIB）或 `COMP_NONE`。

## 源码关键位置

| 文件 | 内容 |
|------|------|
| `kex.h` | `COMP_NONE` / `COMP_DELAYED` 常量定义 |
| `cipher.c` | `compression_alg_list()` 函数，返回协商算法列表 |
| `cipher.h` | `compression_alg_list()` 函数声明 |
| `packet.c` | `ssh_packet_init_compression()`、`start_compression_out()`、`start_compression_in()` 压缩初始化与启停逻辑 |
| `sshd-auth.c` | 服务端 KEX 时根据 `options.compression` 决定提议 `none` 还是完整列表 |
| `readconf.c` | 客户端 `Compression` 配置项解析 |
| `servconf.c` | 服务端 `Compression` 配置项解析 |

## 注意事项

- 早期 OpenSSH 曾支持标准的 `zlib` 算法名（非延迟模式），但当前版本已移除，只保留 `zlib@openssh.com`。
- 如果编译时未启用 `WITH_ZLIB`，则只能使用 `none`，SSH 客户端的 `-C` 选项会被忽略并输出错误提示。
