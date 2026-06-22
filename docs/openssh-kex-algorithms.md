# OpenSSH 密钥交换算法（KEX）

> 基于 OpenSSH Portable 源码分析，涵盖所有支持的密钥交换算法、默认配置与工作原理。

---

## 目录

1. [全部支持的 KEX 算法](#1-全部支持的-kex-算法)
2. [默认启用配置](#2-默认启用配置)
3. [密钥交换的工作原理](#3-密钥交换的工作原理)
4. [各算法详细交互过程](#4-各算法详细交互过程)
5. [算法分类详解](#5-算法分类详解)
6. [相关源码文件](#6-相关源码文件)

---

## 1. 全部支持的 KEX 算法

OpenSSH 在 `kex-names.c` 的 `kexalgs[]` 数组中注册了 **15 种**密钥交换算法，按类别如下：

### 1.1 后量子混合密钥交换（Post-Quantum Hybrid）

| # | 算法名 | 哈希 | 是否 PQ | 编译条件 |
|---|--------|------|:---:|---------|
| 1 | `mlkem768x25519-sha256` | SHA-256 | 是 | `USE_MLKEM768X25519` |
| 2 | `sntrup761x25519-sha512` | SHA-512 | 是 | `USE_SNTRUP761X25519` |
| 3 | `sntrup761x25519-sha512@openssh.com` | SHA-512 | 是 | `USE_SNTRUP761X25519`（旧名称） |

### 1.2 Curve25519 系列

| # | 算法名 | 哈希 | 编译条件 |
|---|--------|------|---------|
| 4 | `curve25519-sha256` | SHA-256 | `HAVE_EVP_SHA256` 或无 OpenSSL |
| 5 | `curve25519-sha256@libssh.org` | SHA-256 | 同上（旧名称） |

### 1.3 ECDH 系列（需 OpenSSL + ECC 支持）

| # | 算法名 | 曲线 | 哈希 |
|---|--------|------|------|
| 6 | `ecdh-sha2-nistp256` | P-256 | SHA-256 |
| 7 | `ecdh-sha2-nistp384` | P-384 | SHA-384 |
| 8 | `ecdh-sha2-nistp521` | P-521 | SHA-512 |

### 1.4 DH Group Exchange（需 OpenSSL）

| # | 算法名 | 哈希 |
|---|--------|------|
| 9 | `diffie-hellman-group-exchange-sha256` | SHA-256 |
| 10 | `diffie-hellman-group-exchange-sha1` | SHA-1 |

### 1.5 DH 固定群（需 OpenSSL）

| # | 算法名 | 群大小 | 哈希 |
|---|--------|--------|------|
| 11 | `diffie-hellman-group18-sha512` | 8192-bit | SHA-512 |
| 12 | `diffie-hellman-group16-sha512` | 4096-bit | SHA-512 |
| 13 | `diffie-hellman-group14-sha256` | 2048-bit | SHA-256 |
| 14 | `diffie-hellman-group14-sha1` | 2048-bit | SHA-1 |
| 15 | `diffie-hellman-group1-sha1` | 1024-bit | SHA-1 |

> **注意**：使用 SHA-1 的算法（`group1-sha1`、`group14-sha1`、`group-exchange-sha1`）因安全性不足，默认不启用，需通过配置手动开启。

---

## 2. 默认启用配置

定义在 `myproposal.h` 中：

### 服务端默认（`KEX_SERVER_KEX`）

```
mlkem768x25519-sha256,
sntrup761x25519-sha512,
sntrup761x25519-sha512@openssh.com,
curve25519-sha256,
curve25519-sha256@libssh.org,
ecdh-sha2-nistp256,
ecdh-sha2-nistp384,
ecdh-sha2-nistp521
```

### 客户端默认（`KEX_CLIENT_KEX`）

在服务端基础上额外追加：

```
diffie-hellman-group-exchange-sha256,
diffie-hellman-group16-sha512,
diffie-hellman-group18-sha512,
diffie-hellman-group14-sha256
```

客户端比服务端多启用了 4 种 DH 算法，以兼容更多老旧服务端。

---

## 3. 密钥交换的工作原理

### 3.1 KEX 在 SSH 连接中的位置

```
TCP 连接建立
    ↓
版本交换（SSH-2.0-xxx）
    ↓
算法协商（KEX_INIT）   ← 双方交换各自支持的算法列表
    ↓
密钥交换（KEX）        ← 核心步骤：协商共享秘密
    ↓
服务端认证（验证主机密钥签名）
    ↓
用户认证
    ↓
加密通信开始
```

### 3.2 算法协商（KEX_INIT）

双方各自发送一个 `SSH_MSG_KEXINIT` 包，包含 10 组算法偏好列表（KEX 算法、加密算法、MAC 算法、压缩算法等，分客户端→服务端和服务端→客户端两个方向）。双方取交集，选出每个类别中第一个共同支持的算法。

### 3.3 执行密钥交换

根据协商结果执行具体算法。OpenSSH 支持两种交互模式：

- **DH 式**（DH 全系列、ECDH、Curve25519，共 12 种）：双方各自生成密钥对，交换公钥后独立计算相同的共享秘密
- **KEM 式**（后量子混合，共 3 种）：客户端发送公钥，服务端封装一个随机密钥并加密返回，客户端解封获取

各系列算法的核心差异：

| 系列 | 数学基础 | 优势 |
|------|---------|------|
| DH Group | 有限域离散对数 | 经典成熟，群大小可选 |
| DH Group Exchange | 同上，但由服务端提供自定义素数 | 可使用更大的素数 |
| ECDH | 椭圆曲线离散对数 | 计算量小，密钥短 |
| Curve25519 | Montgomery 曲线 | 速度快，抗侧信道攻击 |
| 后量子混合 | 格问题（SNTRUP/ML-KEM）+ X25519 | 抗量子计算攻击 |

> 每种算法的详细交互流程图见 [第 4 节](#4-各算法详细交互过程)。

### 3.4 派生会话密钥

共享秘密 K 不直接用作加密密钥，而是通过密钥派生函数（KDF）结合交换哈希 H 派生出多组密钥。

**派生公式**（`kex.c` 中的 `derive_key()` 函数）：

```
IV_C→S       = Hash(K || H || "A" || session_id)
IV_S→C       = Hash(K || H || "B" || session_id)
enc_key_C→S  = Hash(K || H || "C" || session_id)
enc_key_S→C  = Hash(K || H || "D" || session_id)
mac_key_C→S  = Hash(K || H || "E" || session_id)
mac_key_S→C  = Hash(K || H || "F" || session_id)
```

各符号含义：

| 符号 | 含义 |
|------|------|
| `\|\|` | **拼接（concatenation）**：将两段字节数据首尾相接，如 `[0x01,0x02] \|\| [0xAA] = [0x01,0x02,0xAA]` |
| `K` | 密钥交换的共享秘密 |
| `H` | 本次 KEX 的交换哈希 |
| `"A"`~`"F"` | 单字节字符标识，确保 6 次调用产生不同的密钥 |
| `session_id` | 首次 KEX 的 H 值（后续 rekey 时不变，用于隔离不同会话） |
| `Hash` | 与当前 KEX 算法绑定的哈希函数（如 SHA-256） |

如果派生出的密钥长度不够（如需要 64 字节但哈希只输出 32 字节），会继续派生（不再用新字母，而是拼接之前所有块）：

```
K1 = Hash(K || H || "C" || session_id)               // 第 1 块
K2 = Hash(K || H || K1)                              // 第 2 块，用 K1 代替字母
Kn = Hash(K || H || K1 || K2 || ... || Kn-1)         // 第 n 块，拼接之前所有块
enc_key = K1 || K2
```

**源码实现**（`kex.c`）：

```c
// =============================================================================
// derive_key() - 派生单组密钥（id 为 'A'~'F' 中的一个）
// =============================================================================
static int
derive_key(struct ssh *ssh, int id, u_int need, u_char *hash, u_int hashlen,
    const struct sshbuf *shared_secret, u_char **keyp)
{
	struct kex *kex = ssh->kex;
	char c = id;
	size_t mdsz = ssh_digest_bytes(kex->hash_alg);  // 哈希输出大小（如 32 字节）
	u_char *digest = calloc(1, ROUNDUP(need, mdsz));

	// 第 1 块：K1 = HASH(K || H || id || session_id)
	ssh_digest_update_buffer(hashctx, shared_secret);  // K
	ssh_digest_update(hashctx, hash, hashlen);          // H
	ssh_digest_update(hashctx, &c, 1);                  // 字母标识 ('A'~'F')
	ssh_digest_update_buffer(hashctx, kex->session_id); // session_id
	ssh_digest_final(hashctx, digest, mdsz);            // K1 = digest[0..mdsz-1]

	// 扩展块：Kn = HASH(K || H || K1 || K2 || ... || Kn-1)
	for (have = mdsz; need > have; have += mdsz) {
		ssh_digest_update_buffer(hashctx, shared_secret);  // K
		ssh_digest_update(hashctx, hash, hashlen);          // H
		ssh_digest_update(hashctx, digest, have);           // 之前所有块拼接
		ssh_digest_final(hashctx, digest + have, mdsz);     // 追加到末尾
	}
	// 最终 digest[0..need-1] 即为派生的密钥材料
	*keyp = digest;
}

// =============================================================================
// kex_derive_keys() - 派生 6 组会话密钥并分配给两个方向
// =============================================================================
#define NKEYS	6
int
kex_derive_keys(struct ssh *ssh, u_char *hash, u_int hashlen,
    const struct sshbuf *shared_secret)
{
	struct kex *kex = ssh->kex;
	u_char *keys[NKEYS];

	// 首次 KEX 时，保存 H 作为 session_id（后续 rekey 时不变）
	if ((kex->flags & KEX_INITIAL) != 0) {
		sshbuf_put(kex->session_id, hash, hashlen);
	}

	// 派生 6 组密钥（id 从 'A' 到 'F'）
	for (i = 0; i < NKEYS; i++) {
		derive_key(ssh, 'A'+i, kex->we_need, hash, hashlen,
		    shared_secret, &keys[i]);
	}

	// 分配密钥到两个方向（c→s 和 s→c）
	for (mode = 0; mode < MODE_MAX; mode++) {
		ctos = (!kex->server && mode == MODE_OUT) ||
		    (kex->server && mode == MODE_IN);
		kex->newkeys[mode]->enc.iv  = keys[ctos ? 0 : 1];  // IV
		kex->newkeys[mode]->enc.key = keys[ctos ? 2 : 3];  // 加密密钥
		kex->newkeys[mode]->mac.key = keys[ctos ? 4 : 5];  // MAC 密钥
	}
	return 0;
}
```

**密钥分配逻辑**：

```
keys[0] → IV_C→S       (客户端→服务端 初始向量)
keys[1] → IV_S→C       (服务端→客户端 初始向量)
keys[2] → enc_key_C→S  (客户端→服务端 加密密钥)
keys[3] → enc_key_S→C  (服务端→客户端 加密密钥)
keys[4] → mac_key_C→S  (客户端→服务端 MAC 密钥)
keys[5] → mac_key_S→C  (服务端→客户端 MAC 密钥)
```

> **注意 `ctos` 的含义**：客户端视角下 `MODE_OUT` 是 c→s（`ctos=true`），服务端视角下 `MODE_IN` 是 c→s（`ctos=true`）。这样保证客户端的发送密钥与服务端的接收密钥是同一组。

**`session_id` 的生成与维持**：

`session_id` 是 SSH 会话的唯一标识，其生成规则如下（源码见 `kex_derive_keys()` 开头）：

```
首次 KEX（初始连接）:  session_id = H（首次交换哈希）
后续 rekey（重新协商）:  session_id 保持不变（仍为首次的 H）
```

源码逻辑（`kex.c`）：

```c
if ((kex->flags & KEX_INITIAL) != 0) {
    // 首次 KEX：必须还没有 session_id
    if (sshbuf_len(kex->session_id) != 0)
        return SSH_ERR_INTERNAL_ERROR;
    // 将本次 H 存为 session_id
    sshbuf_put(kex->session_id, hash, hashlen);
} else {
    // rekey：必须已经有 session_id，且不允许覆盖
    if (sshbuf_len(kex->session_id) == 0)
        return SSH_ERR_INTERNAL_ERROR;
    // session_id 不变，仍用首次的 H
}
```

**为什么要区分首次和 rekey？**

```
时间线：
  首次 KEX:    H₁ → session_id = H₁
               密钥 = Hash(K₁ || H₁ || id || H₁)

  rekey #1:    H₂（新的临时密钥、新的 H）
               密钥 = Hash(K₂ || H₂ || id || H₁)  ← session_id 仍为 H₁

  rekey #2:    H₃
               密钥 = Hash(K₃ || H₃ || id || H₁)  ← session_id 仍为 H₁
```

- 每次 rekey 会产生新的 K 和新的 H（因为临时密钥不同）
- 但 session_id 固定为首次的 H₁，确保同一会话内所有密钥派生都绑定到同一个会话标识
- 这防止了攻击者在 rekey 时注入伪造的 KEX_INIT 来替换 session_id

### 3.5 交换哈希 H 的作用

整个 KEX 过程中所有的公开参数（双方版本号、KEX_INIT 包内容、服务端主机公钥、DH/ECDH 公开值等）会被哈希成一个**交换哈希 H**，有两个关键用途：

- **绑定身份**：服务端用自己的主机私钥对 H 签名，客户端验证该签名 → 防止中间人攻击
- **密钥隔离**：H 参与密钥派生，确保每次连接的会话密钥都不同

### 3.6 后量子混合 KEX 的特殊设计

`sntrup761x25519-sha512` 和 `mlkem768x25519-sha256` 采用 **KEM（密钥封装机制）**，与 DH 式算法的交互模式完全不同，详见 [4.5](#45-sntrup761x25519-混合-kem2-种) 和 [4.6](#46-ml-kem768x25519-混合-kem1-种)。

---

## 4. 各算法详细交互过程

OpenSSH 的 15 种 KEX 算法按交互模式分为三类：

| 模式 | 适用算法 | 消息轮数 | 特点 |
|------|---------|:---:|------|
| DH 固定群 | `diffie-hellman-group{1,14,16,18}-*`（5 种） | 2 | 双方对等，使用预定义素数 |
| DH Group Exchange | `diffie-hellman-group-exchange-*`（2 种） | 3 | 多一轮协商素数 |
| ECDH | `ecdh-sha2-nistp*`（3 种） | 2 | 椭圆曲线版 DH |
| Curve25519 | `curve25519-sha256*`（2 种） | 2 | Montgomery 曲线 DH |
| 后量子 KEM 混合 | `sntrup761x25519-*`、`mlkem768x25519-*`（3 种） | 2 | 不对称：封装/解封模式 |

### 4.1 DH 固定群（5 种）

**适用算法**：`diffie-hellman-group1-sha1`、`diffie-hellman-group14-sha1`、`diffie-hellman-group14-sha256`、`diffie-hellman-group16-sha512`、`diffie-hellman-group18-sha512`

**源码文件**：`kexdh.c` + `kexgen.c`

**交互流程（2 条消息）**：

```
客户端                                          服务端
  │                                                │
  │ ① 从 RFC 预定义群中加载 (p, g)                  │
  │    （如 group14: 2048-bit 素数 p，生成元 g=2）    │
  │ ② 随机生成私钥 x（临时，CSPRNG）                  │
  │ ③ 计算公钥 e = g^x mod p                        │
  │                                                │
  │─── SSH2_MSG_KEXDH_INIT {e} ───────────────────>│
  │                                                │
  │                              ④ 加载同一组 (p, g)  │
  │                              ⑤ 随机生成私钥 y      │
  │                              ⑥ 计算公钥 f = g^y mod p
  │                              ⑦ 计算 K = e^y mod p  │
  │                              ⑧ 计算交换哈希 H       │
  │                              ⑨ 用主机私钥签名 H     │
  │                                                │
  │<── SSH2_MSG_KEXDH_REPLY {hostkey, f, sig} ─────│
  │                                                │
  │ ⑩ 验证主机密钥签名                                │
  │ ⑪ 计算 K = f^x mod p（= e^y mod p，数学相等）    │
  │ ⑫ 计算相同的交换哈希 H                            │
  │ ⑬ 派生 6 组会话密钥（见 3.4 节）                  │
  │                                                │
```

**关键源码调用链**：

| 步骤 | 客户端函数 | 服务端函数 |
|------|-----------|----------|
| 加载群参数 | `dh_new_group1/14/16/18()` | 同左 |
| 生成密钥对 | `dh_gen_key(kex->dh, need*8)` | 同左 |
| 发送/接收公钥 | `kex_dh_keypair()` → `kex_dh_enc()` | `kex_dh_enc()` |
| 计算共享秘密 | `DH_compute_key()` via `kex_dh_dec()` | `DH_compute_key()` via `kex_dh_enc()` |

**各群的参数**：

| 群名 | 素数 p 大小 | 生成元 g | 来源 RFC |
|------|:---------:|:-------:|:-------:|
| group1 | 1024-bit | 2 | RFC 2409 |
| group14 | 2048-bit | 2 | RFC 3526 |
| group16 | 4096-bit | 2 | RFC 3526 |
| group18 | 8192-bit | 2 | RFC 3526 |

#### 4.1.1 私钥 x 的生成机制

DH 私钥 `x` 是每次连接时临时生成的随机大整数，不存储、不复用。其生成由 `dh.c` 中的 `dh_gen_key()` 函数完成，完整流程如下：

```c
int dh_gen_key(DH *dh, int need) {
    // need = kex->we_need * 8（对称密钥材料最大字节数 × 8 = 位数）

    // ① 最低安全位数为 256 bit
    if (need < 256)
        need = 256;

    // ② 设置 x 的位长度（翻倍，因为 Pollard Rho 攻击复杂度为 O(√n)）
    //    封顶于 pbits - 1
    DH_set_length(dh, MINIMUM(need * 2, pbits - 1));

    // ③ OpenSSL 通过 CSPRNG（/dev/urandom）生成随机 x，并计算 e = g^x mod p
    DH_generate_key(dh);

    // ④ 验证生成的公钥 e 是否合法
    if (!dh_pub_is_valid(dh, pub_key))
        return SSH_ERR_INVALID_FORMAT;
}
```

**x 的三个生成约束**：

| 约束 | 具体要求 | 原因 |
|------|---------|------|
| **随机性** | 必须来自 CSPRNG（`/dev/urandom`），不可预测 | 防止攻击者猜出 x，直接算出共享秘密 K |
| **最小位数** | `min(need × 2, pbits - 1)` 位 | 抵抗 Pollard Rho 等 O(√n) 复杂度攻击 |
| **值域** | `1 < x < p-1` | 保证公钥 e = g^x mod p 不落入退化值 |

生成后还会通过 `dh_pub_is_valid()` 反向验证公钥 `e`：

- `e > 1`（排除 g^0 = 1 的情况）
- `e < p-1`（排除 g^(p-1) = 1 的情况）
- `e` 的二进制表示中至少有 4 个 1-bit（防止弱值，如 g^1、g^2 等使离散对数变得微不足道）

#### 4.1.2 x 位长度的计算链路

`x` 的位长度并非固定值，而是取决于本次连接协商出的加密和 MAC 算法所需的最大密钥材料。

**`we_need` 的含义**：

`kex->we_need` 是本次连接两个方向（c→s 和 s→c）中，所有派生密钥材料长度的最大值（单位：字节）。计算逻辑在 `kex.c` 中：

```c
need = 0;
for (mode = 0; mode < MODE_MAX; mode++) {   // MODE_MAX = 2，遍历两个方向
    newkeys = kex->newkeys[mode];
    need = MAXIMUM(need, newkeys->enc.key_len);    // 加密密钥长度
    need = MAXIMUM(need, newkeys->enc.block_size);  // 分组大小
    need = MAXIMUM(need, newkeys->enc.iv_len);      // IV 长度
    need = MAXIMUM(need, newkeys->mac.key_len);     // MAC 密钥长度
}
kex->we_need = need;
```

> **关于 `MODE_MAX`**：SSH 协议允许两个方向使用不同的加密/MAC 算法（如 c→s 用 aes128-ctr，s→c 用 aes256-ctr），因此需要遍历两个方向取最大值。OpenSSH 的默认提案中双向算法列表相同，实际结果通常一致，但此设计保证了协议兼容性。

**完整计算链路**：

```
协商的加密/MAC 算法 → we_need（字节）→ ×8 → need（bit）→ ×2（抗攻击翻倍）→ x 位长度 → 受 pbits-1 封顶
```

**不同算法组合下的 x 位长度示例**（以 group14, p=2048-bit 为例）：

| 加密算法 | MAC 算法 | we_need | need (bit) | x 位长度 |
|---------|---------|:------:|:---------:|:-------:|
| aes128-ctr | hmac-sha2-256 | 32 | 256 | 512 |
| aes256-ctr | hmac-sha2-256 | 32 | 256 | 512 |
| aes256-ctr | hmac-sha2-512 | 64 | 512 | 1024 |
| chacha20-poly1305 | （隐式 MAC） | 64 | 512 | 1024 |

**各群在不同算法组合下的 x 位长度**：

| 群 | p 位数 | we_need=32 时 | we_need=64 时 |
|----|:-----:|:-----------:|:-----------:|
| group1 | 1024 | min(512, 1023) = **512** | min(1024, 1023) = **1023** |
| group14 | 2048 | min(512, 2047) = **512** | min(1024, 2047) = **1024** |
| group16 | 4096 | min(512, 4095) = **512** | min(1024, 4095) = **1024** |
| group18 | 8192 | min(512, 8191) = **512** | min(1024, 8191) = **1024** |

#### 4.1.3 RFC/标准对 x 长度的规定

DH 算法标准本身**不强制规定** x 的长度，但有多份 RFC 给出建议：

| 标准 | 规定内容 | 约束级别 |
|------|---------|----------|
| **RFC 2631**（DH 核心标准） | x 必须从 `[2, q-2]` 中随机选取（q 为子群阶） | MUST（值域约束） |
| **RFC 4419**（SSH DH GEX） | 私钥位数应至少为对称密钥位数的两倍 | SHOULD（推荐） |
| **NIST SP 800-56A** | 按群大小给出推荐最小 x 位数（见下表） | 推荐 |

NIST 推荐的最小 x 位数：

| 群 p 大小 | 推荐 x 最小位数 | 对应安全等级 |
|:---------:|:-------------:|:---------:|
| 2048-bit | 224 | 112-bit |
| 3072-bit | 256 | 128-bit |
| 4096-bit | 304 | 140-bit |
| 8192-bit | 404 | 175-bit |

OpenSSH 的策略（`we_need × 2`）遵循 RFC 4419 建议，在典型场景下（we_need=256 bit → x=512 bit）已超过 NIST 的推荐值。

> **数学上，任意满足 `1 < x < p-1` 的 x 都能使 DH 协议正确工作**。即使 `x = 2`，双方也能算出相同的共享秘密。但 `x = 2` 的安全性为零——攻击者无需破解离散对数即可猜出 x。因此长度约束纯粹是安全性考量，而非协议正确性要求。

#### 4.1.4 素数 p 的硬编码值与递增嵌套结构

4 个素数 p 全部硬编码在 `dh.c` 中，生成元 g 统一为 2。5 种算法实际只用了 4 个不同的素数（`group14-sha1` 和 `group14-sha256` 共用 group14）。

**递增嵌套结构**：4 个素数共享相同的高位前缀，更大的群是在较小群的基础上追加低位：

```
Group 1  (1024-bit): FF...F C90FDAA2...ECE65381 FF...F
Group 14 (2048-bit): FF...F C90FDAA2...ECE45B3D ... AA68 FF...F    ← 前 1024 位与 group1 相同
Group 16 (4096-bit): FF...F C90FDAA2...          ... 3199 FF...F    ← 前 2048 位与 group14 相同
Group 18 (8192-bit): FF...F C90FDAA2...          ... D3DF FF...F    ← 前 4096 位与 group16 相同
```

这是因为它们都按以下公式构造（RFC 3526）：

```
p = 2^k - 2^(k-64) - 1 + 2^64 × (⌊2^(k-130) × π⌋ + 124476)
```

其中 π 是圆周率。这种构造保证了：

1. p 是**安全素数**（`p = 2q + 1`，`q` 也是素数）
2. 使用 π 的位数保证了素数的“不可预测性”（无后门嫌疑）
3. 高位和低位都是 `FF...FF`，优化模运算性能

> `C90FDAA2...` 这个共享开头正是 π 的二进制编码决定的。

#### 4.1.5 RFC 定义的完整 DH 群列表

OpenSSH 实现的 4 个群只是 RFC 定义的一个子集。以下是各 RFC 中定义的全部 DH 群：

**RFC 2409（1998）— Oakley Groups**

| 群编号 | 大小 | OpenSSH 实现 | 状态 |
|:------:|:---:|:---:|------|
| Group 1 | 768-bit | ❌ | 已废弃，不安全 |
| Group 2 | 1024-bit | ✅（名为 group1） | 遗留使用 |

**RFC 3526（2003）— MODP Groups（当前主力）**

| 群编号 | 大小 | OpenSSH 实现 | 其他实现支持 |
|:------:|:---:|:---:|------|
| Group 5 | 1536-bit | ❌ | libssh |
| **Group 14** | **2048-bit** | **✅** | 广泛支持 |
| Group 15 | 3072-bit | ❌ | libssh, PuTTY, paramiko |
| **Group 16** | **4096-bit** | **✅** | 广泛支持 |
| Group 17 | 6144-bit | ❌ | 少数实现 |
| **Group 18** | **8192-bit** | **✅** | 广泛支持 |

**RFC 5114（2008）— Additional Groups（非安全素数）**

| 群编号 | 大小 | 特点 | SSH 中使用 |
|:------:|:---:|------|:---:|
| Group 22 | 1024-bit | 160-bit 子群阶（`p ≠ 2q+1`） | ❌ |
| Group 23 | 2048-bit | 224-bit 子群阶 | ❌ |
| Group 24 | 2048-bit | 256-bit 子群阶 | ❌ |

> **RFC 8268**（2017）本身不定义新素数，仅将 RFC 3526 中的群注册为标准 SSH 算法名（如 `diffie-hellman-group15-sha512`），并规定 wire format 和哈希算法的绑定关系。

**OpenSSH 跳过 group15/group17 的原因**：OpenSSH 选择群的标准是 2 的整数次幂（1024 → 2048 → 4096 → 8192）。group15（3072-bit）对应 128-bit 安全等级，与 group14（112-bit）差距不大，OpenSSH 认为不需要这个“中间档”；group17（6144-bit）同理。

**协议兼容性说明**：SSH 协议的算法协商机制是开放的，只要双方都支持同一个算法名并使用相同的素数 p（来自 RFC 3526），就能正常完成密钥交换。因此其他 SSH 实现（如 libssh、PuTTY）支持 group15/group17 是完全合法的。

### 4.2 DH Group Exchange（2 种）

**适用算法**：`diffie-hellman-group-exchange-sha1`、`diffie-hellman-group-exchange-sha256`

**源码文件**：`kexgexc.c`（客户端）+ `kexgexs.c`（服务端）+ `kexgex.c`

**交互流程（3 条消息，比固定群多一轮）**：

```
客户端                                          服务端
  │                                                │
  │ ① 确定所需素数位数范围                            │
  │    min=2048, nbits=目标位数, max=8192            │
  │                                                │
  │─── SSH2_MSG_KEX_DH_GEX_REQUEST ───────────────>│
  │    {min, nbits, max}                            │
  │                                                │
  │                     ② 从 moduli 文件中选择满足条件的
  │                        素数 p 和生成元 g           │
  │                     ③ 随机生成私钥 y               │
  │                     ④ 计算公钥 f = g^y mod p      │
  │                                                │
  │<── SSH2_MSG_KEX_DH_GEX_GROUP {p, g} ──────────│
  │                                                │
  │ ⑤ 验证 p 的位数在 [min, max] 范围内               │
  │ ⑥ 用收到的 (p, g) 随机生成私钥 x                   │
  │ ⑦ 计算公钥 e = g^x mod p                        │
  │                                                │
  │─── SSH2_MSG_KEX_DH_GEX_INIT {e} ──────────────>│
  │                                                │
  │                     ⑧ 计算 K = e^y mod p          │
  │                     ⑨ 计算交换哈希 H（含 p,g 参数）│
  │                     ⑩ 用主机私钥签名 H             │
  │                                                │
  │<── SSH2_MSG_KEX_DH_GEX_REPLY ──────────────────│
  │    {hostkey, f, sig}                            │
  │                                                │
  │ ⑪ 验证签名                                      │
  │ ⑫ 计算 K = f^x mod p                           │
  │ ⑬ 派生 6 组会话密钥（见 3.4 节）                  │
```

**与固定群的核心区别**：

| 对比项 | DH 固定群 | DH Group Exchange |
|--------|----------|-------------------|
| 消息轮数 | 2 | 3（多一轮 GEX_REQUEST/GROUP） |
| 素数来源 | 硬编码在代码中 | 服务端从 `/etc/ssh/moduli` 文件动态选择 |
| 素数灵活性 | 固定的几个群 | 可按需选择任意大小的素数 |
| 哈希 H 内容 | 不含 p, g | 包含 p, g, min, nbits, max |

**服务端选择素数的逻辑**（`kexgexs.c`）：

```c
// 服务端收到 (min, nbits, max) 后，通过特权进程选择
kex->dh = mm_choose_dh(min, nbits, max);
```

`mm_choose_dh()` 会从 `moduli` 文件中挑选一个位数最接近 `nbits`、且在 `[min, max]` 范围内的素数。

#### 4.2.1 min、nbits、max 的来源

客户端在发送 `GEX_REQUEST` 时需要确定三个参数：`min`、`nbits`、`max`。这三个值由客户端的安全策略决定，**RFC 没有规定具体值**，只规定了语义：

- `min`：客户端可接受的最小素数位数
- `nbits`：客户端偏好的素数位数
- `max`：客户端可接受的最大素数位数

**OpenSSH 的实现**（`kexgexc.c`）：

```c
nbits = dh_estimate(kex->dh_need * 8);   // 根据对称安全强度动态计算
kex->min = DH_GRP_MIN;                    // 硬编码 2048
kex->max = DH_GRP_MAX;                    // 硬编码 8192
```

**min 和 max：硬编码常量**（定义在 `dh.h`）：

```c
/* Max value from RFC4419, Min value from RFC8270 */
#define DH_GRP_MIN  2048    // RFC 8270 规定的最低安全要求
#define DH_GRP_MAX  8192    // RFC 4419 示例值
```

| 常量 | 值 | 来源 |
|------|:---:|------|
| `DH_GRP_MIN` | 2048 | RFC 8270（2017）规定的最低安全要求 |
| `DH_GRP_MAX` | 8192 | RFC 4419（2006）示例值 |

**nbits：根据对称安全强度动态计算**（`dh.c`）：

```c
u_int dh_estimate(int bits) {
    if (bits <= 112) return 2048;   // 对应 112-bit 安全强度
    if (bits <= 128) return 3072;   // 对应 128-bit 安全强度
    if (bits <= 192) return 7680;   // 对应 192-bit 安全强度
    return 8192;                    // 对应 256-bit 安全强度
}
```

参数 `bits = kex->dh_need * 8`，其中 `kex->dh_need` 是本次连接协商出的加密算法所需的安全强度（字节），乘以 8 转为比特。对应关系参考 **RFC 8270 / NIST SP 800-56A** 推荐值：

| 对称安全强度（bit） | 推荐 nbits | 典型加密算法 |
|:---:|:---:|------|
| 112 | 2048 | aes128-ctr |
| 128 | 3072 | — |
| 192 | 7680 | — |
| 256 | 8192 | aes256-ctr, chacha20-poly1305 |

**计算示例**：

| 协商的加密算法 | dh_need（字节） | dh_need × 8 | dh_estimate() 返回 |
|-------------|:---------:|:---------:|:--------------:|
| aes128-ctr | 16 | 128 | **3072** |
| aes256-ctr | 32 | 256 | **8192** |
| chacha20-poly1305 | 64 | 512 | **8192** |

**服务端的钳位处理**（`kexgexs.c`）：

服务端收到客户端发来的 `(min, nbits, max)` 后会做安全钳位：

```c
// 客户端发来的 min 太小？钳位到 DH_GRP_MIN (2048)
min = MAXIMUM(DH_GRP_MIN, min);

// 客户端发来的 max 太大？钳位到 DH_GRP_MAX (8192)
max = MINIMUM(DH_GRP_MAX, max);

// 钳位后如果 min > max 或 nbits 不合理，拒绝连接
if (min > max || nbits < min || nbits > max)
    return SSH_ERR_DH_GEX_OUT_OF_RANGE;
```

**客户端的验证**（`kexgexc.c`）：

客户端收到服务端返回的素数 p 后，验证其位数是否在请求范围内：

```c
if ((bits = BN_num_bits(p)) < kex->min || bits > kex->max) {
    r = SSH_ERR_DH_GEX_OUT_OF_RANGE;  // 拒绝不在范围内的素数
}
```

**兼容处理**：如果服务端是老旧实现（`SSH_BUG_DHGEX_LARGE`），客户端会将 nbits 限制在 4096 以内：

```c
if (ssh->compat & SSH_BUG_DHGEX_LARGE)
    kex->nbits = MINIMUM(kex->nbits, 4096);
```

> **其他 SSH 实现**：min/max 是各实现自定义的安全策略，不是协议统一值。旧版 OpenSSH（< 7.0）的 `DH_GRP_MIN` 曾是 1024，RFC 8270 发布后才提高到 2048。

### 4.3 ECDH（3 种）

**适用算法**：`ecdh-sha2-nistp256`、`ecdh-sha2-nistp384`、`ecdh-sha2-nistp521`

**源码文件**：`kexecdh.c`

**交互流程（2 条消息）**：

```
客户端                                          服务端
  │                                                │
  │ ① 在指定曲线上生成 EC 密钥对                      │
  │    EC_KEY_new_by_curve_name(nid)                │
  │    EC_KEY_generate_key()                         │
  │    → 私钥 d（随机标量），公钥 Q = d·G（曲线点）     │
  │                                                │
  │─── SSH2_MSG_KEX_ECDH_INIT {Q_C} ─────────────>│
  │    （Q_C = 客户端公钥，椭圆曲线上的一个点）          │
  │                                                │
  │                     ② 在同一曲线上生成 EC 密钥对    │
  │                        → 私钥 d_S，公钥 Q_S        │
  │                     ③ 计算共享点 P = d_S · Q_C     │
  │                        （椭圆曲线标量乘法）          │
  │                     ④ K = P 的 x 坐标（整数）       │
  │                     ⑤ 计算 H，签名                 │
  │                                                │
  │<── SSH2_MSG_KEX_ECDH_REPLY ────────────────────│
  │    {hostkey, Q_S, sig}                          │
  │                                                │
  │ ⑥ 计算共享点 P = d_C · Q_S                       │
  │ ⑦ K = P 的 x 坐标（与 ④ 相同，ECDH 数学保证）     │
  │ ⑧ 验证签名，派生 6 组会话密钥（见 3.4 节）          │
```

**与 DH 的数学对应关系**：

| DH | ECDH |
|----|------|
| 有限域 `Z_p` | 椭圆曲线群 `E(F_q)` |
| 幂运算 `g^x mod p` | 标量乘法 `x·G`（G 为基点） |
| 共享秘密 `f^x mod p` | 共享点 `x·Q`（取 x 坐标） |
| 安全性：离散对数 | 安全性：椭圆曲线离散对数 |

**各曲线的参数**：

| 曲线 | OID | 私钥 d 大小 | 共享秘密大小 | OpenSSL NID |
|------|-----|:---------:|:---------:|:-----------:|
| P-256 | 1.2.840.10045.3.1.7 | 256-bit | 32 字节 | `NID_X9_62_prime256v1` |
| P-384 | 1.3.132.0.34 | 384-bit | 48 字节 | `NID_secp384r1` |
| P-521 | 1.3.132.0.35 | 521-bit | 66 字节 | `NID_secp521r1` |

**关键源码**（`kexecdh.c`）：

```c
// 客户端生成密钥对
client_key = EC_KEY_new_by_curve_name(kex->ec_nid);
EC_KEY_generate_key(client_key);  // 随机生成私钥 d，计算公钥 Q = d·G

// 计算共享秘密（双方调用同一函数）
ECDH_compute_key(kbuf, klen, dh_pub, key, NULL);
// 内部执行：P = 私钥·对方公钥，取 P 的 x 坐标作为 K
```

#### 4.3.1 ECDH 数学原理

ECDH（Elliptic Curve Diffie-Hellman）是经典 DH 算法在椭圆曲线上的类比实现。两者数学结构完全对应，但运算基础从有限域乘法群变为椭圆曲线加法群。

**核心概念**：

- **椭圆曲线** `E(F_p)`：定义在有限域 `F_p` 上的所有点 `(x, y)` 满足方程 `y² = x³ + ax + b (mod p)`，加上一个“无穷远点” `O`
- **基点 `G`**：曲线上一个预定义的固定点，其阶 `n` 是大素数（即 `n·G = O`）
- **标量乘法 `k·G`**：将 `G` 与自身相加 `k` 次（曲线上的点加法运算）

**密钥交换过程**：

```
客户端：
  ① 随机生成私钥 d_C ∈ [1, n-1]    （大整数）
  ② 计算公钥 Q_C = d_C · G          （曲线上的点）

服务端：
  ③ 随机生成私钥 d_S ∈ [1, n-1]
  ④ 计算公钥 Q_S = d_S · G

双方各自计算共享点：
  客户端: P = d_C · Q_S = d_C · (d_S · G) = (d_C · d_S) · G
  服务端: P = d_S · Q_C = d_S · (d_C · G) = (d_C · d_S) · G
  结果相同！

共享秘密: K = P.x （取共享点 P 的 x 坐标）
```

**安全性**：基于椭圆曲线离散对数问题（ECDLP）——已知 `Q` 和 `G`，求 `d` 使得 `Q = d·G`，在计算上是不可行的。

**与经典 DH 的数学对应**：

| 概念 | 经典 DH | ECDH |
|------|--------|------|
| 群 | 有限域乘法群 `Z_p*` | 椭圆曲线加法群 `E(F_p)` |
| 群元素 | 大整数 | 曲线上的点 `(x, y)` |
| 群运算 | 模幂 `g^x mod p` | 标量乘法 `x·G` |
| 生成元 | `g`（整数） | `G`（曲线点） |
| 私钥 | `x ∈ [2, p-2]` | `d ∈ [1, n-1]` |
| 公钥 | `e = g^x mod p` | `Q = d·G`（点） |
| 共享秘密 | `K = f^x mod p` | `K = (d·Q).x`（点的 x 坐标） |
| 困难问题 | 离散对数（DLP） | 椭圆曲线离散对数（ECDLP） |
| 同等安全所需位数 | 2048-bit | 256-bit |

**NIST 曲线的参数**：

| 曲线 | 域大小 | 私钥位数 | 安全强度 | 基点阶 n |
|------|:-----:|:-----:|:-----:|------|
| P-256 | 256-bit | 256 | 128-bit | 256-bit 素数 |
| P-384 | 384-bit | 384 | 192-bit | 384-bit 素数 |
| P-521 | 521-bit | 521 | 256-bit | 521-bit 素数 |

#### 4.3.2 OpenSSL 调用链详解

OpenSSH 的 ECDH 完全依赖 OpenSSL 的 EC/ECDH API，共涉及三个核心步骤。以下是每一步的 OpenSSL 调用链：

**步骤 ① 创建曲线对象**：

```c
EC_KEY *client_key = EC_KEY_new_by_curve_name(kex->ec_nid);
```

- `kex->ec_nid` 是 OpenSSL 内部的曲线标识符（整数 NID），由 `kex-names.c` 中的 `kexalgs[]` 注册表绑定
- `EC_KEY_new_by_curve_name()` 一次性完成：分配 `EC_KEY` 对象 + 设置曲线参数 `(p, a, b, G, n, h)`
- 三种曲线对应的 NID：

```c
// kex-names.c 中的绑定
{ KEX_ECDH_SHA2_NISTP256, KEX_ECDH_SHA2, NID_X9_62_prime256v1, SSH_DIGEST_SHA256 }
{ KEX_ECDH_SHA2_NISTP384, KEX_ECDH_SHA2, NID_secp384r1,        SSH_DIGEST_SHA384 }
{ KEX_ECDH_SHA2_NISTP521, KEX_ECDH_SHA2, NID_secp521r1,        SSH_DIGEST_SHA512 }
```

**步骤 ② 生成密钥对**：

```c
if (EC_KEY_generate_key(client_key) != 1) {
    r = SSH_ERR_LIBCRYPTO_ERROR;
    goto out;
}
```

`EC_KEY_generate_key()` 内部流程：

```
① 生成随机私钥 d ∈ [1, n-1]（来自 OpenSSL CSPRNG）
② 计算公钥 Q = d·G（椭圆曲线标量乘法）
③ 验证 Q 是否在曲线上且非无穷远点
④ 将 (d, Q) 存入 EC_KEY 对象
```

生成后可通过以下 API 取出私钥和公钥：

```c
const BIGNUM *private_key = EC_KEY_get0_private_key(client_key);  // 私钥 d
const EC_POINT *public_key = EC_KEY_get0_public_key(client_key);  // 公钥 Q
const EC_GROUP *group = EC_KEY_get0_group(client_key);            // 曲线参数
```

**步骤 ③ 序列化公钥用于发送**：

```c
// 将公钥 Q（曲线点）序列化为 SSH wire format 并放入 sshbuf
sshbuf_put_ec(buf, public_key, group);

// 跳过长度前缀（sshbuf_get_u32 返回后指针移到实际数据开头）
sshbuf_get_u32(buf, NULL);
```

`sshbuf_put_ec()` 内部将 EC 点序列化为 **未压缩格式**：

```
0x04 || x坐标字节 || y坐标字节
```

其中 `0x04` 是未压缩格式标识符。这就是发送给对方的公钥 Q_C 或 Q_S。

**步骤 ④ 计算共享秘密**：

```c
static int
kex_ecdh_dec_key_group(struct kex *kex, const struct sshbuf *ec_blob,
    EC_KEY *key, const EC_GROUP *group, struct sshbuf **shared_secretp)
{
    // 1. 从 blob 中反序列化对方的公钥
    EC_POINT *dh_pub = EC_POINT_new(group);
    sshbuf_get_ec(buf, dh_pub, group);  // 解析 0x04||x||y 格式

    // 2. 验证对方公钥合法性
    sshkey_ec_validate_public(group, dh_pub);
    // 验证：点不为 O、在曲线上、不在小子群中

    // 3. 确定共享秘密的字节长度
    size_t klen = (EC_GROUP_get_degree(group) + 7) / 8;
    // P-256: (256+7)/8 = 32 字节
    // P-384: (384+7)/8 = 48 字节
    // P-521: (521+7)/8 = 66 字节

    // 4. 调用 OpenSSL ECDH 计算共享秘密
    ECDH_compute_key(kbuf, klen, dh_pub, key, NULL);
    // 内部执行：
    //   P = d_C · Q_S  （客户端）或 P = d_S · Q_C  （服务端）
    //   取 P 的 x 坐标，填充到 kbuf[0..klen-1]

    // 5. 转为 BIGNUM 并以 mpint 格式存入 sshbuf
    BN_bin2bn(kbuf, klen, shared_secret);
    sshbuf_put_bignum2(buf, shared_secret);  // mpint 编码的 K
}
```

**完整调用时序图**：

```
客户端 (kex_ecdh_keypair)              服务端 (kex_ecdh_enc)
  │                                      │
  │ EC_KEY_new_by_curve_name(nid)        │
  │ EC_KEY_generate_key(key)             │
  │   → d_C (随机), Q_C = d_C·G         │
  │ sshbuf_put_ec(buf, Q_C, group)       │
  │   → 序列化公钥为 0x04||x||y         │
  │                                      │
  │──── {Q_C} ──────────────────────────>│
  │                                      │
  │                    EC_KEY_new_by_curve_name(nid)
  │                    EC_KEY_generate_key(key)
  │                      → d_S (随机), Q_S = d_S·G
  │                    sshbuf_put_ec(buf, Q_S, group)
  │                    ECDH_compute_key(kbuf, klen, Q_C, key, NULL)
  │                      → K = (d_S · Q_C).x
  │                    BN_bin2bn(kbuf, klen, shared_secret)
  │                    sshbuf_put_bignum2(buf, shared_secret)
  │                                      │
  │<──── {Q_S} ──────────────────────────│
  │                                      │
  │ sshbuf_get_ec(buf, dh_pub, group)    │
  │   → 解析 Q_S                         │
  │ sshkey_ec_validate_public(group, Q_S) │
  │ ECDH_compute_key(kbuf, klen, Q_S, key, NULL)
  │   → K = (d_C · Q_S).x              │
  │   = (d_C · d_S · G).x              │
  │   = (d_S · d_C · G).x              │
  │   = 服务端的 K ✓                     │
  │ BN_bin2bn(kbuf, klen, shared_secret) │
  │ sshbuf_put_bignum2(buf, shared_secret)│
```

**OpenSSL API 汇总**：

| API | 作用 | 调用时机 |
|-----|------|----------|
| `EC_KEY_new_by_curve_name(nid)` | 创建指定曲线的 EC_KEY 对象 | 生成密钥对前 |
| `EC_KEY_generate_key(key)` | 生成随机私钥 d，计算公钥 Q=d·G | 密钥对生成 |
| `EC_KEY_get0_public_key(key)` | 取出公钥 Q（EC_POINT*） | 序列化前 |
| `EC_KEY_get0_group(key)` | 取出曲线参数（EC_GROUP*） | 序列化/反序列化时 |
| `EC_POINT_new(group)` | 分配一个曲线点对象 | 反序列化对方公钥前 |
| `ECDH_compute_key(kbuf, klen, pub, key, NULL)` | 计算 P=d·Q，取 x 坐标存入 kbuf | 共享秘密计算 |
| `EC_KEY_free(key)` | 释放 EC_KEY（含私钥清零） | KEX 完成后 |

### 4.4 Curve25519（2 种）

**适用算法**：`curve25519-sha256`、`curve25519-sha256@libssh.org`

**源码文件**：`kexc25519.c`

**交互流程（2 条消息）**：

```
客户端                                          服务端
  │                                                │
  │ ① 随机生成 32 字节私钥 key_C                      │
  │    arc4random_buf(key, 32)                      │
  │ ② 计算公钥 pub_C = X25519(key_C, basepoint)     │
  │    （basepoint = {9, 0, 0, ...}）               │
  │                                                │
  │─── SSH2_MSG_KEX_ECDH_INIT {pub_C} ────────────>│
  │    （32 字节原始公钥）                             │
  │                                                │
  │                     ③ 随机生成 32 字节私钥 key_S    │
  │                     ④ 计算公钥 pub_S = X25519(key_S, basepoint)
  │                     ⑤ 计算 K = X25519(key_S, pub_C)
  │                     ⑥ 检查 K ≠ 全零               │
  │                     ⑦ 计算 H，签名                 │
  │                                                │
  │<── SSH2_MSG_KEX_ECDH_REPLY ────────────────────│
  │    {hostkey, pub_S, sig}                        │
  │                                                │
  │ ⑧ 计算 K = X25519(key_C, pub_S)                 │
  │    （= X25519(key_S, pub_C)，数学保证）           │
  │ ⑨ 验证签名，派生 6 组会话密钥（见 3.4 节）          │
```

**关键源码**（`kexc25519.c`）：

```c
// 生成密钥对
void kexc25519_keygen(u_char key[32], u_char pub[32]) {
    static const u_char basepoint[32] = {9};
    arc4random_buf(key, 32);                        // 随机私钥
    crypto_scalarmult_curve25519(pub, key, basepoint); // pub = key·G
}

// 计算共享秘密（双方调用同一函数）
crypto_scalarmult_curve25519(shared_key, key, pub); // K = key·对方pub
```

**与 ECDH 的区别**：

| | ECDH (NIST 曲线) | Curve25519 |
|---|-----------------|------------|
| 曲线类型 | Weierstrass 曲线 | Montgomery 曲线 |
| 公钥格式 | EC 点（压缩/非压缩编码） | 32 字节原始 x 坐标 |
| 共享秘密 | EC 点的 x 坐标 | 32 字节标量乘法结果 |
| 依赖 | OpenSSL EC 库 | 自研/独立实现 |
| 私钥处理 | 需要按曲线阶取模 | 低位清零 + 高位设置（clamping） |

### 4.5 SNTRUP761+X25519 混合 KEM（2 种）

**适用算法**：`sntrup761x25519-sha512`、`sntrup761x25519-sha512@openssh.com`

**源码文件**：`kexsntrup761x25519.c`

**交互流程（2 条消息，但不对称）**：

```
客户端                                          服务端
  │                                                │
  │ ① 生成 SNTRUP761 KEM 密钥对                      │
  │    crypto_kem_sntrup761_keypair(pq_pub, pq_priv) │
  │ ② 生成 X25519 密钥对 (c_key, c_pub)              │
  │    kexc25519_keygen(c_key, c_pub)               │
  │                                                │
  │─── SSH2_MSG_KEX_ECDH_INIT ────────────────────>│
  │    {pq_pub (1039字节) || c_pub (32字节)}          │
  │                                                │
  │                     ③ 随机生成 KEM 密钥 K_kem      │
  │                        crypto_kem_sntrup761_enc(  │
  │                          ciphertext, K_kem,       │
  │                          pq_pub)                  │
  │                        → 用客户端 pq_pub 加密 K_kem │
  │                     ④ 生成 X25519 密钥对 (s_key, s_pub)
  │                     ⑤ 计算 K_x = X25519(s_key, c_pub)
  │                     ⑥ buf = K_kem || K_x          │
  │                     ⑦ K = SHA-512(buf)            │
  │                     ⑧ 计算 H，签名                 │
  │                                                │
  │<── SSH2_MSG_KEX_ECDH_REPLY ────────────────────│
  │    {hostkey, ciphertext (1039B) || s_pub (32B), sig}
  │                                                │
  │ ⑨ 用 pq_priv 解密 ciphertext → 得到 K_kem         │
  │    crypto_kem_sntrup761_dec(K_kem, ciphertext,   │
  │                              pq_priv)             │
  │ ⑩ 计算 K_x = X25519(c_key, s_pub)               │
  │ ⑪ buf = K_kem || K_x                            │
  │ ⑫ K = SHA-512(buf)（与 ⑦ 相同）                  │
  │ ⑬ 验证签名，派生 6 组会话密钥（见 3.4 节）          │
```

**关键源码**（`kexsntrup761x25519.c`）：

```c
// 客户端：生成两组密钥对
kex_kem_sntrup761_keypair(struct kex *kex) {
    crypto_kem_sntrup761_keypair(cp, kex->sntrup761_client_key); // KEM
    kexc25519_keygen(kex->c25519_client_key, cp);                // X25519
}

// 服务端：封装 KEM 密钥 + 执行 X25519
kex_kem_sntrup761x25519_enc() {
    crypto_kem_sntrup761_enc(ciphertext, kem_key, client_pub); // 封装
    kexc25519_keygen(server_key, server_pub);                  // X25519
    kexc25519_shared_key_ext(server_key, client_pub, buf, 1);  // K_x
    ssh_digest_buffer(kex->hash_alg, buf, hash, ...);          // K = Hash(K_kem||K_x)
}

// 客户端：解封 KEM 密钥 + 执行 X25519
kex_kem_sntrup761x25519_dec() {
    crypto_kem_sntrup761_dec(kem_key, ciphertext, kex->sntrup761_client_key); // 解封
    kexc25519_shared_key_ext(kex->c25519_client_key, server_pub, buf, 1);     // K_x
    ssh_digest_buffer(kex->hash_alg, buf, hash, ...);                         // K = Hash(K_kem||K_x)
}
```

**消息大小**：

| 方向 | 内容 | 大小 |
|------|------|-----:|
| 客户端→服务端 | pq_pub + c_pub | 1039 + 32 = **1071 字节** |
| 服务端→客户端 | ciphertext + s_pub | 1039 + 32 = **1071 字节** |

### 4.6 ML-KEM768+X25519 混合 KEM（1 种）

**适用算法**：`mlkem768x25519-sha256`

**源码文件**：`kexmlkem768x25519.c`

**交互流程（2 条消息，与 SNTRUP761 模式相同）**：

```
客户端                                          服务端
  │                                                │
  │ ① 随机种子 → 生成 ML-KEM768 密钥对               │
  │    arc4random_buf(rnd, 64)                      │
  │    libcrux_ml_kem_mlkem768_portable_             │
  │      generate_key_pair(rnd)                      │
  │    → (mlkem_pub, mlkem_priv)                     │
  │ ② 生成 X25519 密钥对 (c_key, c_pub)              │
  │                                                │
  │─── SSH2_MSG_KEX_ECDH_INIT ────────────────────>│
  │    {mlkem_pub (1184字节) || c_pub (32字节)}       │
  │                                                │
  │                     ③ 验证 mlkem_pub 合法性        │
  │                     ④ 封装：(ciphertext, K_kem) = │
  │                        Encapsulate(mlkem_pub, rnd) │
  │                     ⑤ 生成 X25519 密钥对 (s_key, s_pub)
  │                     ⑥ 计算 K_x = X25519(s_key, c_pub)
  │                     ⑦ buf = K_kem || K_x          │
  │                     ⑧ K = SHA-256(buf)            │
  │                     ⑨ 计算 H，签名                 │
  │                                                │
  │<── SSH2_MSG_KEX_ECDH_REPLY ────────────────────│
  │    {hostkey, ciphertext (1088B) || s_pub (32B), sig}
  │                                                │
  │ ⑩ 解封：K_kem = Decapsulate(mlkem_priv, ciphertext)
  │ ⑪ 计算 K_x = X25519(c_key, s_pub)               │
  │ ⑫ buf = K_kem || K_x                            │
  │ ⑬ K = SHA-256(buf)（与 ⑧ 相同）                  │
  │ ⑭ 验证签名，派生 6 组会话密钥（见 3.4 节）          │
```

**与 SNTRUP761 版本的对比**：

| 对比项 | SNTRUP761+X25519 | ML-KEM768+X25519 |
|--------|-----------------|------------------|
| 后量子算法 | SNTRUP761（NTRU Prime） | ML-KEM768（CRYSTALS-Kyber） |
| KEM 公钥大小 | 1039 字节 | 1184 字节 |
| KEM 密文大小 | 1039 字节 | 1088 字节 |
| KEM 共享密钥大小 | 32 字节 | 32 字节 |
| 最终哈希 | SHA-512 | SHA-256 |
| 客户端消息总大小 | 1071 字节 | 1216 字节 |
| 服务端消息总大小 | 1071 字节 | 1120 字节 |
| 密钥生成方式 | 内部随机 | 外部提供 64 字节随机种子 |
| 公钥验证 | 无 | `validate_public_key()` |
| 实现来源 | OpenSSH 自带 C 代码 | libcrux 库（Rust 编译产物） |

**关键源码**（`kexmlkem768x25519.c`）：

```c
// 客户端：随机种子生成 ML-KEM 密钥对
arc4random_buf(rnd, sizeof(rnd));  // 64 字节随机种子
keypair = libcrux_ml_kem_mlkem768_portable_generate_key_pair(rnd);

// 服务端：封装
enc = libcrux_ml_kem_mlkem768_portable_encapsulate(&mlkem_pub, rnd);
// enc.fst = ciphertext（密文），enc.snd = K_kem（共享密钥）

// 客户端：解封
libcrux_ml_kem_mlkem768_portable_decapsulate(&mlkem_priv,
    &mlkem_ciphertext, mlkem_key);  // 从密文恢复 K_kem
```

### 4.7 交互模式总结对比

| 特征 | DH 固定群 | DH GEX | ECDH | Curve25519 | PQ KEM 混合 |
|------|----------|--------|------|------------|------------|
| 消息轮数 | 2 | **3** | 2 | 2 | 2 |
| 客户端→服务端 | e | (min,nbits,max) → e | Q_C | pub_C | pq_pub + c_pub |
| 服务端→客户端 | hostkey,f,sig | p,g → hostkey,f,sig | hostkey,Q_S,sig | hostkey,pub_S,sig | hostkey,C+s_pub,sig |
| 共享秘密计算 | 双方对称 | 双方对称 | 双方对称 | 双方对称 | **不对称**（封装/解封） |
| 临时私钥 x | 大整数 (CSPRNG) | 大整数 (CSPRNG) | EC 标量 (OpenSSL) | 32字节 (arc4random) | KEM内部 + 32字节 |
| 最终共享秘密 | K 原值 | K 原值 | K 原值 | K 原值 | **Hash(K_kem \|\| K_x)** |

### 4.8 交换哈希 H 的计算

所有 15 种 KEX 算法的签名与验证流程相同（服务端对 H 签名，客户端验证），但 **H 的内容**因算法类型而异。OpenSSH 中有两个哈希函数：

| 哈希函数 | 适用算法 | 源码文件 |
|---------|---------|----------|
| `kex_gen_hash()` | DH 固定群、DH GEX 以外的所有 12 种 | `kexgen.c` |
| `kexgex_hash()` | 仅 DH Group Exchange 的 2 种 | `kexgex.c` |

#### 4.8.1 DH 固定群 / ECDH / Curve25519 / 后量子 KEM（12 种）

这 12 种算法共用 `kex_gen_hash()` 函数，H 的通用结构为：

```
H = hash_alg(
    string  V_C          ← 客户端版本字符串（如 "SSH-2.0-OpenSSH_9.9"）
    string  V_S          ← 服务端版本字符串
    string  I_C          ← 客户端 KEX_INIT 消息（含 length + SSH2_MSG_KEXINIT 头）
    string  I_S          ← 服务端 KEX_INIT 消息（同上）
    string  K_S          ← 服务端主机公钥的完整序列化（host_key_blob）
    string  e            ← 客户端的 KEX 公开值
    string  f            ← 服务端的 KEX 公开值
    string  K            ← 共享秘密（编码方式因算法而异）
)
```

**各算法中 e、f、K 的具体含义**：

| 算法类型 | 算法 | e（客户端公开值） | f（服务端公开值） | K（共享秘密）编码方式 |
|---------|------|:---:|:---:|:---:|
| DH 固定群 | group1-sha1 | `e = g^x mod p`（mpint） | `f = g^y mod p`（mpint） | mpint |
| DH 固定群 | group14-sha1 | 同上 | 同上 | mpint |
| DH 固定群 | group14-sha256 | 同上 | 同上 | mpint |
| DH 固定群 | group16-sha512 | 同上 | 同上 | mpint |
| DH 固定群 | group18-sha512 | 同上 | 同上 | mpint |
| ECDH | nistp256 | Q_C（EC 点） | Q_S（EC 点） | mpint（EC 共享点 x 坐标） |
| ECDH | nistp384 | 同上 | 同上 | mpint |
| ECDH | nistp521 | 同上 | 同上 | mpint |
| Curve25519 | sha256 | pub_C（32 字节） | pub_S（32 字节） | mpint（32 字节标量乘结果） |
| Curve25519 | sha256@libssh.org | 同上 | 同上 | mpint |
| PQ KEM | sntrup761x25519 | pq_pub ‖ c_pub | ciphertext ‖ s_pub | **string**（SHA-512 哈希结果） |
| PQ KEM | mlkem768x25519 | mlkem_pub ‖ c_pub | ciphertext ‖ s_pub | **string**（SHA-256 哈希结果） |

> **注意 K 的编码差异**：DH / ECDH / Curve25519 的共享秘密 K 以 **mpint** 格式放入 H（`sshbuf_put_bignum2`），而 PQ KEM 混合算法的 K 以 **string** 格式放入 H（`sshbuf_put_string`）。这是因为 PQ KEM 的 K 本身就是哈希输出（固定长度的字节串），而非大整数运算的结果。

**源码关键片段**（`kexgen.c`）：

```c
static int
kex_gen_hash(
    int hash_alg,
    const struct sshbuf *client_version,
    const struct sshbuf *server_version,
    const struct sshbuf *client_kexinit,
    const struct sshbuf *server_kexinit,
    const struct sshbuf *server_host_key_blob,
    const struct sshbuf *client_pub,    // ← e
    const struct sshbuf *server_pub,    // ← f
    const struct sshbuf *shared_secret, // ← K（已编码）
    u_char *hash, size_t *hashlen)
{
    // 按顺序拼接所有字段
    sshbuf_put_stringb(b, client_version);
    sshbuf_put_stringb(b, server_version);
    sshbuf_put_u32(b, len(client_kexinit) + 1);   // KEX_INIT 长度头
    sshbuf_put_u8(b, SSH2_MSG_KEXINIT);
    sshbuf_putb(b, client_kexinit);
    sshbuf_put_u32(b, len(server_kexinit) + 1);
    sshbuf_put_u8(b, SSH2_MSG_KEXINIT);
    sshbuf_putb(b, server_kexinit);
    sshbuf_put_stringb(b, server_host_key_blob);
    sshbuf_put_stringb(b, client_pub);             // e
    sshbuf_put_stringb(b, server_pub);             // f
    sshbuf_putb(b, shared_secret);                 // K（已含编码前缀）

    // 取哈希
    ssh_digest_buffer(hash_alg, b, hash, *hashlen);
}
```

#### 4.8.2 DH Group Exchange（2 种）

`kexgex_hash()` 在通用结构基础上**额外包含 5 个字段**：

```
H = hash_alg(
    string  V_C          ← 客户端版本字符串
    string  V_S          ← 服务端版本字符串
    string  I_C          ← 客户端 KEX_INIT 消息
    string  I_S          ← 服务端 KEX_INIT 消息
    string  K_S          ← 服务端主机公钥
    uint32  min          ← 客户端请求的最小群大小（比特）
    uint32  nbits        ← 客户端请求的目标群大小（比特）
    uint32  max          ← 客户端请求的最大群大小（比特）
    mpint   p            ← 服务端选择的素数
    mpint   g            ← 服务端选择的生成元
    mpint   e            ← 客户端 DH 公钥
    mpint   f            ← 服务端 DH 公钥
    mpint   K            ← 共享秘密
)
```

**与通用结构的对比**：

```
通用（12种算法）:  V_C | V_S | I_C | I_S | K_S | e | f | K
GEX（2种算法）:   V_C | V_S | I_C | I_S | K_S | min | nbits | max | p | g | e | f | K
                                               ↑────── 额外 5 个字段 ──────↑
```

**为什么 GEX 多了 5 个字段？**

GEX 的群参数 `(p, g)` 是**动态协商**的（客户端发 `min/nbits/max`，服务端从 `/etc/ssh/moduli` 中选择），而非像固定群那样预定义。如果 H 不包含这些参数，中间人可以：

1. 篡改 `min/nbits/max` → 迫使服务端选择弱素数（如 1024-bit）
2. 替换 `(p, g)` → 使用自己控制的弱群

把 `min, nbits, max, p, g` 全部绑定进 H 后，任何篡改都会导致双方计算出的 H 不同，签名验证失败。

**源码关键片段**（`kexgex.c`）：

```c
static int
kexgex_hash(
    int hash_alg,
    const struct sshbuf *client_version,
    const struct sshbuf *server_version,
    const struct sshbuf *client_kexinit,
    const struct sshbuf *server_kexinit,
    const struct sshbuf *server_host_key_blob,
    int min, int wantbits, int max,          // ← 额外的群大小参数
    const BIGNUM *prime,                      // ← 额外的素数 p
    const BIGNUM *gen,                        // ← 额外的生成元 g
    const BIGNUM *client_dh_pub,
    const BIGNUM *server_dh_pub,
    const u_char *shared_secret, size_t secretlen,
    u_char *hash, size_t *hashlen)
{
    sshbuf_put_stringb(b, client_version);
    sshbuf_put_stringb(b, server_version);
    // ... I_C, I_S, K_S 同上 ...
    sshbuf_put_stringb(b, server_host_key_blob);
    sshbuf_put_u32(b, min);                   // ← 额外字段
    sshbuf_put_u32(b, wantbits);              // ← 额外字段
    sshbuf_put_u32(b, max);                   // ← 额外字段
    sshbuf_put_bignum2(b, prime);             // ← 额外字段
    sshbuf_put_bignum2(b, gen);               // ← 额外字段
    sshbuf_put_bignum2(b, client_dh_pub);     // e
    sshbuf_put_bignum2(b, server_dh_pub);     // f
    sshbuf_put(b, shared_secret, secretlen);  // K

    ssh_digest_buffer(hash_alg, b, hash, *hashlen);
}
```

#### 4.8.3 每种 KEX 算法使用的哈希函数

每个 KEX 算法名隐含绑定了哈希算法，在 `kex-names.c` 的 `kexalgs[]` 注册表中定义。**同一哈希函数同时用于 H 的计算和密钥派生**。

| KEX 算法名 | 哈希函数 | H 输出大小 | 哈希函数源码标识 |
|---------|---------|:--------:|:----------:|
| `diffie-hellman-group1-sha1` | **SHA-1** | 20 字节 | `SSH_DIGEST_SHA1` |
| `diffie-hellman-group14-sha1` | **SHA-1** | 20 字节 | `SSH_DIGEST_SHA1` |
| `diffie-hellman-group14-sha256` | **SHA-256** | 32 字节 | `SSH_DIGEST_SHA256` |
| `diffie-hellman-group16-sha512` | **SHA-512** | 64 字节 | `SSH_DIGEST_SHA512` |
| `diffie-hellman-group18-sha512` | **SHA-512** | 64 字节 | `SSH_DIGEST_SHA512` |
| `diffie-hellman-group-exchange-sha1` | **SHA-1** | 20 字节 | `SSH_DIGEST_SHA1` |
| `diffie-hellman-group-exchange-sha256` | **SHA-256** | 32 字节 | `SSH_DIGEST_SHA256` |
| `ecdh-sha2-nistp256` | **SHA-256** | 32 字节 | `SSH_DIGEST_SHA256` |
| `ecdh-sha2-nistp384` | **SHA-384** | 48 字节 | `SSH_DIGEST_SHA384` |
| `ecdh-sha2-nistp521` | **SHA-512** | 64 字节 | `SSH_DIGEST_SHA512` |
| `curve25519-sha256` | **SHA-256** | 32 字节 | `SSH_DIGEST_SHA256` |
| `curve25519-sha256@libssh.org` | **SHA-256** | 32 字节 | `SSH_DIGEST_SHA256` |
| `sntrup761x25519-sha512` | **SHA-512** | 64 字节 | `SSH_DIGEST_SHA512` |
| `sntrup761x25519-sha512@openssh.com` | **SHA-512** | 64 字节 | `SSH_DIGEST_SHA512` |
| `mlkem768x25519-sha256` | **SHA-256** | 32 字节 | `SSH_DIGEST_SHA256` |

> **算法名中的后缀就是哈希函数**：算法名末尾的 `-sha1`、`-sha256`、`-sha512` 直接表明哈希函数。唯一的例外是 ECDH 系列（`ecdh-sha2-nistp*`），其哈希函数由曲线大小隐含决定（nistp256→SHA-256，nistp384→SHA-384，nistp521→SHA-512）。
>
> **H 输出大小的影响**：H 不仅决定签名输入的大小，还决定密钥派生时每次调用 `Hash()` 输出的字节数。例如 `diffie-hellman-group1-sha1` 每次只能派生 20 字节密钥，而 `diffie-hellman-group16-sha512` 每次可派生 64 字节。

#### 4.8.4 H 的作用

计算出的 H 有两个用途：

1. **签名/验证**：服务端用主机私钥对 H 签名（`kex->sign`），客户端用主机公钥验证（`sshkey_verify`） → 防止中间人攻击
2. **密钥派生**：H 和 K 一起参与 `derive_key()` 函数，通过 `Hash(K || H || "A"~"F" || session_id)` 派生出 6 组会话密钥，详见 [3.4 派生会话密钥](#34-派生会话密钥)

签名算法（`kex->hostkey_alg`）由 KEX_INIT 协商确定，与 KEX 算法本身无关。例如 `curve25519-sha256` 可以用 Ed25519、RSA 或 ECDSA 主机密钥，只要双方协商支持即可。

---

## 5. 算法分类详解

### 5.1 按安全性分级

| 推荐级别 | 算法 | 说明 |
|---------|------|------|
| **推荐** | `mlkem768x25519-sha256` | 后量子安全，面向未来 |
| **推荐** | `sntrup761x25519-sha512` | 后量子安全，已广泛部署 |
| **推荐** | `curve25519-sha256` | 当前最佳通用选择 |
| 可接受 | `ecdh-sha2-nistp256/384/521` | NIST 曲线，广泛兼容 |
| 可接受 | `diffie-hellman-group16-sha512` | 4096-bit DH，安全性高 |
| 可接受 | `diffie-hellman-group18-sha512` | 8192-bit DH，安全性极高但慢 |
| 可接受 | `diffie-hellman-group14-sha256` | 2048-bit DH，最低可接受 |
| **不推荐** | `diffie-hellman-group-exchange-sha1` | SHA-1 已不安全 |
| **不推荐** | `diffie-hellman-group14-sha1` | SHA-1 已不安全 |
| **不推荐** | `diffie-hellman-group1-sha1` | 1024-bit + SHA-1，严重不安全 |

### 5.2 按 OpenSSL 依赖

| 无需 OpenSSL | 需要 OpenSSL | 需要 OpenSSL + ECC |
|-------------|-------------|-------------------|
| `curve25519-sha256` | `diffie-hellman-group*-sha*` | `ecdh-sha2-nistp*` |
| `curve25519-sha256@libssh.org` | `diffie-hellman-group-exchange-*` | |
| `sntrup761x25519-sha512` | | |
| `sntrup761x25519-sha512@openssh.com` | | |
| `mlkem768x25519-sha256` | | |

---

## 6. 相关源码文件

| 文件 | 说明 |
|------|------|
| `kex.h` | KEX 算法名称宏定义、类型枚举 |
| `kex-names.c` | 算法注册表 `kexalgs[]`、名称查找与验证 |
| `myproposal.h` | 默认算法列表（服务端/客户端） |
| `kex.c` | KEX 核心逻辑：协商、密钥派生 |
| `kexdh.c` | DH 固定群密钥交换实现 |
| `kexgex.c` / `kexgexc.c` / `kexgexs.c` | DH Group Exchange 实现 |
| `kexecdh.c` | ECDH 密钥交换实现 |
| `kexc25519.c` | Curve25519 密钥交换实现 |
| `kexsntrup761x25519.c` | SNTRUP761+X25519 混合 KEX 实现 |
| `kexmlkem768x25519.c` | ML-KEM768+X25519 混合 KEX 实现 |
| `kexgen.c` | 通用 KEX 框架（消息收发、状态机） |
