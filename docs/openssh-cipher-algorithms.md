# OpenSSH 加密算法（Ciphers）

> 基于 OpenSSH Portable 源码分析，涵盖握手后数据传输阶段支持的所有加密算法、默认配置与工作原理。

---

## 目录

1. [全部实现的加密算法](#1-全部实现的加密算法)
2. [默认启用配置](#2-默认启用配置)
3. [算法分类详解](#3-算法分类详解)
4. [AEAD 与非 AEAD 的区别](#4-aead-与非-aead-的区别)
5. [编译条件与依赖](#5-编译条件与依赖)
6. [相关源码文件](#6-相关源码文件)

---

## 1. 全部实现的加密算法

OpenSSH 在 `cipher.c` 的 `ciphers[]` 数组中注册了所有已实现的加密算法，共计 **11 种**（含 1 种内部算法）。

### 1.1 AEAD 认证加密（Authenticated Encryption）

| # | 算法名 | 分组大小 | 密钥长度 | IV 长度 | 认证标签长度 | 模式 |
|---|--------|:-------:|:-------:|:------:|:----------:|------|
| 1 | `chacha20-poly1305@openssh.com` | 8 | 64 字节 | 0 | 16 字节 | ChaCha20 + Poly1305 |
| 2 | `aes128-gcm@openssh.com` | 16 | 16 字节 | 12 字节 | 16 字节 | AES-128-GCM |
| 3 | `aes256-gcm@openssh.com` | 16 | 32 字节 | 12 字节 | 16 字节 | AES-256-GCM |

### 1.2 CTR 计数器模式

| # | 算法名 | 分组大小 | 密钥长度 | IV 长度 | 模式 |
|---|--------|:-------:|:-------:|:------:|------|
| 4 | `aes128-ctr` | 16 | 16 字节 | 16 字节 | AES-128-CTR |
| 5 | `aes192-ctr` | 16 | 24 字节 | 16 字节 | AES-192-CTR |
| 6 | `aes256-ctr` | 16 | 32 字节 | 16 字节 | AES-256-CTR |

### 1.3 CBC 模式（需 OpenSSL，默认不启用）

| # | 算法名 | 分组大小 | 密钥长度 | IV 长度 | 模式 |
|---|--------|:-------:|:-------:|:------:|------|
| 7 | `3des-cbc` | 8 | 24 字节 | 8 字节 | Triple-DES-CBC |
| 8 | `aes128-cbc` | 16 | 16 字节 | 16 字节 | AES-128-CBC |
| 9 | `aes192-cbc` | 16 | 24 字节 | 16 字节 | AES-192-CBC |
| 10 | `aes256-cbc` | 16 | 32 字节 | 16 字节 | AES-256-CBC |

### 1.4 内部算法（不可协商）

| # | 算法名 | 分组大小 | 密钥长度 | 说明 |
|---|--------|:-------:|:-------:|------|
| 11 | `none` | 8 | 0 | 明文传输，标记为 `CFLAG_INTERNAL`，仅在密钥交换完成前的内部阶段使用，不会出现在协商列表中 |

---

## 2. 默认启用配置

定义在 `myproposal.h` 中，客户端和服务端使用相同的默认加密列表（`KEX_SERVER_ENCRYPT` = `KEX_CLIENT_ENCRYPT`）：

```
chacha20-poly1305@openssh.com,
aes128-gcm@openssh.com,
aes256-gcm@openssh.com,
aes128-ctr,
aes192-ctr,
aes256-ctr
```

> **注意**：CBC 模式算法（`3des-cbc`、`aes*-cbc`）虽然已实现，但**不包含在默认协商列表中**。用户可通过 `sshd_config` 或 `ssh_config` 的 `Ciphers` 配置项手动启用。

---

## 3. 算法分类详解

### 3.1 ChaCha20-Poly1305

- **算法标识**：`chacha20-poly1305@openssh.com`
- **密钥结构**：64 字节 = 两个 256-bit 密钥（主密钥 + 头部密钥）
- **IV 机制**：无需外部 IV，使用数据包序列号（seqnr）作为 nonce
- **认证标签**：16 字节 Poly1305 MAC
- **实现位置**：`cipher-chachapoly.c` / `cipher-chachapoly-libcrypto.c`
- **特点**：
  - 内置实现，不依赖 OpenSSL
  - 使用双 ChaCha20 实例：一个加密载荷（main），一个加密包头长度（header）
  - Poly1305 密钥由每次数据包加密时动态生成
  - 优先级最高，位于默认列表首位

### 3.2 AES-GCM

- **算法标识**：`aes128-gcm@openssh.com` / `aes256-gcm@openssh.com`
- **IV 长度**：12 字节（固定），采用隐式 IV 自增机制（`EVP_CTRL_GCM_IV_GEN`）
- **认证标签**：16 字节
- **依赖**：OpenSSL EVP 接口（`EVP_aes_128_gcm` / `EVP_aes_256_gcm`）
- **特点**：
  - AES-GCM 模式自带认证能力，无需额外 MAC
  - 包头（packet length）作为 AAD（附加认证数据）传输，不加密但参与认证
  - 仅支持 128-bit 和 256-bit 两种密钥长度

### 3.3 AES-CTR

- **算法标识**：`aes128-ctr` / `aes192-ctr` / `aes256-ctr`
- **IV 长度**：16 字节（等于分组大小）
- **认证**：无内置认证，需配合独立 MAC 算法使用
- **依赖**：OpenSSL（`EVP_aes_*_ctr`）；无 OpenSSL 时使用内置 `aesctr` 实现
- **特点**：
  - 计数器模式，可并行加解密
  - 支持 128/192/256 三种密钥长度
  - 需要额外的 MAC 保障完整性

### 3.4 AES-CBC / 3DES-CBC

- **算法标识**：`aes128-cbc` / `aes192-cbc` / `aes256-cbc` / `3des-cbc`
- **标记**：`CFLAG_CBC`
- **认证**：无内置认证，需配合独立 MAC 算法使用
- **依赖**：OpenSSL（`EVP_aes_*_cbc` / `EVP_des_ede3_cbc`）
- **特点**：
  - CBC 模式无法并行解密
  - `3des-cbc` 有效安全强度仅 112-bit（`cipher_seclen()` 返回 14 字节）
  - 默认不启用，仅在用户显式配置时可用
  - 不满足 `auth_only` 过滤条件（`auth_len = 0`）

---

## 4. AEAD 与非 AEAD 的区别

| 特性 | AEAD 算法 | 非 AEAD 算法 |
|------|----------|-------------|
| 完整性校验 | 内置认证标签 | 需要独立 MAC 算法 |
| MAC 启用 | 不启用（`mac = NULL`） | 必须启用 |
| 包头处理 | AAD 模式（加密/认证） | 明文或参与认证 |
| 代表算法 | chacha20-poly1305, aes-gcm | aes-ctr, aes-cbc, 3des-cbc |

在 `packet.c` 的 `ssh_packet_read_poll2()` 中：
```c
if ((authlen = cipher_authlen(enc->cipher)) != 0)
    mac = NULL;  // AEAD 算法禁用独立 MAC
```

---

## 5. 编译条件与依赖

| 算法 | 编译条件 | 依赖库 |
|------|---------|--------|
| `chacha20-poly1305@openssh.com` | 无条件可用 | 内置实现 |
| `aes*-gcm@openssh.com` | `WITH_OPENSSL` | OpenSSL EVP |
| `aes*-ctr` | 有 OpenSSL 时用 EVP；无 OpenSSL 时用内置 `aesctr` | OpenSSL 或内置 |
| `aes*-cbc` | `WITH_OPENSSL` | OpenSSL EVP |
| `3des-cbc` | `WITH_OPENSSL` + `!OPENSSL_NO_DES` | OpenSSL EVP |

---

## 6. 相关源码文件

| 文件 | 说明 |
|------|------|
| `cipher.c` | 加密算法注册表（`ciphers[]`）、初始化、加解密核心逻辑 |
| `cipher.h` | 加密算法接口声明 |
| `cipher-chachapoly.c` | ChaCha20-Poly1305 内置实现 |
| `cipher-chachapoly-libcrypto.c` | ChaCha20-Poly1305 OpenSSL 实现 |
| `cipher-aesctr.c` | AES-CTR 内置实现（无 OpenSSL 时使用） |
| `cipher-aes.c` | AES 底层辅助函数 |
| `myproposal.h` | 默认协商算法列表定义 |
| `packet.c` | 数据包加解密调用入口 |
| `kex.c` | KEX 协商中加密算法选择（`choose_enc()`） |
