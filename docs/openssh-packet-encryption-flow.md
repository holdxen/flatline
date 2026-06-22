# OpenSSH 数据包加密流程

> 基于 OpenSSH Portable 源码（`packet.c` / `cipher.c` / `cipher-chachapoly.c` / `mac.c`）分析 SSH2 协议中对称加密、MAC、压缩三种算法如何协同工作来保护一个数据包。

---

## 目录

1. [SSH2 数据包结构](#1-ssh2-数据包结构)
2. [发送端完整流程](#2-发送端完整流程)
3. [接收端完整流程](#3-接收端完整流程)
4. [压缩：压缩了什么](#4-压缩压缩了什么)
5. [MAC：计算了什么](#5-mac计算了什么)
6. [cipher_crypt 内部：非 AEAD 与 AEAD 的路径差异](#6-cipher_crypt-内部非-aead-与-aead-的路径差异)
7. [ChaCha20-Poly1305 完整加密过程](#7-chacha20-poly1305-完整加密过程)
8. [AES-GCM 完整加密过程](#8-aes-gcm-完整加密过程)
9. [Padding 计算详解](#9-padding-计算详解)
10. [cipher_crypt 参数详解](#10-cipher_crypt-参数详解)
11. [四种场景逐步走查](#11-四种场景逐步走查)
12. [压缩的启用时机](#12-压缩的启用时机)
13. [序列号的作用](#13-序列号的作用)
14. [完整数据流图](#14-完整数据流图)

---

## 1. SSH2 数据包结构

每个 SSH2 数据包在**加密前**的逻辑结构如下：

```
+--------------------+------------------+
| uint32 packet_len  |  4 字节          |  不包含自身长度
| byte   pad_len     |  1 字节          |  padding 的字节数
| byte[] payload     |  N 字节          |  type(1B) + 实际数据
| byte[] padding     |  pad_len 字节    |  随机填充
+--------------------+------------------+
```

- `packet_len` = 1（pad_len）+ N（payload）+ pad_len（padding）
- padding 最小 4 字节，且 `(packet_len + 4)` 必须是 `block_size` 的整数倍

---

## 2. 发送端完整流程

核心函数：`ssh_packet_send2_wrapped()`（[packet.c:1225](../packet.c)）

### 步骤总览

```
原始 payload
    │
    ▼
① 压缩（如果启用）
    │
    ▼
② 计算 padding + 填充随机数据
    │
    ▼
③ 填入 packet_length 和 pad_length 字段
    │
    ▼
④ 计算 MAC（仅 Encrypt-and-MAC 模式，在加密前计算）
    │
    ▼
⑤ 加密（cipher_crypt）
    │
    ▼
⑥ 计算 MAC（仅 Encrypt-then-MAC 模式，在加密后计算）
    │
    ▼
⑦ 追加 MAC 到输出缓冲区
    │
    ▼
⑧ 序列号 +1
```

### ① 压缩

```c
if (comp && comp->enabled) {
    sshbuf_consume(state->outgoing_packet, 5);       // 跳过前 5 字节头部
    compress_buffer(ssh, state->outgoing_packet,     // 压缩 type + data
        state->compression_buffer);
    sshbuf_put(state->outgoing_packet, "\0\0\0\0\0", 5); // 恢复空头部
    sshbuf_putb(state->outgoing_packet, state->compression_buffer);
}
```

> 详见 [第 4 节](#4-压缩压缩了什么)。

### ② 计算 Padding

```c
len = sshbuf_len(state->outgoing_packet);   // = 4 + 1 + payload_len
len -= aadlen;   // EtM/AEAD 模式下 packet_length(4字节) 不参与对齐

padlen = block_size - (len % block_size);
if (padlen < 4)
    padlen += block_size;    // 最小 4 字节 padding
```

填充随机数据：

```c
if (enc && !cipher_ctx_is_plaintext(state->send_context)) {
    arc4random_buf(cp, padlen);     // 加密模式：随机填充
} else {
    explicit_bzero(cp, padlen);     // 明文模式：零填充
}
```

### ③ 填入头部字段

```c
POKE_U32(cp, len - 4);   // packet_length（不含自身的 4 字节）
cp[4] = padlen;           // pad_length
```

此时 outgoing_packet 的完整内容为：

```
[4B packet_length] [1B pad_len] [payload...] [padding...]
```

### ④ MAC 计算 —— Encrypt-and-MAC 模式

```c
if (mac && mac->enabled && !mac->etm) {
    mac_compute(mac, state->p_send.seqnr,
        sshbuf_ptr(state->outgoing_packet), len,
        macbuf, sizeof(macbuf));
}
```

> 详见 [第 5 节](#5-mac计算了什么)。

### ⑤ 加密

```c
cipher_crypt(state->send_context, state->p_send.seqnr, cp,
    sshbuf_ptr(state->outgoing_packet),
    len - aadlen,    // 需要加密的长度
    aadlen,          // AAD 长度
    authlen);        // 认证标签长度（AEAD）
```

> 详见 [第 6 节](#6-cipher_crypt-内部非-aead-与-aead-的路径差异)，非 AEAD 和 AEAD 走完全不同的代码路径。

### ⑥ MAC 计算 —— Encrypt-then-MAC 模式

```c
if (mac && mac->enabled && mac->etm) {
    mac_compute(mac, state->p_send.seqnr,
        cp, len,     // cp 指向加密后的输出缓冲区
        macbuf, sizeof(macbuf));
}
```

### ⑦ 追加 MAC

```c
if (mac && mac->enabled) {
    sshbuf_put(state->output, macbuf, mac->mac_len);
}
```

### ⑧ 序列号递增

```c
if (++state->p_send.seqnr == 0) {
    // 序列号回绕检查
}
```

---

## 3. 接收端完整流程

核心函数：`ssh_packet_read_poll2()`（[packet.c:1624](../packet.c)）

```
网络数据到达
    │
    ▼
① 获取数据包长度
    │  ├─ ChaCha20-Poly1305：header_ctx 解密前 4 字节
    │  ├─ AES-GCM / EtM：packet_length 是明文，直接读取
    │  └─ 非 AEAD（CTR/CBC）：解密第一个 block，读取 packet_length
    │
    ▼
② 等待接收完整数据包
    │
    ▼
③ 验证 MAC（EtM 模式，在解密前验证）
    │
    ▼
④ 解密（cipher_crypt）
    │
    ▼
⑤ 验证 MAC（Encrypt-and-MAC 模式，在解密后验证）
    │
    ▼
⑥ 去除 padding
    │
    ▼
⑦ 解压缩（如果启用）
    │
    ▼
⑧ 提取 packet type，返回 payload
```

### ① 获取数据包长度

**ChaCha20-Poly1305**（[cipher-chachapoly.c:122](../cipher-chachapoly.c)）：
```c
// 用 header_ctx（独立密钥）解密前 4 字节
chacha_ivsetup(&ctx->header_ctx, seqbuf, NULL);
chacha_encrypt_bytes(&ctx->header_ctx, cp, buf, 4);
*plenp = PEEK_U32(buf);
```

**AES-GCM / EtM**（[cipher.c:412-414](../cipher.c)）：
```c
// packet_length 是明文（AAD 或 aadlen=4 明文传输），直接读
*plenp = PEEK_U32(cp);
```

**非 AEAD（CTR/CBC）**：
```c
// 必须先解密第一个 block 才能读到 packet_length
cipher_crypt(state->receive_context, ..., block_size, 0, 0);
state->packlen = PEEK_U32(sshbuf_ptr(state->incoming_packet));
```

### ③ EtM MAC 验证（解密前）

```c
if (mac && mac->enabled && mac->etm) {
    mac_check(mac, state->p_read.seqnr,
        sshbuf_ptr(state->input), aadlen + need,    // 密文数据
        sshbuf_ptr(state->input) + aadlen + need + authlen, maclen);
}
```

### ⑤ 非 EtM MAC 验证（解密后）

```c
if (!mac->etm) {
    mac_check(mac, state->p_read.seqnr,
        sshbuf_ptr(state->incoming_packet), ...,    // 明文数据
        sshbuf_ptr(state->input), maclen);
}
```

### ⑦ 解压缩

```c
if (comp && comp->enabled) {
    uncompress_buffer(ssh, state->incoming_packet, state->compression_buffer);
}
```

---

## 4. 压缩：压缩了什么

### outgoing_packet 的初始构造

`sshpkt_start()` 写入 6 字节占位（[packet.c:2853](../packet.c)）：

```c
u_char buf[6]; /* u32 packet length, u8 pad len, u8 type */
buf[sizeof(buf) - 1] = type;
sshbuf_put(ssh->state->outgoing_packet, buf, sizeof(buf));
```

之后 `sshpkt_put*()` 追加应用数据。所以进入 `ssh_packet_send2_wrapped()` 时：

```
outgoing_packet:
┌──────────────┬──────────┬────────────────────────┐
│ packet_len   │ pad_len  │ type(1B) │ data...     │
│   4字节占位   │ 1字节占位 │            │ N字节      │
└──────────────┴──────────┴────────────────────────┘
```

### 压缩操作（[packet.c:1256-1273](../packet.c)）

```c
// 1. 跳过前 5 字节（packet_len占位 + pad_len占位）
sshbuf_consume(state->outgoing_packet, 5);

// 2. 压缩剩下的内容 = [type] + [data...]
compress_buffer(ssh, state->outgoing_packet, state->compression_buffer);

// 3. 重新拼回：5字节空头部 + 压缩后的 payload
sshbuf_put(state->outgoing_packet, "\0\0\0\0\0", 5);
sshbuf_putb(state->outgoing_packet, state->compression_buffer);
```

### 结论

**压缩的对象是 `[type(1字节)] + [应用数据(N字节)]`**，即消息类型字节和后面的全部载荷数据。`packet_length`(4字节) 和 `padding_length`(1字节) 这两个协议头部字段不参与压缩。

```
outgoing_packet:
┌──────────────┬──────────┬────────────────────────┐
│ packet_len   │ pad_len  │ type │ data...          │
│   4字节      │ 1字节    │ 1B   │ N B              │
└──────────────┴──────────┴────────────────────────┘
                              ^^^^^^^^^^^^^^^^^^^^^^
                                ↑ 只压缩这部分
```

---

## 5. MAC：计算了什么

### MAC 底层实现（[mac.c:170-177](../mac.c)）

```c
// HMAC 的实际输入是两部分拼接：
ssh_hmac_update(mac->hmac_ctx, b, sizeof(b));    // b = seqnr (4字节)
ssh_hmac_update(mac->hmac_ctx, data, datalen);   // data = 调用方传入的数据
```

**MAC = HMAC(seqnr(4字节) || data)**，区别在于 `data` 是什么。

### 情况 1：Encrypt-and-MAC（如 aes256-ctr + hmac-sha2-256）

[packet.c:1329-1336](../packet.c)：

```c
/* compute MAC over seqnr and packet(length fields, payload, padding) */
if (mac && mac->enabled && !mac->etm) {
    mac_compute(mac, state->p_send.seqnr,
        sshbuf_ptr(state->outgoing_packet), len,   // ← 整个明文包
        macbuf, sizeof(macbuf));
}
```

此时 `outgoing_packet` 已填好 packet_len、pad_len、padding，是**完整明文包**：

```
data = 整个明文 outgoing_packet:
┌──────────────┬──────────┬─────────────────────┬──────────┐
│ packet_len   │ pad_len  │ type │ payload       │ padding  │
│   4字节      │ 1字节    │                      │ 随机填充  │
└──────────────┴──────────┴─────────────────────┴──────────┘
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                   MAC 覆盖全部明文
```

MAC 在加密**之前**计算，然后**再加密**整个包。线上格式：`[全部密文] [MAC]`

### 情况 2：Encrypt-then-MAC（如 aes256-ctr + hmac-sha2-256-etm）

[packet.c:1346-1354](../packet.c)：

```c
if (mac->etm) {
    mac_compute(mac, state->p_send.seqnr,
        cp, len, macbuf, sizeof(macbuf));   // ← cp 是加密后的输出
}
```

`cp` 指向 `state->output` 缓冲区中**加密后的数据**。由于 EtM 模式下 `aadlen=4`，packet_length 不加密：

```
cp = 加密后的输出（len 字节）:
┌──────────────┬──────────────────────────────────────────┐
│ packet_len   │  encrypted(pad_len + payload + padding)  │
│   4字节明文   │              密文                         │
└──────────────┴──────────────────────────────────────────┘
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
          MAC 覆盖：明文 packet_len + 密文
```

**先加密，再对密文算 MAC。** 线上格式：`[packet_len明文] [密文] [MAC]`

### 情况 3：AEAD（chacha20-poly1305 / aes*-gcm）

[packet.c:1242-1243](../packet.c)：

```c
if ((authlen = cipher_authlen(enc->cipher)) != 0)
    mac = NULL;   // 直接禁用独立 MAC
```

AEAD **没有独立的 MAC 计算步骤**。加密和认证在 `cipher_crypt()` 内部一步完成。详见 [第 7 节](#7-chacha20-poly1305-完整加密过程) 和 [第 8 节](#8-aes-gcm-完整加密过程)。

### 总结

| 模式 | MAC 输入 data | MAC 覆盖内容 |
|------|-------------|-------------|
| Encrypt-and-MAC | 完整明文包 | packet_len + pad_len + type + payload + padding |
| Encrypt-then-MAC | 明文 packet_len + 密文 | packet_len(明文) + encrypted(pad_len + payload + padding) |
| AEAD | 无独立 MAC | 认证标签内置于加密过程 |

---

## 6. cipher_crypt 内部：非 AEAD 与 AEAD 的路径差异

`cipher_crypt()`（[cipher.c:342](../cipher.c)）是加密的统一入口，但非 AEAD 和 AEAD 走完全不同的代码路径。

### 调用参数差异

`ssh_packet_send2_wrapped()` 调用 `cipher_crypt()` 时（[packet.c:1341](../packet.c)）：

```c
cipher_crypt(state->send_context, state->p_send.seqnr, cp,
    sshbuf_ptr(state->outgoing_packet),
    len - aadlen,    // 第5个参数：加密长度
    aadlen,          // 第6个参数：AAD 长度
    authlen);        // 第7个参数：认证标签长度
```

三种算法传入的值：

| | `len`（加密长度） | `aadlen` | `authlen` |
|---|---|:---:|:---:|
| **非 AEAD**（如 aes256-ctr） | **整个包的长度** | **0** | **0** |
| **AES-GCM**（AEAD） | 包长 - 4 | 4 | 16 |
| **ChaCha20-Poly1305**（AEAD） | 包长 - 4 | 4 | 16 |

### 非 AEAD 路径（aes256-ctr / aes128-cbc 等）

`authlen=0`，`aadlen=0`，代码极其简单（[cipher.c:364-401](../cipher.c)）：

```c
// authlen == 0 → 跳过 IV/Tag 准备
// aadlen == 0  → 跳过 AAD 处理

// 唯一的操作：加密全部内容
EVP_Cipher(cc->evp, dest + 0, (u_char *)src + 0, len);
//                    ^^^^^                 ^^^^^  ^^^
//                    dest从0开始           src从0   len=整个包

// authlen == 0 → 跳过 Tag 生成
```

**就一步：把整个包全部加密。packet_length、pad_len、payload、padding 统统进入加密器。**

```
outgoing_packet:
┌──────────────┬──────────┬─────────────────────┬──────────┐
│ packet_len   │ pad_len  │ type │ payload       │ padding  │
│   4B         │ 1B       │      N B             │ M B      │
└──────────────┴──────────┴─────────────────────┴──────────┘
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                    全部加密
                    EVP_Cipher(dest, src, len)
                    一个调用搞定

输出: [================== 全部密文 ==================]
```

### AES-GCM 路径（AEAD）

`authlen=16`，`aadlen=4`，代码分四个阶段（[cipher.c:364-401](../cipher.c)）：

```c
// 阶段 1: IV 自增
EVP_CIPHER_CTX_ctrl(cc->evp, EVP_CTRL_GCM_IV_GEN, 1, lastiv);

// 阶段 2: 处理 AAD（认证但不加密）
EVP_Cipher(cc->evp, NULL, (u_char *)src, 4);   // dest=NULL → 不输出密文
memcpy(dest, src, 4);                            // 原样拷贝 packet_length

// 阶段 3: 加密 payload
EVP_Cipher(cc->evp, dest + 4, (u_char *)src + 4, len);

// 阶段 4: 生成认证标签
EVP_Cipher(cc->evp, NULL, NULL, 0);              // finalize
EVP_CIPHER_CTX_ctrl(cc->evp, EVP_CTRL_GCM_GET_TAG, 16, dest + 4 + len);
```

```
outgoing_packet:
┌──────────────┬──────────────────────────────────────────────┐
│ packet_len   │ pad_len │ type │ payload │ padding           │
│   4B         │ 1B      │      N B      │ M B               │
└──────────────┴──────────────────────────────────────────────┘
     │                     │
     │ 阶段2                │ 阶段3
     ▼                     ▼
EVP_Cipher(NULL,src,4)  EVP_Cipher(dest+4, src+4, len)
  只认证，不加密            加密
  原样拷贝到输出            输出密文

                    阶段4: GCM Tag
                    EVP_Cipher(NULL,NULL,0) → finalize
                    GET_TAG → 16字节 tag

输出: [pkt_len明文] [========= 密文 =========] [tag(16B)]
        4B             len 字节                  16字节
```

### 核心差异总结

| | 非 AEAD | AES-GCM | ChaCha20-Poly1305 |
|---|---|---|---|
| **packet_length 线上状态** | 密文（被加密） | 明文（AAD，只认证） | 密文（header_ctx 加密） |
| **加密调用次数** | 1 次 | 2 次（AAD + payload） | 3 次（poly_key + header + payload） |
| **认证机制** | 无（需外部 MAC） | GCM tag（内置） | Poly1305 tag（内置） |
| **IV/nonce 来源** | KEX 派生（固定） | KEX 派生 + 隐式自增 | seqnr（每包变化） |
| **密钥流使用** | 1 段连续密钥流 | GCM 标准 | counter=0 生成 poly_key，counter=1 加密数据 |

---

## 7. ChaCha20-Poly1305 完整加密过程

源码：[cipher-chachapoly.c:68-120](../cipher-chachapoly.c)（内置实现）/ [cipher-chachapoly-libcrypto.c:82-143](../cipher-chachapoly-libcrypto.c)（OpenSSL 实现）

### 密钥结构：一个 64 字节密钥拆成两把

```c
// chachapoly_new():
chacha_keysetup(&ctx->main_ctx, key, 256);        // key[0..31]  → main_ctx
chacha_keysetup(&ctx->header_ctx, key + 32, 256);  // key[32..63] → header_ctx
```

### 加密 5 步

```c
// ---- 步骤 1: 用 main_ctx + counter=0 生成 Poly1305 一次性密钥 ----
POKE_U64(seqbuf, seqnr);                         // nonce = seqnr
chacha_ivsetup(&ctx->main_ctx, seqbuf, NULL);     // counter = 0
chacha_encrypt_bytes(&ctx->main_ctx, poly_key, poly_key, 32);
//                                          ^^^^^^^^^^^^^^^^^^^
//                                          加密 32 字节全零 → 得到 poly_key

// ---- 步骤 2: 解密时先验证 tag（authenticate-then-decrypt） ----
if (!do_encrypt) {
    poly1305_auth(expected_tag, src, aadlen + len, poly_key);
    if (timingsafe_bcmp(expected_tag, tag, POLY1305_TAGLEN) != 0)
        return SSH_ERR_MAC_INVALID;   // tag 不匹配，直接丢弃
}

// ---- 步骤 3: 用 header_ctx 加密 packet_length (4字节) ----
chacha_ivsetup(&ctx->header_ctx, seqbuf, NULL);
chacha_encrypt_bytes(&ctx->header_ctx, src, dest, aadlen);
//                                          ^^^  ^^^^  ^^^^^^
//                                          明文  密文  4字节

// ---- 步骤 4: 用 main_ctx + counter=1 加密 payload ----
chacha_ivsetup(&ctx->main_ctx, seqbuf, one);       // counter = 1
chacha_encrypt_bytes(&ctx->main_ctx, src + aadlen, dest + aadlen, len);

// ---- 步骤 5: 加密时计算 Poly1305 tag ----
if (do_encrypt) {
    poly1305_auth(dest + aadlen + len,                // tag 写到这里
                  dest,                                // 输入: 密文header + 密文payload
                  aadlen + len,                        // 整个密文长度
                  poly_key);                           // 一次性密钥
}
```

### 数据流图

```
64字节密钥 = [K_main(32B)] [K_header(32B)]
                  │              │
                  ▼              ▼
           ChaCha20-main    ChaCha20-header
                  │              │
     ┌────────────┤              │
     │ counter=0  │              │ nonce=seqnr
     │ nonce=seqnr│              │
     ▼            │              ▼
  Poly1305密钥    │        加密 packet_length(4B)
  (一次性,32B)    │        明文 → 密文
     │            │              │
     │  counter=1  │              │
     │  nonce=seqnr│              │
     ▼            ▼              ▼
  加密 pad_len + payload + padding     密文header(4B)
  明文 → 密文                           │
     │                                 │
     ▼                                 │
  Poly1305(K_poly, 密文header || 密文payload)
     │
     ▼
  16字节认证标签 tag
```

### 特殊之处

**1. packet_length 也被加密了**

这与 AES-GCM 和 EtM 完全不同：

| 模式 | packet_length 线上状态 |
|------|----------------------|
| EtM (aes-ctr + hmac-etm) | 明文 |
| AES-GCM | 明文（作为 AAD） |
| ChaCha20-Poly1305 | **密文**（被 header_ctx 加密） |

虽然调用时传入 `aadlen=4`，但这 4 字节对 ChaCha20-Poly1305 不是传统 AAD（"认证但不加密"），而是被 header_ctx 加密后**再**参与 Poly1305 认证。

**2. Poly1305 密钥每包动态生成**

同一个 main_ctx，用同一个 nonce（seqnr），但通过不同的 block counter 确保密钥生成和数据加密使用不同的密钥流：

- counter=0 → 加密全零 → 得到 32 字节一次性 poly_key
- counter=1 → 加密实际数据

**3. authenticate-then-decrypt**

解密时先验证 Poly1305 tag，通过后才解密数据。恶意数据在解密前就被丢弃，不会进入解密器。

**4. 接收端用 header_ctx 独立解密 packet_length**

```c
// chachapoly_get_length():
chacha_ivsetup(&ctx->header_ctx, seqbuf, NULL);
chacha_encrypt_bytes(&ctx->header_ctx, cp, buf, 4);   // 只解密4字节
*plenp = PEEK_U32(buf);
```

接收端只需前 4 字节即可获知包长度，不需要等完整包到达。

---

## 8. AES-GCM 完整加密过程

源码：[cipher.c:364-401](../cipher.c)

### IV 结构

AES-GCM 使用 12 字节 IV：
- 前 4 字节：KEX 派生的固定部分（`EVP_CTRL_GCM_SET_IV_FIXED` 设置）
- 后 8 字节：隐式自增计数器（`EVP_CTRL_GCM_IV_GEN` 每次加密时 +1）

无需显式传输 IV，通信双方各自维护计数器。

### 加密 4 步

```c
// 阶段 1: IV 自增
EVP_CIPHER_CTX_ctrl(cc->evp, EVP_CTRL_GCM_IV_GEN, 1, lastiv);

// 阶段 2: 将 packet_length 作为 AAD 喂入（不加密，只认证）
EVP_Cipher(cc->evp, NULL, (u_char *)src, aadlen);   // dest=NULL → 不输出
memcpy(dest, src, aadlen);                            // 原样拷贝（明文）

// 阶段 3: 加密 pad_len + payload + padding
EVP_Cipher(cc->evp, dest + aadlen, (u_char *)src + aadlen, len);

// 阶段 4: 生成 GCM 认证标签
EVP_Cipher(cc->evp, NULL, NULL, 0);                   // finalize
EVP_CIPHER_CTX_ctrl(cc->evp, EVP_CTRL_GCM_GET_TAG, 16, dest + aadlen + len);
```

### 与 ChaCha20-Poly1305 的关键区别

| | AES-GCM | ChaCha20-Poly1305 |
|---|---|---|
| packet_length | 明文 AAD（不加密） | 密文（header_ctx 加密） |
| 密钥数量 | 1 个 | 2 个（main + header） |
| IV/nonce | 隐式自增（12字节） | seqnr（8字节） |
| 认证标签 | GCM 内置 | Poly1305 外置计算 |
| 解密顺序 | decrypt-then-verify | authenticate-then-decrypt |
| OpenSSL 依赖 | 是 | 可内置，也可用 OpenSSL |

---

## 9. Padding 计算详解

### 为什么需要 padding

SSH2 协议要求：

1. `4 + packet_length` 必须是 `block_size` 的整数倍（对齐加密块边界）
2. **padding 最少 4 字节**（RFC 4253 规定，防止流量分析精确推断 payload 长度）

### 计算逻辑

```c
len = sshbuf_len(state->outgoing_packet);   // 4(pkt_len) + 1(pad_len) + payload
len -= aadlen;   // AEAD/EtM 模式下 packet_length(4字节) 不参与对齐计算

padlen = block_size - (len % block_size);
if (padlen < 4)
    padlen += block_size;
```

### `if (padlen < 4)` 的含义

`block_size - (len % block_size)` 算出"对齐到 block 边界需要多少 padding"。如果算出来不够 4 字节，就再加一整个 block。

以 `block_size = 16` 为例：

| `len % 16` | padlen 初始值 | < 4 ? | padlen 最终值 | 总包大小 |
|:---:|:---:|:---:|:---:|:---:|
| 10 | 6 | 否 | 6 | len+6 |
| 12 | 4 | 否 | 4 | len+4 |
| **13** | **3** | **是** | **3+16=19** | len+19 |
| **14** | **2** | **是** | **2+16=18** | len+18 |
| **15** | **1** | **是** | **1+16=17** | len+17 |
| 0 | 16 | 否 | 16 | len+16 |

当 `len % 16 == 0` 时，padlen = 16（不是 0），因为 `16 - 0 = 16`，直接就是一整个 block 的 padding，满足 ≥ 4。

### 不同算法的 block_size 影响

| 算法 | block_size | 特点 |
|------|:---:|------|
| aes256-ctr | 16 | 对齐要求严格，padding 变化大 |
| aes256-gcm | 16 | 同上 |
| **chacha20-poly1305** | **8** | 对齐要求更宽松，padding 更少 |
| 3des-cbc | 8 | 同 chacha20 的 block_size |

例如 `len=20`（压缩后的 outgoing_packet）：

- block_size=16：`padlen = 16 - (20%16) = 16 - 4 = 12`，但 aadlen=4 时 `len` 先减 4 变成 16，`padlen = 16 - (16%16) = 16`
- block_size=8：aadlen=4 时 `len=16`，`padlen = 8 - (16%8) = 8`

**这就是 ChaCha20-Poly1305 的包通常比 AES-GCM 更小的原因之一——padding 更少。**

---

## 10. cipher_crypt 参数详解

### 调用处（[packet.c:1341](../packet.c)）

```c
cipher_crypt(state->send_context, state->p_send.seqnr, cp,
    sshbuf_ptr(state->outgoing_packet),
    len - aadlen,    // 第5个参数：需要加密的长度
    aadlen,          // 第6个参数：AAD 长度（不加密但参与认证）
    authlen);        // 第7个参数：认证标签长度
```

### 三个参数的含义

| 参数 | 公式 | 含义 |
|------|------|------|
| 第 5 个 `len` | `len - aadlen` | **需要加密的部分长度** |
| 第 6 个 `aadlen` | `aadlen` | **不加密、但参与认证的前缀长度** |
| 第 7 个 `authlen` | `authlen` | **认证标签长度** |

### 三种算法下的值

```
非 AEAD (aes256-ctr + hmac-sha2-256):
  aadlen=0, authlen=0
  cipher_crypt(..., len-0, 0, 0)
                ^^^^^^  ^  ^
                整个包  无AAD  无标签
                全部加密

AES-GCM:
  aadlen=4, authlen=16
  cipher_crypt(..., len-4, 4, 16)
                ^^^^^^  ^  ^^
                后28B   前4B  16B标签
                加密    AAD
```

### 在 cipher_crypt 内部怎么用

```c
// src 指向整个 outgoing_packet:
// [packet_len(4B)] [pad_len(1B)] [payload] [padding]

// AAD 部分：复制前 aadlen 字节（不加密）
memcpy(dest, src, aadlen);

// 加密部分：从偏移 aadlen 开始，加密 len 字节
EVP_Cipher(cc->evp, dest + aadlen, src + aadlen, len);
//                     ^^^^^^^^^^        ^^^^^^^^^^  ^^^
//                     输出偏移aadlen     输入偏移aadlen  加密长度
```

非 AEAD 时 `aadlen=0`：`memcpy(dest, src, 0)` 什么都不做，`EVP_Cipher` 从偏移 0 开始加密整个包。

---

## 11. 四种场景逐步走查

以下所有场景使用相同的基础数据：payload = 20 字节（`SSH2_MSG_CHANNEL_DATA` + 20B 应用数据），假设压缩后 `type+payload` 从 21B 变为 15B。

进入 `ssh_packet_send2_wrapped()` 时的初始状态（`sshpkt_start()` 写入 6 字节占位 + `sshpkt_put*()` 追加 20 字节）：

```
outgoing_packet（26字节）:
偏移:  0  1  2  3  4  5  6 ... 25
内容: [00 00 00 00] [00] [5e] [应用数据 20B]
       ^packet_len  ^pad  ^type  ^payload
       占位         占位
```

### 场景 A：非 AEAD + 无压缩（aes256-ctr + hmac-sha2-256）

**参数**：`authlen=0`, `aadlen=0`, `block_size=16`, `mac` = hmac-sha2-256

**步骤 1：压缩 → 跳过**（`comp->enabled == false`）

**步骤 2：计算 padding**

```c
len = 26;                    // 无 aadlen，不减
padlen = 16 - (26 % 16) = 6; // 6 ≥ 4，直接用
arc4random_buf(cp, 6);       // 6 字节随机 padding
```

**步骤 3：填入头部**

```c
POKE_U32(cp, 32 - 4);   // packet_length = 28
cp[4] = 6;               // pad_len = 6
```

此时 outgoing_packet（32字节）：

```
[00 00 00 1c] [06] [5e] [应用数据 20B] [随机 6B]
 ^pkt_len=28  ^pad=6  ^type  ^payload    ^padding
```

**步骤 4：计算 MAC（Encrypt-and-MAC，先算）**

```c
mac_compute(mac, seqnr, outgoing_packet, 32, macbuf, 32);
// 内部：MAC = HMAC(seqnr(4B) || 完整明文包(32B))
```

**步骤 5：加密**

```c
cipher_crypt(..., dest, src, 32, 0, 0);
// 内部：EVP_Cipher(dest, src, 32) — 全部 32 字节加密
```

**步骤 6：追加 MAC**

```c
sshbuf_put(output, macbuf, 32);  // 追加 32 字节 MAC
```

**线上发送 64 字节**：

```
[================ 密文 32B ================] [==== MAC 32B ====]
Enc(pkt_len||pad_len||type||payload||padding)  HMAC(seqnr||明文)
```

---

### 场景 B：非 AEAD + 压缩开启（aes256-ctr + hmac-sha2-256）

**参数**：`authlen=0`, `aadlen=0`, `block_size=16`, `mac` = hmac-sha2-256

**步骤 1：压缩**

```c
sshbuf_consume(outgoing_packet, 5);    // 跳过前 5 字节，剩 21B
compress_buffer(outgoing_packet, compression_buffer);  // 21B → 15B
sshbuf_put(outgoing_packet, "\0\0\0\0\0", 5);  // 恢复空头部
sshbuf_putb(outgoing_packet, compression_buffer);      // 追加压缩数据
```

outgoing_packet 变为 20 字节：

```
[00 00 00 00] [00] [压缩数据 15B]
 ^pkt_len占位  ^pad占位  ^compressed
```

**步骤 2：计算 padding**

```c
len = 20;                    // 压缩后包变短了
padlen = 16 - (20 % 16) = 12; // 需要 12 字节 padding
arc4random_buf(cp, 12);
```

**步骤 3：填入头部**

```c
POKE_U32(cp, 32 - 4);   // packet_length = 28
cp[4] = 12;              // pad_len = 12
```

outgoing_packet（32字节）：

```
[00 00 00 1c] [0c] [压缩数据 15B] [随机 12B]
 ^pkt_len=28  ^pad=12  ^compressed  ^padding
```

**步骤 4-6 与场景 A 完全一样**（MAC → 加密 → 追加 MAC）

**线上发送 64 字节**（与场景 A 相同大小，因为恰好对齐到 32B）

```
[================ 密文 32B ================] [==== MAC 32B ====]
Enc(pkt_len||pad_len||压缩数据||padding)      HMAC(seqnr||明文)
```

---

### 场景 C：AES-GCM + 压缩开启（aes256-gcm@openssh.com）

**参数**：`authlen=16`, `aadlen=4`, `block_size=16`, `mac=NULL`

**步骤 1：压缩** → 同场景 B，outgoing_packet 变为 20 字节

**步骤 2：计算 padding**

```c
len = 20;
len -= aadlen;   // aadlen=4 → len = 16
//                  ^^^^^^^^
//                  packet_length(4B) 不参与对齐！

padlen = 16 - (16 % 16) = 16;  // 注意：因为 len 减了 4，结果不同
arc4random_buf(cp, 16);
```

**步骤 3：填入头部**

```c
POKE_U32(cp, 36 - 4);   // packet_length = 32
cp[4] = 16;              // pad_len = 16
```

outgoing_packet（36字节）：

```
[00 00 00 20] [10] [压缩数据 15B] [随机 16B]
 ^pkt_len=32  ^pad=16  ^compressed  ^padding
```

**步骤 4：MAC → 跳过**（`mac == NULL`，AEAD 无独立 MAC）

**步骤 5：加密**

```c
cipher_crypt(..., dest, src, 32, 4, 16);
//                     ^^^  ^  ^^
//                     加密  AAD tag
```

进入 `cipher_crypt()` 的 OpenSSL GCM 路径（[cipher.c:364-401](../cipher.c)）：

```
阶段 1: IV 自增
  EVP_CTRL_GCM_IV_GEN → IV 后 8 字节 +1

阶段 2: AAD（前 4 字节 packet_length）
  EVP_Cipher(NULL, src, 4)   → dest=NULL，不输出密文
                                 只把数据喂入 GHASH 认证引擎
  memcpy(dest, src, 4)       → 原样拷贝到输出（明文）
                                 dest[0..3] = [00 00 00 20] ← 明文!

阶段 3: 加密 pad_len + 压缩数据 + padding（后 32 字节）
  EVP_Cipher(dest+4, src+4, 32)  → 正常加密，输出密文

阶段 4: 生成 GCM 认证标签
  EVP_Cipher(NULL, NULL, 0)       → finalize，触发 tag 计算
  GET_TAG(16B, dest+36)           → 16 字节 tag 写到输出末尾
```

**GCM 中 AAD 的特殊处理**：

`EVP_Cipher(evp, NULL, src, 4)` 是 OpenSSL GCM 的特殊约定：

- `dest = NULL` → 不做加密，只把数据"喂入"GCM 的 GHASH 认证函数
- GHASH 会对这 4 字节做多项式运算，混入最终 tag
- 但这 4 字节不会被加密，原样输出

因此 packet_length 虽然是明文，但受到 GCM tag 保护——如果有人篡改 packet_length，接收端 GHASH 算出的 tag 会不匹配，解密失败。

```
tag = GHASH(key, AAD || ciphertext)
              ↑ 覆盖两部分：
  ┌───────────┐  ┌────────────────────────────────────┐
  │ AAD(4B)   │  │ 密文(32B)                           │
  │ pkt_len   │  │ Enc(pad_len + 压缩数据 + padding)   │
  │ 明文,只认证│  │ AES-CTR 加密                        │
  └───────────┘  └────────────────────────────────────┘
       ↑                     ↑
  EVP_Cipher(NULL,src,4)  EVP_Cipher(dest+4,src+4,32)
  只混入 GHASH            加密 + 混入 GHASH
```

**步骤 6：追加 MAC → 跳过**（mac == NULL）

**线上发送 52 字节**：

```
[pkt_len明文] [========== 密文 32B ==========] [GCM tag 16B]
   4B          Enc(pad+压缩数据+padding)        认证标签
  (AAD)                                        (覆盖AAD+密文)
```

---

### 场景 D：ChaCha20-Poly1305 + 压缩开启（chacha20-poly1305@openssh.com）

**参数**：`authlen=16`, `aadlen=4`, `block_size=8`, `mac=NULL`

> 注意 block_size 是 **8**，不是 16！

**步骤 1：压缩** → 同场景 B/C，outgoing_packet 变为 20 字节

**步骤 2：计算 padding**

```c
len = 20;
len -= aadlen;   // aadlen=4 → len = 16

padlen = 8 - (16 % 8) = 8;  // block_size=8，padding 更少！
arc4random_buf(cp, 8);
```

**步骤 3：填入头部**

```c
POKE_U32(cp, 28 - 4);   // packet_length = 24
cp[4] = 8;               // pad_len = 8
```

outgoing_packet（28字节）：

```
[00 00 00 18] [08] [压缩数据 15B] [随机 8B]
 ^pkt_len=24  ^pad=8   ^compressed  ^padding
```

**步骤 4：MAC → 跳过**

**步骤 5：加密** — 进入专属实现（[cipher-chachapoly.c:68](../cipher-chachapoly.c)）

```c
// 不走 OpenSSL EVP 路径，直接调用 chachapoly_crypt()
if ((cc->cipher->flags & CFLAG_CHACHAPOLY) != 0) {
    return chachapoly_crypt(cc->cp_ctx, seqnr, dest, src,
        len, aadlen, authlen, cc->encrypt);
}
```

chachapoly_crypt 内部 4 个阶段：

```
阶段 1: 生成 Poly1305 一次性密钥
  main_ctx(key[0..31]) + nonce=seqnr + counter=0
  ChaCha20(全零 32B) → poly_key(32B)
  意思：counter=0 的密钥流 XOR 全零 = 密钥流本身，作为认证密钥

阶段 2: header_ctx 加密 packet_length（4字节）
  header_ctx(key[32..63]) + nonce=seqnr
  ChaCha20([00 00 00 18]) → [XX XX XX XX]（密文）
  dest[0..3] = pkt_len 密文 ← 加密了！不像 AES-GCM 是明文

阶段 3: main_ctx + counter=1 加密 payload（后 24 字节）
  main_ctx(key[0..31]) + nonce=seqnr + counter=1
  ChaCha20([08][压缩数据 15B][随机 8B]) → 密文(24B)
  counter=1 确保和阶段 1（counter=0）使用不同的密钥流

阶段 4: Poly1305 认证标签
  poly1305_auth(dest+28, dest, 28, poly_key)
  覆盖: 密文header(4B) + 密文payload(24B) → 16字节 tag
```

**步骤 6：追加 MAC → 跳过**

**线上发送 44 字节**：

```
[pkt_len密文] [========== 密文 24B ==========] [Poly1305 tag 16B]
  4B           Enc(pad+压缩数据+padding)        认证标签
 header_ctx     main_ctx(counter=1)           (认证整个密文)
```

---

### 四种场景对比表

| | A: 非AEAD 无压缩 | B: 非AEAD 压缩 | C: AES-GCM 压缩 | D: ChaCha20 压缩 |
|---|---|---|---|---|
| **aadlen** | 0 | 0 | 4 | 4 |
| **authlen** | 0 | 0 | 16 | 16 |
| **block_size** | 16 | 16 | 16 | **8** |
| **mac** | hmac-sha2-256 | hmac-sha2-256 | NULL | NULL |
| 压缩 | 无 | 21B→15B | 21B→15B | 21B→15B |
| padlen 计算 | `16-(26%16)=6` | `16-(20%16)=12` | `16-((20-4)%16)=16` | `8-((20-4)%8)=8` |
| 明文包大小 | 32B | 32B | 36B | **28B** |
| cipher_crypt | `(32,0,0)` | `(32,0,0)` | `(32,4,16)` | `(24,4,16)` |
| 加密范围 | 全部 32B | 全部 32B | 后 32B | header(4B)+payload(24B) |
| pkt_len 线上 | 密文 | 密文 | **明文** | **密文** |
| 认证方式 | HMAC 32B | HMAC 32B | GCM tag 16B | Poly1305 16B |
| **线上总大小** | **64B** | **64B** | **52B** | **44B** |

---

## 12. 压缩的启用时机

### 立即压缩

KEX 完成、收到 `SSH2_MSG_NEWKEYS` 后立即启用。

### 延迟压缩（`COMP_DELAYED`）

```c
// 发送端：在发送 SSH2_MSG_USERAUTH_SUCCESS 后启用
if (type == SSH2_MSG_USERAUTH_SUCCESS && state->server_side)
    r = ssh_packet_enable_delayed_compress(ssh);

// 接收端：在收到 SSH2_MSG_USERAUTH_SUCCESS 后启用
if (*typep == SSH2_MSG_USERAUTH_SUCCESS && !state->server_side)
    r = ssh_packet_enable_delayed_compress(ssh);
```

**设计意图**：认证前不压缩，避免压缩流量泄露认证过程中的敏感信息（如密码长度）。认证后才启用压缩以提高后续数据传输效率。

---

## 13. 序列号的作用

每个方向维护独立的 32 位序列号（`state->p_send.seqnr` / `state->p_read.seqnr`）：

1. **从 0 开始**：连接建立后从 0 开始计数
2. **每个包 +1**：无论发送/接收任何类型的包都递增
3. **参与 MAC 计算**：`MAC = HMAC(seqnr || data)`，防止包重放/乱序
4. **ChaCha20-Poly1305 nonce**：seqnr 直接作为 ChaCha20 的 nonce
5. **Strict KEX 重置**：在 Strict KEX 模式下，发送/收到 `SSH2_MSG_NEWKEYS` 时序列号重置为 0

---

## 14. 完整数据流图

### 发送端（以 EtM 模式为例）

```
应用层数据（如 channel data）
        │
        ▼
┌─────────────────────────┐
│  sshpkt_start(type)     │  构造 outgoing_packet:
│  sshpkt_put*(data...)   │  [00 00 00 00] [00] [type] [data...]
│  sshpkt_send()          │      4B占位      1B占位
└──────────┬──────────────┘
           │
           ▼
┌─────────────────────────┐
│  ① 压缩                 │  compress_buffer() — zlib deflate
│     (跳过前5字节头部)    │  只压缩 [type] + [data]
└──────────┬──────────────┘
           │
           ▼
┌─────────────────────────┐
│  ② 计算 padding         │  padlen = block_size - (len % block_size)
│  ③ 填充随机数据          │  arc4random_buf(padding, padlen)
│  ④ 填写 packet_len      │  POKE_U32(cp, len - 4)
│     和 pad_len          │  cp[4] = padlen
└──────────┬──────────────┘
           │
           │  此时 outgoing_packet:
           │  [packet_len(4B)] [pad_len(1B)] [type+payload] [padding]
           │
           ▼
┌─────────────────────────┐
│  ⑤ 加密                 │  cipher_crypt()
│  (跳过前4字节不加密)     │  加密: pad_len + payload + padding
│                          │  AAD: packet_length (4字节,明文)
└──────────┬──────────────┘
           │
           │  密文:
           │  [packet_len明文] [encrypted(pad_len+payload+padding)]
           │
           ▼
┌─────────────────────────┐
│  ⑥ 计算 MAC             │  MAC = HMAC(seqnr || 密文)
│     (Encrypt-then-MAC)   │  包含明文 packet_len
└──────────┬──────────────┘
           │
           ▼
┌─────────────────────────┐
│  ⑦ 输出到网络            │  output = 密文 || MAC
└──────────┬──────────────┘
           │
           ▼
      线上发送 →
```

---

## 附：线上字节流格式总结

### 非 AEAD + Encrypt-and-MAC（如 aes256-ctr + hmac-sha2-256）

```
[============= 全部加密 =============]  [====== MAC ======]
[packet_len | pad_len | payload | padding]  [HMAC(seqnr||明文)]
                  密文                           明文MAC
```

### 非 AEAD + Encrypt-then-MAC（如 aes256-ctr + hmac-sha2-256-etm）

```
[明文]  [========== 加密 =========]  [====== MAC ======]
[pkt_len | pad_len | payload | padding]  [HMAC(seqnr||密文)]
  4B              密文                       明文MAC
```

### AEAD - AES-GCM（如 aes256-gcm@openssh.com）

```
[AAD明文]  [======== GCM加密 ========]  [== GCM Tag ==]
[pkt_len]  [pad_len | payload | padding]  [tag(16B)]
  4B                 密文                   认证标签
(不加密,只认证)
```

### AEAD - ChaCha20-Poly1305（如 chacha20-poly1305@openssh.com）

```
[header_ctx加密]  [===== main_ctx加密 =====]  [== Poly1305 ==]
[pkt_len]         [pad_len | payload | padding]  [tag(16B)]
  4B                        密文                  认证标签
(加密+认证)               (counter=1)        (认证整个密文)
```

> ChaCha20-Poly1305 是唯一对 packet_length 也做加密的算法，提供更好的流量分析抵抗力。
