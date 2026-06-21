# OpenSSH 密钥格式解析指南

> 基于 OpenSSH Portable 源码分析，涵盖公钥/私钥的完整格式、解析方法与认证流程。

---

## 目录

1. [支持的密钥类型](#1-支持的密钥类型)
2. [公钥格式与解析](#2-公钥格式与解析)
3. [私钥文件格式（三层结构）](#3-私钥文件格式三层结构)
4. [KDF 与 Cipher 的职责分工](#4-kdf-与-cipher-的职责分工)
5. [私钥明文区解析（解密后）](#5-私钥明文区解析解密后)
6. [mpint 编码规则](#6-mpint-编码规则)
7. [SSH 公钥认证流程](#7-ssh-公钥认证流程)
8. [各 key_type 到认证三要素的详细映射](#8-各-key_type-到认证三要素的详细映射)

---

## 1. 支持的密钥类型

OpenSSH 在 `sshkey.c` 的 `keyimpls[]` 数组中注册了 **20 种条目**，但分为两类：

| 类别 | 数量 | `.sigonly` | 含义 |
|------|:---:|:---:|------|
| **密钥类型** | 16 种 | `0` | 可以出现在私钥文件中作为 `key_type` |
| **签名算法标识** | 4 种 | `1` | 仅用于协议层签名协商，不会写入私钥文件 |

### 1.1 Ed25519（2 种）

| # | key_type | 类型 | 依赖 OpenSSL |
|---|---------|------|:---:|
| 1 | `ssh-ed25519` | 普通 | 否 |
| 2 | `ssh-ed25519-cert-v01@openssh.com` | 证书 | 否 |

### 1.2 Ed25519-SK（2 种）

| # | key_type | 类型 | 依赖 OpenSSL |
|---|---------|------|:---:|
| 3 | `sk-ssh-ed25519@openssh.com` | 普通 | 否 |
| 4 | `sk-ssh-ed25519-cert-v01@openssh.com` | 证书 | 否 |

### 1.3 ECDSA（6 种）

| # | key_type | 类型 | 依赖 OpenSSL |
|---|---------|------|:---:|
| 5 | `ecdsa-sha2-nistp256` | 普通 | 是（EC 对象） |
| 6 | `ecdsa-sha2-nistp256-cert-v01@openssh.com` | 证书 | 是 |
| 7 | `ecdsa-sha2-nistp384` | 普通 | 是 |
| 8 | `ecdsa-sha2-nistp384-cert-v01@openssh.com` | 证书 | 是 |
| 9 | `ecdsa-sha2-nistp521` | 普通 | 是 |
| 10 | `ecdsa-sha2-nistp521-cert-v01@openssh.com` | 证书 | 是 |

### 1.4 ECDSA-SK（4 种）

| # | key_type | 类型 | 依赖 OpenSSL |
|---|---------|------|:---:|
| 11 | `sk-ecdsa-sha2-nistp256@openssh.com` | 普通 | 是 |
| 12 | `sk-ecdsa-sha2-nistp256-cert-v01@openssh.com` | 证书 | 是 |
| 13 | `webauthn-sk-ecdsa-sha2-nistp256@openssh.com` | 普通（WebAuthn） | 是 |
| 14 | `webauthn-sk-ecdsa-sha2-nistp256-cert-v01@openssh.com` | 证书（WebAuthn） | 是 |

### 1.5 RSA 密钥类型（2 种，可出现在私钥文件中）

| # | key_type | 类型 | 依赖 OpenSSL |
|---|---------|------|:---:|
| 15 | `ssh-rsa` | 普通 | 是（RSA 大数） |
| 16 | `ssh-rsa-cert-v01@openssh.com` | 证书 | 是 |

**私钥文件中的 RSA key_type 永远是 `ssh-rsa`（或证书变体），不会写入 `rsa-sha2-256` 或 `rsa-sha2-512`。**

### 1.6 RSA 签名算法标识（4 种，sigonly=1，不出现在文件中）

| # | 名称 | 内部 type | 哈希算法 | `.sigonly` |
|---|------|----------|---------|:---:|
| 17 | `rsa-sha2-256` | KEY_RSA | SHA-256 | 1 |
| 18 | `rsa-sha2-512` | KEY_RSA | SHA-512 | 1 |
| 19 | `rsa-sha2-256-cert-v01@openssh.com` | KEY_RSA_CERT | SHA-256 | 1 |
| 20 | `rsa-sha2-512-cert-v01@openssh.com` | KEY_RSA_CERT | SHA-512 | 1 |

这 4 种与 `ssh-rsa` 使用完全相同的私钥数据，区别仅在签名时使用的哈希算法不同。它们仅用于协议层的签名算法协商（`PubkeyAcceptedAlgorithms`、`server_sig_algs`），不会作为 key_type 写入私钥文件。

源码依据（`ssh-rsa.c:620-642`）：
```c
// ssh-rsa — sigonly = 0 → 可以写入私钥文件
const struct sshkey_impl sshkey_rsa_impl = {
    .name = "ssh-rsa",
    .sigonly = 0,
};

// rsa-sha2-256 — sigonly = 1 → 仅用于协议协商
const struct sshkey_impl sshkey_rsa_sha256_impl = {
    .name = "rsa-sha2-256",
    .sigonly = 1,
};
```

### 1.7 私钥磁盘格式

`enum sshkey_private_format`：
- `SSHKEY_PRIVATE_OPENSSH` — OpenSSH 自有格式（`openssh-key-v1`）
- `SSHKEY_PRIVATE_PEM` — OpenSSL PEM 格式
- `SSHKEY_PRIVATE_PKCS8` — PKCS#8 格式

---

## 2. 公钥格式与解析

### 2.1 文本格式

`.pub` 文件每一行格式：

```
<type_string> <base64_data> [comment]
```

- **type_string**：如 `ssh-ed25519`、`ssh-rsa`
- **base64_data**：公钥的 SSH wire format blob 的 Base64 编码
- **comment**：可选，任意文本

### 2.2 分隔规则

- 三个字段之间可以用**任意数量的空格或 Tab**分隔（混合也行）
- **从文件加载时**：允许前导空白（`sshkey_try_load_public()` 在 `authfile.c:226` 会跳过）
- **直接调用 `sshkey_read()`**：不允许前导空白（会解析失败）

### 2.3 base64_data 的二进制结构（SSH wire format）

每个字段采用 **string 编码**：`uint32_be 长度 + 数据`

```
[uint32_be len1] [type_string bytes]
[uint32_be len2] [算法特定公钥数据...]
```

#### Ed25519 公钥

```
string  "ssh-ed25519"     ← 11 字节
string  public_key        ← 固定 32 字节
```

#### RSA 公钥

```
string  "ssh-rsa"
mpint   e                 ← 公钥指数（注意：e 在 n 前面）
mpint   n                 ← 模数
```

#### ECDSA 公钥

```
string  "ecdsa-sha2-nistp256"
string  "nistp256"        ← curve name
string  ec_point          ← 0x04 || X || Y（非压缩格式）
```

### 2.4 解析归属

| 组件 | 实现方 |
|------|--------|
| 文本行分割、Base64 解码 | OpenSSH 自研 |
| wire format 字段解析 | OpenSSH 自研 |
| Ed25519 密钥对象 | OpenSSH 内置实现（不依赖 OpenSSL） |
| RSA/ECDSA 底层对象构建 | OpenSSL API |
| PEM/PKCS8 格式解析 | 完全委托 OpenSSL |

---

## 3. 私钥文件格式（三层结构）

### 第一层：文本层（PEM 封装）

```
-----BEGIN OPENSSH PRIVATE KEY-----
<base64 编码数据，支持 70 字符换行>
-----END OPENSSH PRIVATE KEY-----
```

### 第二层：信封层（Base64 解码后的二进制）

```
┌─────────────────────────────────────────────────────┐
│  明文区域（不加密）                                    │
│                                                     │
│  AUTH_MAGIC   "openssh-key-v1\0"  (15 字节，含 \0)   │
│  ciphername   string (如 "aes256-ctr" 或 "none")    │
│  kdfname      string (如 "bcrypt" 或 "none")        │
│  kdf_options  string (KDF 参数，string 编码嵌套)     │
│  num_keys     uint32 (固定为 1)                     │
│  pubkey_blob  string (完整公钥 wire format blob)    │
│  encrypted_len uint32 (加密区长度)                   │
├─────────────────────────────────────────────────────┤
│  加密区域                                             │
│  encrypted_data  byte[] (长度为 encrypted_len)       │
├─────────────────────────────────────────────────────┤
│  AEAD 认证标签（仅 GCM/ChaCha20-Poly1305）           │
│  auth_tag  16 字节                                   │
└─────────────────────────────────────────────────────┘
```

> **注意**：公钥（`pubkey_blob`）是明文存储的，不需要密码就能读取。

### 第三层：解密后的明文区

当 `ciphername != "none"` 时，需要用密码解密 `encrypted_data`，得到：

```
┌─────────────────────────────────────┐
│  uint32  check1                     │
│  uint32  check2  (必须 == check1)    │
│  string  key_type_name              │
│  ...     算法特定字段（见第 5 节）     │
│  string  comment                    │
│  byte[]  padding: 0x01,0x02,0x03...│
└─────────────────────────────────────┘
```

**验证密码正确性**：解密后检查 `check1 == check2`，不等则说明密码错误。

### 支持的加密算法

| ciphername | block_size | key_len | iv_len | auth_len |
|-----------|:---:|:---:|:---:|:---:|
| `none` | 8 | 0 | 0 | 0 |
| `aes256-ctr` | 16 | 32 | 16 | 0 |
| `aes192-ctr` | 16 | 24 | 16 | 0 |
| `aes128-ctr` | 16 | 16 | 16 | 0 |
| `aes256-cbc` | 16 | 32 | 16 | 0 |
| `aes192-cbc` | 16 | 24 | 16 | 0 |
| `aes128-cbc` | 16 | 16 | 16 | 0 |
| `aes256-gcm@openssh.com` | 16 | 32 | 12 | 16 |
| `aes128-gcm@openssh.com` | 16 | 16 | 12 | 16 |
| `chacha20-poly1305@openssh.com` | 8 | 64 | 0 | 16 |

---

## 4. KDF 与 Cipher 的职责分工

### KDF（Key Derivation Function）— 不加密任何数据

bcrypt_pbkdf 的作用：把用户密码变成加密密钥 + IV。

```
passphrase + salt + rounds  ──bcrypt_pbkdf──→  [加密密钥(key_len) | IV(iv_len)]
```

源码位置：`sshkey.c:2858-2859`

```c
bcrypt_pbkdf(passphrase, strlen(passphrase),
    salt, SALT_LEN, key, keylen + ivlen, rounds)
```

KDF 参数（`kdf_options` 字段内部结构）：
- `kdfname = "none"`：空（无密码）
- `kdfname = "bcrypt"`：`string salt(16字节) + uint32 rounds`

### Cipher — 只加密私钥明文区

Cipher 使用 KDF 派生的密钥，加密/解密第三层（`encrypted_data`）。

源码位置：`sshkey.c:2912-2914`

```c
cipher_crypt(ciphercontext, 0, cp,
    sshbuf_ptr(encrypted), sshbuf_len(encrypted), 0, authlen)
```

**一句话**：KDF 生成密钥，Cipher 用这个密钥加密私钥数据。公钥始终明文存储。

---

## 5. 私钥明文区解析（解密后）

### 解析入口

解密后的 buffer 由 `sshkey_private_deserialize()` 解析（`sshkey.c:2606`）：

1. 读取 `check1`、`check2`，验证 `check1 == check2`
2. 读取 `key_type_name`（string）
3. 如果是证书密钥，先读取 `cert_blob`（string）
4. 按算法类型分发到各 `deserialize_private` 函数
5. 读取 `comment`（string）
6. 验证 padding：字节值为 `1, 2, 3, ..., N`（mod 256）

### 5.1 Ed25519（`ssh-ed25519`）

源码：`ssh-ed25519.c:120`

```
string  "ssh-ed25519"
string  public_key     ← 固定 32 字节
string  secret_key     ← 固定 64 字节 (前32=seed, 后32=public_key拷贝)
```

逐字节示例：
```
00 00 00 0B  73 73 68 2D 65 64 32 35 35 31 39  ← "ssh-ed25519"
00 00 00 20  [32字节公钥]
00 00 00 40  [64字节私钥: seed(32) || pubkey(32)]
```

> 后 32 字节与前 32 字节公钥内容相同，这是 Ed25519 标准密钥格式。

### 5.2 RSA（`ssh-rsa`）

源码：`ssh-rsa.c:232`

```
string  "ssh-rsa"
mpint   n        ← 模数（注意：n 在 e 前面，与公钥相反！）
mpint   e        ← 公钥指数
mpint   d        ← 私钥指数
mpint   iqmp     ← q^(-1) mod p
mpint   p        ← 素数 prime1
mpint   q        ← 素数 prime2
```

> 证书密钥（`ssh-rsa-cert-v01@openssh.com`）：先读 `cert_blob`，然后只读 `d, iqmp, p, q`（n, e 在证书中）。

### 5.3 ECDSA（`ecdsa-sha2-nistp256`）

源码：`ssh-ecdsa.c:282`

```
string  "ecdsa-sha2-nistp256"
string  curve_name    ← "nistp256" / "nistp384" / "nistp521"
string  ec_point      ← 0x04 || X || Y（非压缩 EC 点）
mpint   d             ← 私钥标量
```

EC 点编码：仅支持 `POINT_CONVERSION_UNCOMPRESSED`（首字节 `0x04`）。

### 5.4 Ed25519-SK（`sk-ssh-ed25519@openssh.com`）

源码：`ssh-ed25519-sk.c:110`

SK 密钥的私钥不在文件中（在硬件安全设备中），存储的是密钥句柄：

```
string  "sk-ssh-ed25519@openssh.com"
string  public_key     ← 32 字节
string  application    ← FIDO RP ID，通常 "ssh:"
uint8   flags          ← SK 标志位
string  key_handle     ← FIDO 密钥句柄
string  reserved       ← 保留字段（通常为空）
```

### 5.5 ECDSA-SK（`sk-ecdsa-sha2-nistp256@openssh.com`）

源码：`ssh-ecdsa-sk.c:141`

```
string  "sk-ecdsa-sha2-nistp256@openssh.com"
string  curve_name     ← "nistp256"
string  ec_point       ← 0x04 || X || Y
string  application
uint8   flags
string  key_handle
string  reserved
```

### 5.6 证书密钥

证书密钥（`*-cert-v01@openssh.com`）的结构：

```
string  cert_type_name
string  cert_blob      ← 完整证书二进制（内含公钥）
...     算法私钥字段    ← 不含公钥（从证书中获取）
```

#### cert_blob 的双重存储

证书密钥的私钥文件中，证书数据**出现了两次**：

```
┌─ 信封头（明文，不加密）──────────────────┐
│  pubkey_blob = key->cert->certblob       │
│  （第 1 份，认证时发给服务器）             │
└──────────────────────────────────────────┘

┌─ 加密区（解密后）────────────────────┐
│  key_type_name                        │
│  cert_blob = key->cert->certblob  ← 第 2 份  │
│  私钥参数 (d, p, q, iqmp 等)         │
└──────────────────────────────────────────┘
```

**源码验证**：

信封头的 pubkey_blob 生成（`sshkey.c:874-882`，`to_blob_buf()`）：
```c
if (sshkey_type_is_cert(type)) {
    /* Use the existing blob */
    sshbuf_putb(b, key->cert->certblob);  // 直接写入 certblob
    return 0;
}
```

加密区的 cert_blob 写入（`sshkey.c:2558`，`sshkey_private_serialize_opt()`）：
```c
if (sshkey_is_cert(key)) {
    sshbuf_put_stringb(b, key->cert->certblob);  // 写入同一份 certblob
}
```

两者都来自同一个 `key->cert->certblob`，**内容完全一致**。

#### cert_blob 的作用

| 作用 | 阶段 | 说明 |
|------|------|------|
| 提取公钥 | 解析时 | 从 cert_blob 中解析出 n/e（RSA）或 32 字节公钥（Ed25519） |
| 交叉校验 | 解析时 | 确保证书中的公钥与私钥区的私钥参数匹配 |
| 发给服务器 | 认证时 | 客户端将证书原样发送，服务器验证 CA 签名 |

#### 解析时的处理建议

对于证书密钥，加密区内的 cert_blob 与信封头的 pubkey_blob 同源。如果你的目标只是 SSH 登录：

- **信封头的 pubkey_blob**：必须保留，认证时发给服务器
- **加密区的 cert_blob**：可以跳过（只移动偏移量，不解析内容），因为信封头已有一份

```python
# 证书密钥解析时跳过 cert_blob
if is_cert:
    cert_blob_len, p = read_u32(plain, p)
    p += cert_blob_len   # 跳过，不解析内容
    # 实际使用时从信封头的 pubkey_blob 获取证书
```

---

## 6. mpint 编码规则

### 二进制结构

```
[uint32_be 字节长度] [大端字节数据...]
```

与 string 编码相同，只是数据按 mpint 规则解释。

### 编码规则（写入时）

源码：`sshbuf-getput-basic.c:569` (`sshbuf_put_bignum2_bytes`)

```c
// 1. 去掉前导零字节
for (; len > 0 && *s == 0; len--, s++) ;

// 2. 如果首字节 MSB=1（≥0x80），前面补一个 0x00
prepend = len > 0 && (s[0] & 0x80) != 0;
```

**MSB（Most Significant Bit）**= 字节的最高位（bit 7）。

```
bit:  7  6  5  4  3  2  1  0
      ↑                    ↑
      MSB                 LSB

0x7F = 0111 1111   → MSB = 0，不需要补零
0x80 = 1000 0000   → MSB = 1，需要补零
```

SSH 协议的 mpint 是有符号格式（MSB=1 表示负数），但密码学参数全是正数，所以正数首字节 MSB=1 时必须补 `0x00` 以表明"这是正数"。

### 解码规则（解析时）

源码：`sshbuf-getput-basic.c:598` (`sshbuf_get_bignum2_bytes_direct`)

```c
// 1. 读 string: len = uint32_be, data = 后续 len 字节
// 2. 拒绝负数: data[0] & 0x80 必须为 0
if ((len != 0 && (*d & 0x80) != 0))
    return SSH_ERR_BIGNUM_IS_NEGATIVE;

// 3. 去掉前导零，得到有效数据
while (len > 0 && *d == 0x00) { d++; len--; }
```

### 解析伪代码

```python
def read_mpint(data, offset):
    """从 SSH wire format 中读取一个 mpint"""
    length = int.from_bytes(data[offset:offset+4], 'big')
    offset += 4
    raw = data[offset:offset+length]
    offset += length

    # 校验：MSB 不能为 1
    if length > 0 and raw[0] & 0x80:
        raise ValueError("非法：负数 mpint")

    # 去前导零，转整数
    trimmed = raw.lstrip(b'\x00')
    value = int.from_bytes(trimmed, 'big') if trimmed else 0

    return value, offset
```

### 编码示例

| 数值 | 大端字节 | 编码结果 (hex) |
|------|---------|---------------|
| 0 | (空) | `00 00 00 00` |
| 1 | `01` | `00 00 00 01 01` |
| 127 (0x7F) | `7F` | `00 00 00 01 7F` |
| 128 (0x80) | `80` | `00 00 00 02 00 80` ← 补零 |
| 65537 | `01 00 01` | `00 00 00 03 01 00 01` |

### 常见 RSA 参数的 mpint 长度

| 参数 | RSA-2048 时的典型长度 | 说明 |
|------|:---:|------|
| n | 257 (0x101) | 256 字节 + 1 前导零 |
| e | 3 | 65537 = 0x010001 |
| d | 256 (0x100) | 通常不需要补零 |
| p, q | 129 (0x81) | 128 字节 + 1 前导零 |
| iqmp | 129 (0x81) | 128 字节 + 1 前导零 |

---

## 7. SSH 公钥认证流程

### 挑战-签名机制

```
1. 客户端 → 服务器：发送签名算法名 + 公钥 blob
2. 服务器 → 客户端：发送挑战数据（challenge）
3. 客户端：用私钥对挑战数据签名
4. 客户端 → 服务器：发送签名
5. 服务器：用公钥验证签名 → 认证成功
```

### 认证三要素

| # | 要素 | 说明 |
|---|------|------|
| 1 | 签名算法名（`alg`） | 告诉服务器用哪种算法验证签名 |
| 2 | 公钥 blob | 发给服务器，用于验证签名 |
| 3 | 私钥 | 本地签名，不发送 |

### .pub 文件是否必需？

**不需要。** 私钥文件的信封头已内嵌公钥 blob（`pubkey_blob`），客户端认证时直接使用即可。

---

## 8. 各 key_type 到认证三要素的详细映射

### 8.1 Ed25519（`ssh-ed25519`）

**解析出的字段：**

| 位置 | 字段 | 内容 |
|------|------|------|
| 信封头 | `pubkey_blob` | `string "ssh-ed25519"` + `string pubkey_32bytes` |
| 加密区 | `key_type` | `"ssh-ed25519"` |
| 加密区 | `pubkey` | 32 字节公钥 |
| 加密区 | `secret_key` | 64 字节私钥（seed ‖ pubkey） |

**映射：**

| SSH 要素 | 值 | 来源 |
|---------|---|------|
| 签名算法名 | `"ssh-ed25519"` | 固定，从 `key_type` 直接取 |
| 公钥 blob | `string "ssh-ed25519"` + `string pubkey_32bytes` | 直接用信封头 `pubkey_blob` |
| 签名密钥 | 64 字节 `secret_key` | 加密区的 64 字节原始数据，直接传给 `crypto_sign_ed25519()` |

签名过程（`ssh-ed25519.c:169`）：
```c
crypto_sign_ed25519(sig, &smlen, data, datalen, key->ed25519_sk)
//                                               ^^^^^^^^^^^^^^
//                                               直接用 64 字节原始数据
```

### 8.2 Ed25519 证书（`ssh-ed25519-cert-v01@openssh.com`）

**解析出的字段：**

| 位置 | 字段 | 内容 |
|------|------|------|
| 信封头 | `pubkey_blob` | 完整 certblob（含 type + nonce + pubkey + 证书元数据 + CA 签名） |
| 加密区 | `key_type` | `"ssh-ed25519-cert-v01@openssh.com"` |
| 加密区 | `cert_blob` | 同上（冗余，跳过） |
| 加密区 | `secret_key` | 64 字节私钥 |

**映射：**

| SSH 要素 | 值 | 来源 |
|---------|---|------|
| 签名算法名 | `"ssh-ed25519"` | 注意：**不是**证书名，是基础算法名 |
| 公钥 blob | 完整 certblob | 直接用信封头 `pubkey_blob` |
| 签名密钥 | 64 字节 `secret_key` | 与非证书完全相同 |

### 8.3 RSA（私钥文件 key_type 固定为 `ssh-rsa`）

**解析出的字段：**

| 位置 | 字段 | 内容 |
|------|------|------|
| 信封头 | `pubkey_blob` | `string "ssh-rsa"` + `mpint e` + `mpint n` |
| 加密区 | `key_type` | `"ssh-rsa"` |
| 加密区 | `n` | mpint（模数） |
| 加密区 | `e` | mpint（公钥指数） |
| 加密区 | `d` | mpint（私钥指数） |
| 加密区 | `iqmp` | mpint（q⁻¹ mod p） |
| 加密区 | `p` | mpint（素数 prime1） |
| 加密区 | `q` | mpint（素数 prime2） |

**映射：**

| SSH 要素 | 值 | 来源 |
|---------|---|------|
| 签名算法名 | `"rsa-sha2-512"` 或 `"rsa-sha2-256"` 或 `"ssh-rsa"` | **必须与服务器协商**，优先 SHA-512 > SHA-256 > SHA-1 |
| 公钥 blob | `string "ssh-rsa"` + `mpint e` + `mpint n` | 直接用信封头 `pubkey_blob`（注意：公钥中 **e 在前，n 在后**） |
| 签名密钥 | OpenSSL `EVP_PKEY*` | 用 `n, e, d, p, q, iqmp` 构建 RSA 对象 |

签名算法名与哈希算法的对应：
- `"ssh-rsa"` → SHA-1
- `"rsa-sha2-256"` → SHA-256
- `"rsa-sha2-512"` → SHA-512

#### RSA 签名密钥的完整构建过程

源码：`ssh-rsa.c:231-308` (`ssh_rsa_deserialize_private`)

**第 1 步：创建空 RSA 对象并设置公钥**

```c
RSA *rsa = RSA_new();

// 从私钥区读取 n 和 e（注意：私钥区顺序是 n 在前，e 在后，与公钥 blob 相反）
BIGNUM *rsa_n, *rsa_e;
sshbuf_get_bignum2(b, &rsa_n);  // mpint → BIGNUM
sshbuf_get_bignum2(b, &rsa_e);  // mpint → BIGNUM

// 设置公钥部分：模数 n + 公钥指数 e
RSA_set0_key(rsa, rsa_n, rsa_e, NULL);
//                      ^^^^^  ^^^^^  ^^^^
//                      n      e      d=NULL（稍后设置）
```

此时 RSA 对象只有公钥，可以做验证但不能做签名。

**第 2 步：读取私钥参数**

```c
BIGNUM *rsa_d, *rsa_iqmp, *rsa_p, *rsa_q;
sshbuf_get_bignum2(b, &rsa_d);     // 私钥指数
sshbuf_get_bignum2(b, &rsa_iqmp);  // q⁻¹ mod p
sshbuf_get_bignum2(b, &rsa_p);     // 素数 prime1
sshbuf_get_bignum2(b, &rsa_q);     // 素数 prime2
```

文件中的顺序固定为：`n, e, d, iqmp, p, q`。

**第 3 步：计算 CRT 派生参数**

文件中没有存 `dmp1` 和 `dmq1`，需要自己算（`ssh-rsa.c:362-398`）：

```c
// dmq1 = d mod (q - 1)
BIGNUM *aux = BN_new();
BN_sub(aux, rsa_q, BN_value_one());  // aux = q - 1
BN_mod(rsa_dmq1, rsa_d, aux, ctx);   // dmq1 = d mod (q - 1)

// dmp1 = d mod (p - 1)
BN_sub(aux, rsa_p, BN_value_one());  // aux = p - 1
BN_mod(rsa_dmp1, rsa_d, aux, ctx);   // dmp1 = d mod (p - 1)
```

Rust 实现：
```rust
let dmp1 = &d % &(&p - 1u32);
let dmq1 = &d % &(&q - 1u32);
```

这两个参数用于 CRT 加速签名（后面解释）。

**第 4 步：设置私钥指数**

```c
RSA_set0_key(rsa, NULL, NULL, rsa_d);
//                   ^^^^  ^^^^  ^^^^^
//                   n=NULL  e=NULL  d（这次设置私钥指数）
```

传 NULL 表示不修改已设置的值，只补充设置 d。

**第 5 步：设置素因子**

```c
RSA_set0_factors(rsa, rsa_p, rsa_q);
//                       ^^^^^  ^^^^^
//                       p      q
```

**第 6 步：设置 CRT 参数**

```c
RSA_set0_crt_params(rsa, rsa_dmp1, rsa_dmq1, rsa_iqmp);
//                       ^^^^^^^^^  ^^^^^^^^^  ^^^^^^^^
//                       dmp1       dmq1       iqmp
```

设置这三个参数后，OpenSSL 签名时会自动用 CRT 快速路径（快约 4 倍）。

**第 7 步：启用 blinding 保护**

```c
RSA_blinding_on(rsa, NULL);
```

防止侧信道攻击（计时攻击），每次签名时随机化输入。

**第 8 步：封装为 EVP_PKEY**

```c
EVP_PKEY *pkey = EVP_PKEY_new();
EVP_PKEY_set1_RSA(pkey, rsa);
```

最终得到的 `pkey` 就是可用于签名的完整 RSA 密钥对象。

#### 构建完成后的 RSA 对象结构

```
EVP_PKEY (pkey)
  └─ RSA
       ├─ n     ← 模数（公钥）
       ├─ e     ← 公钥指数
       ├─ d     ← 私钥指数
       ├─ p     ← 素数1
       ├─ q     ← 素数2
       ├─ dmp1  ← d mod (p-1)  ← 计算得出
       ├─ dmq1  ← d mod (q-1)  ← 计算得出
       └─ iqmp  ← q⁻¹ mod p   ← 从文件读出
```

#### 签名时的执行流程

源码：`ssh-rsa.c:402-445` (`ssh_rsa_sign`) + `sshkey.c:487-517` (`sshkey_pkey_digest_sign`)

```
输入：挑战数据 data + 签名算法名 alg（协商得出）

1. 根据 alg 确定哈希算法
   "rsa-sha2-512" → SHA-512
   "rsa-sha2-256" → SHA-256
   "ssh-rsa"      → SHA-1

2. 调用 OpenSSL 签名
   EVP_DigestSignInit(ctx, NULL, evpmd, NULL, pkey);
   EVP_DigestSign(ctx, sig, &slen, data, datalen);
   // 内部自动使用 CRT 参数（如果已设置）进行快速签名

3. 编码签名为 SSH 格式
   ssh_rsa_encode_store_sig(hash_alg, sig, slen, sigp, lenp)
   // 输出：string alg_name + string signature_bytes
```

#### 普通 RSA vs 证书 RSA 的构建差异

| 步骤 | 普通 RSA | 证书 RSA |
|------|---------|--------|
| 获取 n, e | 从私钥区读取 | 从 certblob 中解析 |
| 获取 d, iqmp, p, q | 从私钥区读取 | 从私钥区读取 |
| RSA 对象创建 | `RSA_new()` 全新创建 | `EVP_PKEY_get1_RSA(key->pkey)` 从已有证书密钥提取 |
| 设置公钥 | `RSA_set0_key(rsa, n, e, NULL)` | 已在证书解析时设置 |
| 后续步骤（设置 d、p、q、CRT） | 相同 | 相同 |

源码依据（`ssh-rsa.c:241-246`）：
```c
if (sshkey_is_cert(key)) {
    // 证书场景：从已有 pkey 中提取 RSA 对象（n, e 已在证书解析时设置）
    rsa = EVP_PKEY_get1_RSA(key->pkey);
} else {
    // 普通场景：全新创建 RSA 对象，从私钥区读 n, e
    rsa = RSA_new();
    sshbuf_get_bignum2(b, &rsa_n);
    sshbuf_get_bignum2(b, &rsa_e);
    RSA_set0_key(rsa, rsa_n, rsa_e, NULL);
}
// 后续步骤两种场景完全相同：读取 d, iqmp, p, q → 设置 → 计算 CRT
```

### 8.4 RSA 证书（`ssh-rsa-cert-v01@openssh.com` 等）

**解析出的字段：**

| 位置 | 字段 | 内容 |
|------|------|------|
| 信封头 | `pubkey_blob` | 完整 certblob |
| 加密区 | `key_type` | `"ssh-rsa-cert-v01@openssh.com"` |
| 加密区 | `cert_blob` | 同上（冗余，跳过） |
| 加密区 | `d, iqmp, p, q` | mpint × 4（**无 n, e**，在证书中） |

**映射：**

| SSH 要素 | 值 | 来源 |
|---------|---|------|
| 签名算法名 | 与非证书 RSA 相同，**需协商** | 证书密钥签名算法仍用基础名协商 |
| 公钥 blob | 完整 certblob | 直接用信封头 `pubkey_blob` |
| 签名密钥 | OpenSSL `EVP_PKEY*` | 需从 `pubkey_blob`（certblob）中提取 n, e，再与 d, p, q, iqmp 一起构建 RSA 对象 |

> **关键区别**：证书密钥的 `n, e` 不在私钥区，必须从 certblob 中解析证书结构才能获取。

### 8.5 ECDSA（`ecdsa-sha2-nistp256` / `nistp384` / `nistp521`）

**解析出的字段：**

| 位置 | 字段 | 内容 |
|------|------|------|
| 信封头 | `pubkey_blob` | `string "ecdsa-sha2-nistp256"` + `string "nistp256"` + `string ec_point` |
| 加密区 | `key_type` | `"ecdsa-sha2-nistp256"` |
| 加密区 | `curve` | `"nistp256"`（或 nistp384/521） |
| 加密区 | `ec_point` | `0x04 ‖ X ‖ Y`（非压缩 EC 点） |
| 加密区 | `d` | mpint（私钥标量） |

**映射：**

| SSH 要素 | 值 | 来源 |
|---------|---|------|
| 签名算法名 | `"ecdsa-sha2-nistp256"` | 固定，直接用 key_type |
| 公钥 blob | `string type` + `string curve` + `string ec_point` | 直接用信封头 `pubkey_blob` |
| 签名密钥 | OpenSSL `EVP_PKEY*` | 用 curve + ec_point + d 构建 EC 对象 |

构建签名密钥：
```c
EC_KEY *ec = EC_KEY_new_by_curve_name(nid);  // nid 从 curve name 推导
EC_KEY_set_public_key(ec, ec_point);
EC_KEY_set_private_key(ec, d);
EVP_PKEY_assign_EC_KEY(pkey, ec);
```

### 8.6 ECDSA 证书（`ecdsa-sha2-nistp256-cert-v01@openssh.com` / `nistp384` / `nistp521`）

**解析出的字段：**

| 位置 | 字段 | 内容 |
|------|------|------|
| 信封头 | `pubkey_blob` | 完整 certblob（含 type + nonce + curve + ec_point + 证书元数据 + CA 签名） |
| 加密区 | `key_type` | `"ecdsa-sha2-nistp256-cert-v01@openssh.com"`（或 nistp384/nistp521） |
| 加密区 | `cert_blob` | 同上（冗余，跳过） |
| 加密区 | `d` | mpint（私钥标量） |

**映射：**

| SSH 要素 | 值 | 来源 |
|---------|---|------|
| 签名算法名 | `"ecdsa-sha2-nistp256"` / `"ecdsa-sha2-nistp384"` / `"ecdsa-sha2-nistp521"` | 固定，由 key_type 去掉证书后缀后的基础算法名 |
| 公钥 blob | 完整 certblob | 信封头 `pubkey_blob` |
| 签名密钥 | OpenSSL `EVP_PKEY*` | 需从 certblob 提取 curve + ec_point，再与 d 构建 EC 对象 |

三种曲线的哈希算法绑定规则（与 8.5 相同）：
- `nistp256` → SHA-256
- `nistp384` → SHA-384
- `nistp521` → SHA-512

### 8.7 Ed25519-SK（`sk-ssh-ed25519@openssh.com`）

**解析出的字段：**

| 位置 | 字段 | 内容 |
|------|------|------|
| 信封头 | `pubkey_blob` | `string type` + `string pubkey_32bytes` + `string application` |
| 加密区 | `key_type` | `"sk-ssh-ed25519@openssh.com"` |
| 加密区 | `pubkey` | 32 字节公钥 |
| 加密区 | `application` | FIDO RP ID（通常 `"ssh:"`） |
| 加密区 | `flags` | 1 字节 |
| 加密区 | `key_handle` | FIDO 密钥句柄 |
| 加密区 | `reserved` | 保留字段（通常空） |

**映射：**

| SSH 要素 | 值 | 来源 |
|---------|---|------|
| 签名算法名 | `"sk-ssh-ed25519@openssh.com"` | 固定 |
| 公钥 blob | `string type` + `string pubkey` + `string application`（三部分） | 信封头 `pubkey_blob` |
| 签名密钥 | `key_handle` + `application` + `flags` | 不是传统私钥，传给 FIDO 硬件设备完成签名 |

### 8.8 ECDSA-SK（`sk-ecdsa-sha2-nistp256@openssh.com`）

**映射：**

| SSH 要素 | 值 | 来源 |
|---------|---|------|
| 签名算法名 | `"sk-ecdsa-sha2-nistp256@openssh.com"` | 固定 |
| 公钥 blob | `string type` + `string curve` + `string ec_point` + `string application`（四部分） | 信封头 `pubkey_blob` |
| 签名密钥 | `key_handle` + `application` + `flags` | FIDO 硬件设备签名 |

### 8.9 总表

> **注意**：私钥文件的 key_type 只有 16 种（见 1.5 节）。`rsa-sha2-256`、`rsa-sha2-512` 等签名算法标识（sigonly=1）不会出现在私钥文件中。

| 私钥文件 key_type | 签名算法名 | 公钥 blob 来源 | 签名密钥构建 |
|----------|-----------|---------------|---------------|
| `ssh-ed25519` | 固定 `ssh-ed25519` | 信封头 `pubkey_blob` | 64 字节原始数据直接签名 |
| `ssh-ed25519-cert-v01@…` | 固定 `ssh-ed25519` | 信封头（证书） | 64 字节原始数据直接签名 |
| `ssh-rsa` | **需协商**（rsa-sha2-512/256/ssh-rsa） | 信封头 `pubkey_blob` | n,e,d,p,q,iqmp → OpenSSL RSA |
| `ssh-rsa-cert-v01@…` | **需协商** | 信封头（证书） | 从 certblob 提取 n,e + d,p,q,iqmp → OpenSSL RSA |
| `ecdsa-sha2-nistp256` | 固定 | 信封头 `pubkey_blob` | curve+ec_point+d → OpenSSL EC |
| `ecdsa-sha2-nistp256-cert-v01@…` | 固定 | 信封头（证书） | 从 certblob 提取 curve+ec_point + d → OpenSSL EC |
| `sk-ssh-ed25519@…` | 固定 | 信封头（含 application） | key_handle+application → FIDO |
| `sk-ecdsa-sha2-nistp256@…` | 固定 | 信封头（含 application） | key_handle+application → FIDO |

**共同规律：**
- **公钥 blob**：所有类型都直接从信封头 `pubkey_blob` 取，不需要自己组装
- **签名算法名**：Ed25519/ECDSA/SK 固定；RSA 必须与服务器协商
- **签名密钥**：Ed25519 直接用原始字节；RSA/ECDSA 需构建 OpenSSL 对象；SK 交给硬件

---

## 完整解析伪代码

```python
import struct

def read_u32(data, offset):
    val = struct.unpack('>I', data[offset:offset+4])[0]
    return val, offset + 4

def read_u8(data, offset):
    return data[offset], offset + 1

def read_string(data, offset):
    length, offset = read_u32(data, offset)
    val = data[offset:offset+length]
    return val, offset + length

def read_mpint(data, offset):
    length, offset = read_u32(data, offset)
    raw = data[offset:offset+length]
    offset += length
    if length > 0 and raw[0] & 0x80:
        raise ValueError("负数 mpint")
    trimmed = raw.lstrip(b'\x00')
    value = int.from_bytes(trimmed, 'big') if trimmed else 0
    return value, offset

def parse_decrypted_private_key(plain):
    """解析解密后的私钥明文区"""
    p = 0

    # 1. check bytes
    check1, p = read_u32(plain, p)
    check2, p = read_u32(plain, p)
    assert check1 == check2, "密码错误（check 不匹配）"

    # 2. key type
    key_type, p = read_string(plain, p)
    is_cert = key_type.endswith(b"-cert-v01@openssh.com")

    # 提取基础类型（去掉证书后缀）
    if is_cert:
        base_type = key_type.replace(b"-cert-v01@openssh.com", b"")
    else:
        base_type = key_type

    # 3. 证书密钥先读 cert blob
    cert_blob = None
    if is_cert:
        cert_blob, p = read_string(plain, p)

    # 4. 按基础类型解析（证书和非证书共用同一分支）

    # 分支 1: Ed25519
    # 匹配 key_type:
    #   - ssh-ed25519
    #   - ssh-ed25519-cert-v01@openssh.com
    if base_type == b"ssh-ed25519":
        if not is_cert:
            pubkey, p = read_string(plain, p)   # 32 bytes
        privkey, p = read_string(plain, p)       # 64 bytes
        result = {"type": "ed25519", "pubkey": pubkey, "privkey": privkey}

    # 分支 2: RSA（含 rsa-sha2-* 变体）
    # 匹配 key_type:
    #   - ssh-rsa
    #   - ssh-rsa-cert-v01@openssh.com
    #   - rsa-sha2-256
    #   - rsa-sha2-256-cert-v01@openssh.com
    #   - rsa-sha2-512
    #   - rsa-sha2-512-cert-v01@openssh.com
    elif b"rsa" in base_type:
        n, e = None, None  # 证书密钥时从 cert_blob 获取，此处不读
        if not is_cert:
            n, p = read_mpint(plain, p)          # 注意：n 在前
            e, p = read_mpint(plain, p)          # e 在后
        d,    p = read_mpint(plain, p)
        iqmp, p = read_mpint(plain, p)
        pr,   p = read_mpint(plain, p)
        qr,   p = read_mpint(plain, p)
        dmp1 = d % (pr - 1)
        dmq1 = d % (qr - 1)
        result = {"type": "rsa", "n": n, "e": e, "d": d, "p": pr, "q": qr, "iqmp": iqmp}

    # 分支 3: ECDSA（非 SK）
    # 匹配 key_type:
    #   - ecdsa-sha2-nistp256
    #   - ecdsa-sha2-nistp256-cert-v01@openssh.com
    #   - ecdsa-sha2-nistp384
    #   - ecdsa-sha2-nistp384-cert-v01@openssh.com
    #   - ecdsa-sha2-nistp521
    #   - ecdsa-sha2-nistp521-cert-v01@openssh.com
    elif base_type.startswith(b"ecdsa-sha2-"):
        curve, ec_pt = None, None  # 证书密钥时从 cert_blob 获取
        if not is_cert:
            curve, p = read_string(plain, p)
            ec_pt, p = read_string(plain, p)
        priv_d, p = read_mpint(plain, p)
        result = {"type": "ecdsa", "curve": curve, "ec_point": ec_pt, "priv_d": priv_d}

    # 分支 4: SK 密钥（硬件安全密钥）
    # 匹配 key_type:
    #   - sk-ssh-ed25519@openssh.com
    #   - sk-ssh-ed25519-cert-v01@openssh.com
    #   - sk-ecdsa-sha2-nistp256@openssh.com
    #   - sk-ecdsa-sha2-nistp256-cert-v01@openssh.com
    #   - webauthn-sk-ecdsa-sha2-nistp256@openssh.com
    #   - webauthn-sk-ecdsa-sha2-nistp256-cert-v01@openssh.com
    elif b"sk-" in base_type:
        if b"ed25519" in base_type:
            if not is_cert:
                pubkey, p = read_string(plain, p)
        elif b"ecdsa" in base_type:
            if not is_cert:
                curve, p = read_string(plain, p)
                ec_pt, p = read_string(plain, p)
        application, p = read_string(plain, p)
        flags,       p = read_u8(plain, p)
        key_handle,  p = read_string(plain, p)
        reserved,    p = read_string(plain, p)
        result = {"type": key_type, "key_handle": key_handle}

    # 5. comment
    comment, p = read_string(plain, p)

    # 6. 验证 padding
    i = 0
    while p < len(plain):
        i += 1
        assert plain[p] == (i & 0xFF), f"padding 错误 at {p}"
        p += 1

    result["comment"] = comment
    return result
```

---

## Ed25519 无密码密钥完整解剖

一个典型的无密码 Ed25519 私钥文件：

```
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtz
c2gtZWQyNTUxOQAAACB[...公钥32字节...]AAAAkAAAAC3z
c2gtZWQyNTUxOQAAACB[...公钥32字节...]AAAABB[...私钥64字节...]AAAAY
[...comment...]AAAABAECAwQ
-----END OPENSSH PRIVATE KEY-----
```

Base64 解码后的二进制结构：

```
"openssh-key-v1\0"                  ← AUTH_MAGIC (15 bytes)
[00 00 00 04] "none"                ← ciphername
[00 00 00 04] "none"                ← kdfname
[00 00 00 00]                        ← kdf_options (空)
[00 00 00 01]                        ← num_keys = 1
[00 00 00 XX] [公钥 blob]           ← pubkey_blob
[00 00 00 YY]                        ← encrypted_len
[...明文数据，无加密...]
  ├─ check1 (4 bytes, 随机)
  ├─ check2 (= check1)
  ├─ "ssh-ed25519" (string)
  ├─ pubkey 32 bytes (string)
  ├─ privkey 64 bytes (string)
  ├─ comment (string)
  └─ padding: 01 02 03 ...
```

---

## 关键源码文件索引

| 文件 | 功能 |
|------|------|
| `sshkey.c` | 核心：密钥类型注册、序列化/反序列化、文件格式解析 |
| `sshkey.h` | 密钥类型枚举、`struct sshkey` 定义 |
| `cipher.c` | 加密算法表、`cipher_init`/`cipher_crypt` |
| `ssh-ed25519.c` | Ed25519 序列化/反序列化/签名/验证 |
| `ssh-rsa.c` | RSA 序列化/反序列化/签名/验证 |
| `ssh-ecdsa.c` | ECDSA 序列化/反序列化/签名/验证 |
| `ssh-ed25519-sk.c` | Ed25519-SK 序列化/反序列化 |
| `ssh-ecdsa-sk.c` | ECDSA-SK 序列化/反序列化 |
| `sshbuf-getput-basic.c` | string/mpint 编码基础函数 |
| `sshbuf-getput-crypto.c` | EC point 编码 |
| `sshbuf-misc.c` | Base64 编解码 |
| `authfile.c` | 文件加载逻辑（含前导空白跳过） |
| `sshconnect2.c` | 客户端认证流程（`sign_and_send_pubkey`） |
| `openbsd-compat/bcrypt_pbkdf.c` | KDF 实现 |
