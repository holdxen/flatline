# SSH2 / SSHv2 协议消息类型与格式（IETF RFC + OpenSSH 扩展）

生成日期：2026-06-23
范围：SSH Protocol Version 2（SSHv2）在 IETF RFC 与 IANA “Secure Shell Protocol Parameters” 中定义/注册的 **SSH 传输层、认证层、连接层消息类型**及其标准格式，以及 **OpenSSH 实现的私有扩展**。

> 说明：
>
> 1. SSH 报文在二进制包协议（Binary Packet Protocol）中承载；这里列出的”消息格式”通常是 `payload` 的结构，且第一个字段通常是 `byte SSH_MSG_*`。
> 2. SSH 的 `30–49` 号段是 **Key Exchange method specific**，不同 KEX 方法可复用同一数字；`60–79` 号段是 **User Authentication method specific**，不同认证方法也可复用同一数字。
> 3. `128–191` 是 client protocols、`192–255` 是 private use；OpenSSH 在 `192–255` 范围内定义了 `SSH2_MSG_PING` (192) 和 `SSH2_MSG_PONG` (193)。
> 4. RFC 9987 的 SSH Agent 子协议不是 SSH 传输层的 `SSH_MSG_*` 消息；但 RFC 9987 新注册的 `agent-req` channel request 与 `agent-connect` channel type 已列入连接层扩展。
> 5. 本文不列 SFTP、publickey subsystem 等运行在 SSH channel 之上的独立子协议消息。
> 6. 标记为 **[OpenSSH]** 的内容为 OpenSSH 私有扩展，不属于 IETF RFC 标准。

---

## 0. 类型记法速查

| 类型          | 含义                                     |
| ----------- | -------------------------------------- |
| `byte`      | 8-bit 无符号整数；消息类型字段通常是 `byte`           |
| `boolean`   | 1 字节，`0` 为 FALSE，非 `0` 为 TRUE          |
| `uint32`    | 32-bit 无符号整数，网络字节序                     |
| `uint64`    | 64-bit 无符号整数，网络字节序                     |
| `string`    | `uint32 length` + `length` 字节；可承载任意二进制 |
| `mpint`     | 多精度整数，二进制补码表示                          |
| `name-list` | `string` 的子类型，逗号分隔的 ASCII 名称列表         |
| `...`       | 方法/请求/通道类型特定字段                         |

### 0.2 String 字段编码标注约定

本文使用以下记法明确标注 `string` 字段的编码要求：

| 记法                 | 含义                                               |
| ------------------ | ------------------------------------------------ |
| `string[bytes]`    | SSH 原生 `string`，即 length-prefixed 二进制数据，不假设字符编码  |
| `string[utf8]`     | SSH `string`，但字段内容按 RFC 明确要求为 ISO-10646 UTF-8    |
| `string[ascii]`    | SSH `string`，但字段内容是 US-ASCII 协议名、方法名、请求名、算法名等    |
| `string[langtag]`  | SSH `string`，但字段内容是 language tag，不按普通 UTF-8 文本处理 |
| `name-list[ascii]` | SSH `name-list`，元素通常是 US-ASCII 名称                |

> 注：`string[utf8]` 在线路编码上仍然是 SSH 的 `string`：`uint32 length || bytes`。区别只是 `bytes` 的内容必须是合法 UTF-8 文本。

### 0.1 SSH 二进制包封装

```text
uint32    packet_length
byte      padding_length
byte[n1]  payload
byte[n2]  random padding
byte[m]   mac
```

其中 `payload` 的第一个字节通常是本文件列出的 `SSH_MSG_*` 类型。

---

## 1. IANA SSH Message Numbers 总览

|    值/范围 | Message ID                          | 层/说明                                | RFC                  |
| ------: | ----------------------------------- | ----------------------------------- | -------------------- |
|       0 | Reserved                            | 保留                                  | RFC 4250             |
|       1 | `SSH_MSG_DISCONNECT`                | Transport generic                   | RFC 4253             |
|       2 | `SSH_MSG_IGNORE`                    | Transport generic                   | RFC 4253             |
|       3 | `SSH_MSG_UNIMPLEMENTED`             | Transport generic                   | RFC 4253             |
|       4 | `SSH_MSG_DEBUG`                     | Transport generic                   | RFC 4253             |
|       5 | `SSH_MSG_SERVICE_REQUEST`           | Transport service                   | RFC 4253             |
|       6 | `SSH_MSG_SERVICE_ACCEPT`            | Transport service                   | RFC 4253             |
|       7 | `SSH_MSG_EXT_INFO`                  | Extension negotiation               | RFC 8308             |
|       8 | `SSH_MSG_NEWCOMPRESS`               | Extension negotiation               | RFC 8308             |
|    9–19 | Unassigned                          | Transport generic                   | IANA                 |
|      20 | `SSH_MSG_KEXINIT`                   | Algorithm negotiation               | RFC 4253             |
|      21 | `SSH_MSG_NEWKEYS`                   | New keys marker                     | RFC 4253             |
|   22–29 | Unassigned                          | Algorithm negotiation               | IANA                 |
|   30–49 | Method specific                     | Key exchange method specific        | 多个 RFC               |
|      50 | `SSH_MSG_USERAUTH_REQUEST`          | User Authentication generic         | RFC 4252             |
|      51 | `SSH_MSG_USERAUTH_FAILURE`          | User Authentication generic         | RFC 4252             |
|      52 | `SSH_MSG_USERAUTH_SUCCESS`          | User Authentication generic         | RFC 4252             |
|      53 | `SSH_MSG_USERAUTH_BANNER`           | User Authentication generic         | RFC 4252             |
|   54–59 | Unassigned                          | User Authentication generic         | IANA                 |
|   60–79 | Method specific                     | User Authentication method specific | RFC 4252/4256/4462 等 |
|      80 | `SSH_MSG_GLOBAL_REQUEST`            | Connection global                   | RFC 4254             |
|      81 | `SSH_MSG_REQUEST_SUCCESS`           | Connection global                   | RFC 4254             |
|      82 | `SSH_MSG_REQUEST_FAILURE`           | Connection global                   | RFC 4254             |
|   83–89 | Unassigned                          | Connection generic                  | IANA                 |
|      90 | `SSH_MSG_CHANNEL_OPEN`              | Channel management                  | RFC 4254             |
|      91 | `SSH_MSG_CHANNEL_OPEN_CONFIRMATION` | Channel management                  | RFC 4254             |
|      92 | `SSH_MSG_CHANNEL_OPEN_FAILURE`      | Channel management                  | RFC 4254             |
|      93 | `SSH_MSG_CHANNEL_WINDOW_ADJUST`     | Channel flow control                | RFC 4254             |
|      94 | `SSH_MSG_CHANNEL_DATA`              | Channel data                        | RFC 4254             |
|      95 | `SSH_MSG_CHANNEL_EXTENDED_DATA`     | Channel extended data               | RFC 4254             |
|      96 | `SSH_MSG_CHANNEL_EOF`               | Channel EOF                         | RFC 4254             |
|      97 | `SSH_MSG_CHANNEL_CLOSE`             | Channel close                       | RFC 4254             |
|      98 | `SSH_MSG_CHANNEL_REQUEST`           | Channel request                     | RFC 4254             |
|      99 | `SSH_MSG_CHANNEL_SUCCESS`           | Channel request reply               | RFC 4254             |
|     100 | `SSH_MSG_CHANNEL_FAILURE`           | Channel request reply               | RFC 4254             |
| 101–127 | Unassigned                          | Connection protocol                 | IANA                 |
| 128–191 | Reserved                            | Client protocols                    | RFC 4250/IANA        |
|     192 | `SSH2_MSG_PING` **[OpenSSH]**       | Transport ping                      | OpenSSH              |
|     193 | `SSH2_MSG_PONG` **[OpenSSH]**       | Transport pong                      | OpenSSH              |
| 194–255 | Local extensions                    | Private Use                         | RFC 4250/IANA        |

---

# 2. Transport Layer：通用消息与算法协商

## 2.1 `SSH_MSG_DISCONNECT` / 1

```text
byte             SSH_MSG_DISCONNECT
uint32           reason code
string[utf8]     description
string[langtag]  language tag
```

## 2.2 `SSH_MSG_IGNORE` / 2

```text
byte            SSH_MSG_IGNORE
string[bytes]   data
```

## 2.3 `SSH_MSG_UNIMPLEMENTED` / 3

```text
byte      SSH_MSG_UNIMPLEMENTED
uint32    packet sequence number of rejected message
```

## 2.4 `SSH_MSG_DEBUG` / 4

```text
byte             SSH_MSG_DEBUG
boolean          always_display
string[utf8]     message
string[langtag]  language tag
```

## 2.5 `SSH_MSG_SERVICE_REQUEST` / 5

```text
byte            SSH_MSG_SERVICE_REQUEST
string[ascii]   service name
```

## 2.6 `SSH_MSG_SERVICE_ACCEPT` / 6

```text
byte            SSH_MSG_SERVICE_ACCEPT
string[ascii]   service name
```

## 2.7 `SSH_MSG_EXT_INFO` / 7（RFC 8308）

```text
byte            SSH_MSG_EXT_INFO
uint32          nr-extensions
repeat nr-extensions times:
  string[ascii]  extension-name
  string[bytes]  extension-value
```

OpenSSH 实际发送的扩展名（源码 `kex.c:292-331`）：

**服务器端**（在 `SSH_MSG_NEWKEYS` 之后发送，nr-extensions = 4）：

| extension-name | extension-value | 说明 |
|----------------|-----------------|------|
| `ext-info-s` | （空） | 声明服务器支持扩展信息 |
| `server-sig-algs` | 逗号分隔的算法名列表 | 服务器支持的公钥签名算法 |
| `publickey-hostbound@openssh.com` | `"0"` | 支持主机绑定公钥认证 |
| `ping@openssh.com` | `"0"` | 支持传输层 ping/pong |
| `agent-forward` | `"0"` | 支持 RFC 9987 agent 转发 |

**客户端**（在 `SSH_MSG_NEWKEYS` 之后发送，nr-extensions = 1）：

| extension-name | extension-value | 说明 |
|----------------|-----------------|------|
| `ext-info-in-auth@openssh.com` | `"0"` | 允许认证期间接收 EXT_INFO |

**认证后 EXT_INFO**（服务器在用户认证成功后可发送第二次，nr-extensions = 1）：

| extension-name | extension-value | 说明 |
|----------------|-----------------|------|
| `server-sig-algs` | 逗号分隔的算法名列表 | 更新服务器签名算法列表（per-user） |

## 2.8 `SSH_MSG_NEWCOMPRESS` / 8（RFC 8308）

```text
byte      SSH_MSG_NEWCOMPRESS
```

> 注：OpenSSH 当前**未实现**此消息。源码中定义了 `SSH2_MSG_NEWCOMPRESS` 常量（`ssh2.h:89`），但没有发送或处理此消息的代码。

## 2.9 `SSH_MSG_KEXINIT` / 20

```text
byte                SSH_MSG_KEXINIT
byte[16]            cookie
name-list[ascii]    kex_algorithms
name-list[ascii]    server_host_key_algorithms
name-list[ascii]    encryption_algorithms_client_to_server
name-list[ascii]    encryption_algorithms_server_to_client
name-list[ascii]    mac_algorithms_client_to_server
name-list[ascii]    mac_algorithms_server_to_client
name-list[ascii]    compression_algorithms_client_to_server
name-list[ascii]    compression_algorithms_server_to_client
name-list[ascii]    languages_client_to_server
name-list[ascii]    languages_server_to_client
boolean             first_kex_packet_follows
uint32              0
```

## 2.10 `SSH_MSG_NEWKEYS` / 21

```text
byte      SSH_MSG_NEWKEYS
```

## 2.11 `SSH2_MSG_PING` / 192 **[OpenSSH]**

```text
byte            SSH2_MSG_PING
string[bytes]   data
```

OpenSSH 实现的传输层 ping 消息，用于延迟测量和连接保活。通过 `SSH2_MSG_EXT_INFO` 广告扩展名 `"ping@openssh.com"`，版本 `"0"`。

## 2.12 `SSH2_MSG_PONG` / 193 **[OpenSSH]**

```text
byte            SSH2_MSG_PONG
string[bytes]   data (从 PING 复制)
```

对 `SSH2_MSG_PING` 的响应，按顺序发送。在 rekeying 期间会排队等待，直到 rekeying 完成后才发送。

---

# 3. Key Exchange：30–49 方法特定消息

> 30–49 的数字会被不同 KEX 方法复用，解析时必须依据当前协商的 KEX 方法。

## 3.1 固定组 Diffie-Hellman KEX（RFC 4253 等）

### `SSH_MSG_KEXDH_INIT` / 30

```text
byte      SSH_MSG_KEXDH_INIT
mpint     e
```

### `SSH_MSG_KEXDH_REPLY` / 31

```text
byte            SSH_MSG_KEXDH_REPLY
string[bytes]   server public host key and certificates (K_S)
mpint           f
string[bytes]   signature of H
```

## 3.2 Diffie-Hellman Group Exchange（RFC 4419）

### `SSH_MSG_KEX_DH_GEX_REQUEST_OLD` / 30

```text
byte      SSH_MSG_KEX_DH_GEX_REQUEST_OLD
uint32    n
```

### `SSH_MSG_KEX_DH_GEX_GROUP` / 31

```text
byte      SSH_MSG_KEX_DH_GEX_GROUP
mpint     p
mpint     g
```

### `SSH_MSG_KEX_DH_GEX_INIT` / 32

```text
byte      SSH_MSG_KEX_DH_GEX_INIT
mpint     e
```

### `SSH_MSG_KEX_DH_GEX_REPLY` / 33

```text
byte            SSH_MSG_KEX_DH_GEX_REPLY
string[bytes]   server public host key and certificates (K_S)
mpint           f
string[bytes]   signature of H
```

### `SSH_MSG_KEX_DH_GEX_REQUEST` / 34

```text
byte      SSH_MSG_KEX_DH_GEX_REQUEST
uint32    min
uint32    n
uint32    max
```

> 注：RFC 4419 文本中有一处 `SSH_MSG_KEY_DH_GEX_REQUEST` 拼写错误；实际注册/使用名称为 `SSH_MSG_KEX_DH_GEX_REQUEST`。

## 3.3 RSA Key Exchange（RFC 4432）

### `SSH_MSG_KEXRSA_PUBKEY` / 30

```text
byte            SSH_MSG_KEXRSA_PUBKEY
string[bytes]   server public host key and certificates (K_S)
string[bytes]   transient RSA public key (K_T)
```

### `SSH_MSG_KEXRSA_SECRET` / 31

```text
byte            SSH_MSG_KEXRSA_SECRET
string[bytes]   RSAES-OAEP-ENCRYPT(K_T, K)
```

### `SSH_MSG_KEXRSA_DONE` / 32

```text
byte            SSH_MSG_KEXRSA_DONE
string[bytes]   signature of H
```

## 3.4 ECDH KEX（RFC 5656）与 Curve25519/Curve448（RFC 8731）

### `SSH_MSG_KEX_ECDH_INIT` / 30

```text
byte            SSH_MSG_KEX_ECDH_INIT
string[bytes]   Q_C
```

### `SSH_MSG_KEX_ECDH_REPLY` / 31

```text
byte            SSH_MSG_KEX_ECDH_REPLY
string[bytes]   K_S
string[bytes]   Q_S
string[bytes]   signature of H
```

## 3.5 Hybrid sntrup761x25519 KEX（RFC 9941）

该方法在线路编码上复用 `SSH_MSG_KEX_ECDH_INIT` / `SSH_MSG_KEX_ECDH_REPLY`。规范可称为 `SSH_MSG_KEX_HYBRID_INIT` / `SSH_MSG_KEX_HYBRID_REPLY`，但 wire byte value 相同。

### Init / 30

```text
byte            SSH_MSG_KEX_ECDH_INIT
string[bytes]   Q_C
```

其中 `Q_C = sntrup761 public key (1158 bytes) || X25519 public key (32 bytes)`，总长 1190 字节。

### Reply / 31

```text
byte            SSH_MSG_KEX_ECDH_REPLY
string[bytes]   K_S
string[bytes]   Q_S
string[bytes]   signature of H
```

其中 `Q_S = sntrup761 ciphertext (1039 bytes) || X25519 public key (32 bytes)`，总长 1071 字节。

## 3.6 Hybrid ML-KEM 768 x25519 KEX **[OpenSSH]**

方法名：`mlkem768x25519-sha256`

该方法基于 NIST FIPS 203 ML-KEM-768 与 X25519 的混合密钥交换，在线路编码上复用 `SSH_MSG_KEX_ECDH_INIT` / `SSH_MSG_KEX_ECDH_REPLY`。

### Init / 30

```text
byte            SSH_MSG_KEX_ECDH_INIT
string[bytes]   Q_C
```

其中 `Q_C = ML-KEM 768 public key (1184 bytes) || X25519 public key (32 bytes)`，总长 1216 字节。

### Reply / 31

```text
byte            SSH_MSG_KEX_ECDH_REPLY
string[bytes]   K_S
string[bytes]   Q_S
string[bytes]   signature of H
```

其中 `Q_S = ML-KEM 768 ciphertext (1088 bytes) || X25519 public key (32 bytes)`，总长 1120 字节。

## 3.7 ECMQV KEX（RFC 5656）

### `SSH_MSG_ECMQV_INIT` / 30

```text
byte            SSH_MSG_ECMQV_INIT
string[bytes]   Q_C
```

### `SSH_MSG_ECMQV_REPLY` / 31

```text
byte            SSH_MSG_ECMQV_REPLY
string[bytes]   K_S
string[bytes]   Q_S
string[bytes]   HMAC tag
```

## 3.8 GSS-API Authenticated KEX（RFC 4462）

### `SSH_MSG_KEXGSS_INIT` / 30

```text
byte            SSH_MSG_KEXGSS_INIT
string[bytes]   output_token
mpint           e
```

### `SSH_MSG_KEXGSS_CONTINUE` / 31

```text
byte            SSH_MSG_KEXGSS_CONTINUE
string[bytes]   output_token
```

错误 token 也复用：

```text
byte            SSH_MSG_KEXGSS_CONTINUE
string[bytes]   error_token
```

### `SSH_MSG_KEXGSS_COMPLETE` / 32

有最终 token：

```text
byte            SSH_MSG_KEXGSS_COMPLETE
mpint           f
string[bytes]   per_msg_token
boolean         TRUE
string[bytes]   output_token
```

无最终 token：

```text
byte            SSH_MSG_KEXGSS_COMPLETE
mpint           f
string[bytes]   per_msg_token
boolean         FALSE
```

### `SSH_MSG_KEXGSS_HOSTKEY` / 33

```text
byte            SSH_MSG_KEXGSS_HOSTKEY
string[bytes]   server public host key and certificates
```

### `SSH_MSG_KEXGSS_ERROR` / 34

```text
byte             SSH_MSG_KEXGSS_ERROR
uint32           major_status
uint32           minor_status
string[utf8]     message
string[langtag]  language tag
```

### `SSH_MSG_KEXGSS_GROUPREQ` / 40

```text
byte      SSH_MSG_KEXGSS_GROUPREQ
uint32    min
uint32    n
uint32    max
```

### `SSH_MSG_KEXGSS_GROUP` / 41

```text
byte      SSH_MSG_KEXGSS_GROUP
mpint     p
mpint     g
```

## 3.9 OpenSSH 特有的 KEX 方法名称 **[OpenSSH]**

OpenSSH 支持以下 KEX 方法名称，其中部分为 OpenSSH 私有扩展或来自 libssh.org：

| KEX 方法名称 | 说明 | 来源 |
|--------------|------|------|
| `curve25519-sha256@libssh.org` | Curve25519 ECDH（与 `curve25519-sha256` 并存） | libssh.org |
| `sntrup761x25519-sha512` | 后量子混合 KEX（标准名） | RFC 9941 |
| `sntrup761x25519-sha512@openssh.com` | 后量子混合 KEX（旧名，功能相同） | OpenSSH |
| `mlkem768x25519-sha256` | 后量子混合 KEX（ML-KEM 768 + X25519） | OpenSSH |
| `ext-info-c` / `ext-info-s` | RFC 8308 扩展协商支持信号（客户端/服务器端） | RFC 8308 |
| `kex-strict-c-v00@openssh.com` / `kex-strict-s-v00@openssh.com` | Terrapin 攻击防护的严格 KEX 硬化 | OpenSSH |

> 注：`curve25519-sha256` 是 RFC 8731 定义的标准名称，`curve25519-sha256@libssh.org` 是早期实现名称，两者功能相同。`sntrup761x25519-sha512` 是 RFC 9941 定义的标准名称，`sntrup761x25519-sha512@openssh.com` 是早期 OpenSSH 私有名称，两者功能相同。`mlkem768x25519-sha256` 是基于 NIST FIPS 203 ML-KEM-768 的后量子混合 KEX，在 OpenSSH 中优先级最高。

---

# 4. User Authentication：通用消息与方法特定消息

## 4.1 `SSH_MSG_USERAUTH_REQUEST` / 50：通用格式

```text
byte            SSH_MSG_USERAUTH_REQUEST
string[utf8]    user name
string[ascii]   service name
string[ascii]   method name
...             method-specific fields
```

## 4.2 `SSH_MSG_USERAUTH_FAILURE` / 51

```text
byte                SSH_MSG_USERAUTH_FAILURE
name-list[ascii]    authentications that can continue
boolean             partial success
```

## 4.3 `SSH_MSG_USERAUTH_SUCCESS` / 52

```text
byte      SSH_MSG_USERAUTH_SUCCESS
```

## 4.4 `SSH_MSG_USERAUTH_BANNER` / 53

```text
byte             SSH_MSG_USERAUTH_BANNER
string[utf8]     message
string[langtag]  language tag
```

## 4.5 `none` 认证方法

```text
byte            SSH_MSG_USERAUTH_REQUEST
string[utf8]    user name
string[ascii]   service name
string[ascii]   "none"
```

## 4.6 `publickey` 认证方法（RFC 4252）

### Public key query / 50

```text
byte            SSH_MSG_USERAUTH_REQUEST
string[utf8]    user name
string[ascii]   service name
string[ascii]   "publickey"
boolean         FALSE
string[ascii]   public key algorithm name
string[bytes]   public key blob
```

### `SSH_MSG_USERAUTH_PK_OK` / 60

```text
byte            SSH_MSG_USERAUTH_PK_OK
string[ascii]   public key algorithm name from request
string[bytes]   public key blob from request
```

### Public key signed request / 50

```text
byte            SSH_MSG_USERAUTH_REQUEST
string[utf8]    user name
string[ascii]   service name
string[ascii]   "publickey"
boolean         TRUE
string[ascii]   public key algorithm name
string[bytes]   public key blob
string[bytes]   signature
```

## 4.7 `password` 认证方法（RFC 4252）

### Password request / 50

```text
byte            SSH_MSG_USERAUTH_REQUEST
string[utf8]    user name
string[ascii]   service name
string[ascii]   "password"
boolean         FALSE
string[utf8]    plaintext password
```

### `SSH_MSG_USERAUTH_PASSWD_CHANGEREQ` / 60

```text
byte             SSH_MSG_USERAUTH_PASSWD_CHANGEREQ
string[utf8]     prompt
string[langtag]  language tag
```

### Password change request / 50

```text
byte            SSH_MSG_USERAUTH_REQUEST
string[utf8]    user name
string[ascii]   service name
string[ascii]   "password"
boolean         TRUE
string[utf8]    plaintext old password
string[utf8]    plaintext new password
```

## 4.8 `hostbased` 认证方法（RFC 4252）

```text
byte            SSH_MSG_USERAUTH_REQUEST
string[utf8]    user name
string[ascii]   service name
string[ascii]   "hostbased"
string[ascii]   public key algorithm for host key
string[bytes]   public host key and certificates for client host
string[ascii]   client host name expressed as the FQDN
string[utf8]    user name on the client host
string[bytes]   signature
```

## 4.9 `keyboard-interactive` 认证方法（RFC 4256）

### Initial request / 50

```text
byte             SSH_MSG_USERAUTH_REQUEST
string[utf8]     user name
string[ascii]    service name
string[ascii]    "keyboard-interactive"
string[langtag]  language tag
string[utf8]     submethods
```

### `SSH_MSG_USERAUTH_INFO_REQUEST` / 60

```text
byte             SSH_MSG_USERAUTH_INFO_REQUEST
string[utf8]     name
string[utf8]     instruction
string[langtag]  language tag
uint32           num-prompts
repeat num-prompts times:
  string[utf8]   prompt
  boolean        echo
```

### `SSH_MSG_USERAUTH_INFO_RESPONSE` / 61

```text
byte            SSH_MSG_USERAUTH_INFO_RESPONSE
uint32          num-responses
repeat num-responses times:
  string[utf8]  response
```

## 4.10 `gssapi-with-mic` 认证方法（RFC 4462）

### Initial request / 50

```text
byte            SSH_MSG_USERAUTH_REQUEST
string[utf8]    user name
string[ascii]   service name
string[ascii]   "gssapi-with-mic"
uint32          n
string[bytes]   mechanism OID[1]
...
string[bytes]   mechanism OID[n]
```

> 注：每个 `mechanism OID` 字段的 SSH string 内容按 ASN.1 DER 编码：`byte 0x06`（OID 标签）+ `byte length` + `OID value bytes`。例如 Kerberos 5 OID `{1,2,840,113554,1,2,2}` 编码为 `06 09 2a 86 48 86 f7 12 01 02 02`（SSH string length = 11）。源码 `sshconnect2.c:793-797`。

### `SSH_MSG_USERAUTH_GSSAPI_RESPONSE` / 60

```text
byte            SSH_MSG_USERAUTH_GSSAPI_RESPONSE
string[bytes]   selected mechanism OID
```

### `SSH_MSG_USERAUTH_GSSAPI_TOKEN` / 61

```text
byte            SSH_MSG_USERAUTH_GSSAPI_TOKEN
string[bytes]   data
```

### `SSH_MSG_USERAUTH_GSSAPI_EXCHANGE_COMPLETE` / 63

```text
byte      SSH_MSG_USERAUTH_GSSAPI_EXCHANGE_COMPLETE
```

### `SSH_MSG_USERAUTH_GSSAPI_ERROR` / 64 **[OpenSSH 编号]**

```text
byte             SSH_MSG_USERAUTH_GSSAPI_ERROR
uint32           major_status
uint32           minor_status
string[utf8]     message
string[langtag]  language tag
```

### `SSH_MSG_USERAUTH_GSSAPI_ERRTOK` / 65 **[OpenSSH 编号]**

```text
byte            SSH_MSG_USERAUTH_GSSAPI_ERRTOK
string[bytes]   error token
```

> 注：RFC 4462 定义 `SSH_MSG_USERAUTH_GSSAPI_ERRTOK` = 64、`SSH_MSG_USERAUTH_GSSAPI_ERROR` = 65。但 OpenSSH（`ssh-gss.h:58-59`）将两者**互换**：ERROR = 64、ERRTOK = 65。这是 OpenSSH 的已知偏差，与其他实现互操作时需注意。

### `SSH_MSG_USERAUTH_GSSAPI_MIC` / 66

```text
byte            SSH_MSG_USERAUTH_GSSAPI_MIC
string[bytes]   MIC
```

> 62 在该方法中未使用。

## 4.11 `gssapi-keyex` 认证方法（RFC 4462）

```text
byte            SSH_MSG_USERAUTH_REQUEST
string[utf8]    user name
string[ascii]   service name
string[ascii]   "gssapi-keyex"
string[bytes]   MIC
```

## 4.12 `publickey-hostbound-v00@openssh.com` 认证方法 **[OpenSSH]**

主机绑定公钥认证，与标准 `publickey` 方法类似，但增加了一个 `server host key` 字段，将认证绑定到特定的服务器主机密钥和会话标识符。

### 初始查询 / 50

```text
byte            SSH_MSG_USERAUTH_REQUEST
string[utf8]    user name
string[ascii]   service name
string[ascii]   "publickey-hostbound-v00@openssh.com"
boolean         FALSE
string[ascii]   public key algorithm name
string[bytes]   public key blob
string[bytes]   server host key
```

### 签名请求 / 50

```text
byte            SSH_MSG_USERAUTH_REQUEST
string[utf8]    user name
string[ascii]   service name
string[ascii]   "publickey-hostbound-v00@openssh.com"
boolean         TRUE
string[ascii]   public key algorithm name
string[bytes]   public key blob
string[bytes]   server host key
string[bytes]   signature
```

> 注：由于整个 `SSH_MSG_USERAUTH_REQUEST` 消息包含在签名数据中，这确保了签名者可以看到目标用户、服务器身份和会话标识符之间的绑定。OpenSSH 通过 ssh-agent 使用此绑定实现 per-key 限制。
>
> 服务器可通过 `SSH2_MSG_EXT_INFO` 机制（RFC 8308）广告此方法，扩展名 `"publickey-hostbound@openssh.com"`，版本 `"0"`。

---

# 5. Connection Protocol：全局请求、Channel、数据与请求

## 5.1 `SSH_MSG_GLOBAL_REQUEST` / 80

```text
byte            SSH_MSG_GLOBAL_REQUEST
string[ascii]   request name
boolean         want reply
...             request-specific data
```

## 5.2 `SSH_MSG_REQUEST_SUCCESS` / 81

```text
byte      SSH_MSG_REQUEST_SUCCESS
...       response-specific data
```

## 5.3 `SSH_MSG_REQUEST_FAILURE` / 82

```text
byte      SSH_MSG_REQUEST_FAILURE
```

## 5.4 全局请求：`tcpip-forward`

```text
byte            SSH_MSG_GLOBAL_REQUEST
string[ascii]   "tcpip-forward"
boolean         want reply
string[ascii]   address to bind
uint32          port number to bind
```

若端口为 `0` 且成功，响应：

```text
byte      SSH_MSG_REQUEST_SUCCESS
uint32    port that was bound on the server
```

## 5.5 全局请求：`cancel-tcpip-forward`

```text
byte            SSH_MSG_GLOBAL_REQUEST
string[ascii]   "cancel-tcpip-forward"
boolean         want reply
string[ascii]   address to bind
uint32          port number to bind
```

## 5.6 全局请求：`streamlocal-forward@openssh.com` **[OpenSSH]**

Unix 域套接字远程转发请求，类似于 `tcpip-forward`，但用于 Unix 域套接字。

```text
byte            SSH_MSG_GLOBAL_REQUEST
string[ascii]   "streamlocal-forward@openssh.com"
boolean         TRUE
string[bytes]   socket path
```

## 5.7 全局请求：`cancel-streamlocal-forward@openssh.com` **[OpenSSH]**

取消 Unix 域套接字远程转发，类似于 `cancel-tcpip-forward`。

```text
byte            SSH_MSG_GLOBAL_REQUEST
string[ascii]   "cancel-streamlocal-forward@openssh.com"
boolean         FALSE
string[bytes]   socket path
```

## 5.8 全局请求：`no-more-sessions@openssh.com` **[OpenSSH]**

客户端声明不再请求 session，服务器将拒绝未来的 `session` channel 打开请求（缓解会话劫持攻击）。

```text
byte            SSH_MSG_GLOBAL_REQUEST
string[ascii]   "no-more-sessions@openssh.com"
boolean         want reply
```

> 注：仅发送给 OpenSSH 服务器（通过 banner 识别）。当客户端禁用连接多路复用时发送此请求。

## 5.9 全局请求：`keepalive@openssh.com` **[OpenSSH]**

连接保活请求，防止连接超时。

```text
byte            SSH_MSG_GLOBAL_REQUEST
string[ascii]   "keepalive@openssh.com"
boolean         want reply
```

## 5.10 全局请求：`hostkeys-00@openssh.com` **[OpenSSH]**

服务器在用户认证完成后通知客户端其所有协议 v2 主机密钥（用于主机密钥轮换）。

```text
byte            SSH_MSG_GLOBAL_REQUEST
string[ascii]   "hostkeys-00@openssh.com"
boolean         FALSE
string[bytes]   host key blob[1]
...
string[bytes]   host key blob[n]
```

## 5.11 全局请求：`hostkeys-prove-00@openssh.com` **[OpenSSH]**

客户端要求服务器证明其拥有的主机密钥（与 `hostkeys-00@openssh.com` 配合使用）。

```text
byte            SSH_MSG_GLOBAL_REQUEST
string[ascii]   "hostkeys-prove-00@openssh.com"
boolean         TRUE
string[bytes]   host key blob to prove[1]
...
string[bytes]   host key blob to prove[n]
```

## 5.12 `SSH_MSG_CHANNEL_OPEN` / 90

```text
byte            SSH_MSG_CHANNEL_OPEN
string[ascii]   channel type
uint32          sender channel
uint32          initial window size
uint32          maximum packet size
...             channel-type-specific data
```

## 5.13 `SSH_MSG_CHANNEL_OPEN_CONFIRMATION` / 91

```text
byte      SSH_MSG_CHANNEL_OPEN_CONFIRMATION
uint32    recipient channel
uint32    sender channel
uint32    initial window size
uint32    maximum packet size
...       channel-type-specific data
```

## 5.14 `SSH_MSG_CHANNEL_OPEN_FAILURE` / 92

```text
byte             SSH_MSG_CHANNEL_OPEN_FAILURE
uint32           recipient channel
uint32           reason code
string[utf8]     description
string[langtag]  language tag
```

常见 reason code：`ADMINISTRATIVELY_PROHIBITED`=1、`CONNECT_FAILED`=2、`UNKNOWN_CHANNEL_TYPE`=3、`RESOURCE_SHORTAGE`=4。

## 5.15 `SSH_MSG_CHANNEL_WINDOW_ADJUST` / 93

```text
byte      SSH_MSG_CHANNEL_WINDOW_ADJUST
uint32    recipient channel
uint32    bytes to add
```

## 5.16 `SSH_MSG_CHANNEL_DATA` / 94

```text
byte            SSH_MSG_CHANNEL_DATA
uint32          recipient channel
string[bytes]   data
```

## 5.17 `SSH_MSG_CHANNEL_EXTENDED_DATA` / 95

```text
byte            SSH_MSG_CHANNEL_EXTENDED_DATA
uint32          recipient channel
uint32          data_type_code
string[bytes]   data
```

`data_type_code = 1` 表示 `SSH_EXTENDED_DATA_STDERR`。

## 5.18 `SSH_MSG_CHANNEL_EOF` / 96

```text
byte      SSH_MSG_CHANNEL_EOF
uint32    recipient channel
```

## 5.19 `SSH_MSG_CHANNEL_CLOSE` / 97

```text
byte      SSH_MSG_CHANNEL_CLOSE
uint32    recipient channel
```

## 5.20 `SSH_MSG_CHANNEL_REQUEST` / 98

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   request type
boolean         want reply
...             type-specific data follows
```

## 5.21 `SSH_MSG_CHANNEL_SUCCESS` / 99

```text
byte      SSH_MSG_CHANNEL_SUCCESS
uint32    recipient channel
```

## 5.22 `SSH_MSG_CHANNEL_FAILURE` / 100

```text
byte      SSH_MSG_CHANNEL_FAILURE
uint32    recipient channel
```

---

# 6. Connection Protocol：标准 Channel Types

## 6.1 `session`

```text
byte            SSH_MSG_CHANNEL_OPEN
string[ascii]   "session"
uint32          sender channel
uint32          initial window size
uint32          maximum packet size
```

## 6.2 `x11`

```text
byte            SSH_MSG_CHANNEL_OPEN
string[ascii]   "x11"
uint32          sender channel
uint32          initial window size
uint32          maximum packet size
string[ascii]   originator address
uint32          originator port
```

## 6.3 `forwarded-tcpip`

```text
byte            SSH_MSG_CHANNEL_OPEN
string[ascii]   "forwarded-tcpip"
uint32          sender channel
uint32          initial window size
uint32          maximum packet size
string[ascii]   address that was connected
uint32          port that was connected
string[ascii]   originator IP address
uint32          originator port
```

## 6.4 `direct-tcpip`

```text
byte            SSH_MSG_CHANNEL_OPEN
string[ascii]   "direct-tcpip"
uint32          sender channel
uint32          initial window size
uint32          maximum packet size
string[ascii]   host to connect
uint32          port to connect
string[ascii]   originator IP address
uint32          originator port
```

## 6.5 `agent-connect`（RFC 9987）

```text
byte            SSH_MSG_CHANNEL_OPEN
string[ascii]   "agent-connect" or "auth-agent@openssh.com"
uint32          channel_id
uint32          local_window
uint32          local_maxpacket
```

## 6.6 `direct-streamlocal@openssh.com` **[OpenSSH]**

Unix 域套接字本地转发，类似于 `direct-tcpip`，但用于 Unix 域套接字。

```text
byte            SSH_MSG_CHANNEL_OPEN
string[ascii]   "direct-streamlocal@openssh.com"
uint32          sender channel
uint32          initial window size
uint32          maximum packet size
string[bytes]   socket path
string[bytes]   reserved
uint32          reserved
```

## 6.7 `forwarded-streamlocal@openssh.com` **[OpenSSH]**

Unix 域套接字远程转发，类似于 `forwarded-tcpip`，当客户端先前发送了 `streamlocal-forward@openssh.com` 全局请求时由服务器发送。

```text
byte            SSH_MSG_CHANNEL_OPEN
string[ascii]   "forwarded-streamlocal@openssh.com"
uint32          sender channel
uint32          initial window size
uint32          maximum packet size
string[bytes]   socket path
string[bytes]   reserved for future use
```

> 注：`reserved` 字段当前未定义，远程端忽略。客户端当前发送空字符串。未来可能用于传递套接字文件信息（如所有权和权限）。

## 6.8 `tun@openssh.com` **[OpenSSH]**

Layer 2（以太网）和 Layer 3（IP）隧道转发，通过 tun(4) 设备在端点之间转发网络包，保持数据报边界完整。

```text
byte            SSH_MSG_CHANNEL_OPEN
string[ascii]   "tun@openssh.com"
uint32          sender channel
uint32          initial window size
uint32          maximum packet size
uint32          tunnel mode        (1=point-to-point/L3, 2=ethernet/L2)
uint32          remote unit number (0x7fffffff = auto)
```

**tunnel mode 常量**：
- `SSH_TUNMODE_POINTOPOINT` = 1（Layer 3 数据包）
- `SSH_TUNMODE_ETHERNET` = 2（Layer 2 帧）

**Layer 3 数据封装**（通过 `SSH_MSG_CHANNEL_DATA` 发送）：
```text
uint32    packet length
uint32    address family     (2=IPv4, 24=IPv6)
byte[]    packet data
```

**Layer 2 帧封装**（通过 `SSH_MSG_CHANNEL_DATA` 发送）：
```text
uint32    packet length
byte[]    IEEE 802.3 Ethernet frame (包含头部)
```

---

# 7. Connection Protocol：标准 Channel Requests

## 7.1 `pty-req`

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "pty-req"
boolean         want_reply
string[bytes]   TERM environment variable value
uint32          terminal width, characters
uint32          terminal height, rows
uint32          terminal width, pixels
uint32          terminal height, pixels
string[bytes]   encoded terminal modes
```

## 7.2 `x11-req`

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "x11-req"
boolean         want reply
boolean         single connection
string[ascii]   x11 authentication protocol
string[bytes]   x11 authentication cookie
uint32          x11 screen number
```

## 7.3 `env`

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "env"
boolean         want reply
string[bytes]   variable name
string[bytes]   variable value
```

## 7.4 `shell`

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "shell"
boolean         want reply
```

## 7.5 `exec`

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "exec"
boolean         want reply
string[bytes]   command
```

## 7.6 `subsystem`

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "subsystem"
boolean         want reply
string[ascii]   subsystem name
```

## 7.7 `window-change`

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "window-change"
boolean         FALSE
uint32          terminal width, columns
uint32          terminal height, rows
uint32          terminal width, pixels
uint32          terminal height, pixels
```

## 7.8 `xon-xoff`

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "xon-xoff"
boolean         FALSE
boolean         client can do
```

## 7.9 `signal`

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "signal"
boolean         FALSE
string[ascii]   signal name without the "SIG" prefix
```

## 7.10 `exit-status`

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "exit-status"
boolean         FALSE
uint32          exit_status
```

## 7.11 `exit-signal`

```text
byte             SSH_MSG_CHANNEL_REQUEST
uint32           recipient channel
string[ascii]    "exit-signal"
boolean          FALSE
string[ascii]    signal name without the "SIG" prefix
boolean          core dumped
string[utf8]     error message
string[langtag]  language tag
```

## 7.12 `break`（RFC 4335）

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "break"
boolean         want_reply
uint32          break-length in milliseconds
```

## 7.13 `agent-req`（RFC 9987）

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          channel_id
string[ascii]   "agent-req" or "auth-agent-req@openssh.com"
boolean         want_reply
```

## 7.14 `eow@openssh.com`（End Of Write）**[OpenSSH]**

通知对端本地输出已关闭或发生写入错误，请求对端停止发送数据，同时保持 channel 开放以便反向数据传输。

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "eow@openssh.com"
boolean         FALSE
```

> 注：
> - 仅发送给 OpenSSH 对端（通过 banner 识别），因为某些 SSH 实现在收到此消息时会违反 RFC 4254 section 5.4 而中止连接。
> - 此消息不消耗窗口空间，即使没有可用窗口空间也可发送。
> - 与 `SSH_MSG_CHANNEL_EOF` 类似，发送 `eow@openssh.com` 后 channel 保持开放，仍可反向发送数据。

## 7.15 `INFO@openssh.com`（信号扩展）**[OpenSSH]**

OpenSSH 支持的扩展信号名称，允许在 BSD 系统上发送 SIGINFO 信号。使用标准的 `signal` channel request，但信号名称为 `INFO`。

```text
byte            SSH_MSG_CHANNEL_REQUEST
uint32          recipient channel
string[ascii]   "signal"
boolean         FALSE
string[ascii]   "INFO"
```

## 7.16 `SIG@openssh.com`（未知信号回退）**[OpenSSH]**

当进程因无法识别的信号退出时，OpenSSH 在 `exit-signal` channel request 中使用 `"SIG@openssh.com"` 作为信号名回退。

```text
byte             SSH_MSG_CHANNEL_REQUEST
uint32           recipient channel
string[ascii]    "exit-signal"
boolean          FALSE
string[ascii]    "SIG@openssh.com"
boolean          core dumped
string[utf8]     error message
string[langtag]  language tag
```

> 注：OpenSSH 的 `sig2name()` 函数在信号编号无法匹配 ABRT、ALRM、FPE、HUP、ILL、INT、KILL、PIPE、QUIT、SEGV、TERM、USR1、USR2 中的任何一个时，返回 `"SIG@openssh.com"`。

---

# 8. 保留/未分配/私有范围

|    值/范围 | 状态                             |
| ------: | ------------------------------ |
|       0 | Reserved                       |
|    9–19 | Unassigned                     |
|   22–29 | Unassigned                     |
|   54–59 | Unassigned                     |
|   83–89 | Unassigned                     |
| 101–127 | Unassigned                     |
| 128–191 | Reserved for client protocols  |
| 192–255 | Local extensions / Private Use |

## 8.1 同一数字为何有多个名字？

示例：`30` 在固定组 DH 中是 `SSH_MSG_KEXDH_INIT`，在 DH GEX 旧请求中是 `SSH_MSG_KEX_DH_GEX_REQUEST_OLD`，在 RSA KEX 中是 `SSH_MSG_KEXRSA_PUBKEY`，在 ECDH 中是 `SSH_MSG_KEX_ECDH_INIT`，在 GSS KEX 中是 `SSH_MSG_KEXGSS_INIT`。这是 SSH 允许 method-specific 号段按当前协商上下文解释的结果。

认证方法类似：`60` 可表示 `SSH_MSG_USERAUTH_PK_OK`、`SSH_MSG_USERAUTH_PASSWD_CHANGEREQ`、`SSH_MSG_USERAUTH_INFO_REQUEST` 或 `SSH_MSG_USERAUTH_GSSAPI_RESPONSE`。

## 8.2 OpenSSH 特有扩展总结 **[OpenSSH]**

### 传输层扩展

| 扩展名称 | 说明 |
|----------|------|
| `SSH2_MSG_PING` (192) | 传输层 ping 请求 |
| `SSH2_MSG_PONG` (193) | 传输层 ping 响应 |
| `ping@openssh.com` | 通过 `SSH_MSG_EXT_INFO` 广告的 ping 支持 |
| `server-sig-algs` | 通过 `SSH_MSG_EXT_INFO` 广告的服务器签名算法列表 |
| `agent-forward` | 通过 `SSH_MSG_EXT_INFO` 广告的 RFC 9987 agent 转发支持 |
| `kex-strict-c-v00@openssh.com` / `kex-strict-s-v00@openssh.com` | Terrapin 攻击防护的严格 KEX 硬化 |
| `ext-info-in-auth@openssh.com` | 允许在用户认证期间发送 `SSH_MSG_EXT_INFO` |
| `zlib@openssh.com` | 延迟压缩，认证完成后才启用 zlib 压缩 |
| `mlkem768x25519-sha256` | ML-KEM 768 + X25519 后量子混合 KEX |
| `sntrup761x25519-sha512@openssh.com` | sntrup761 + X25519 后量子混合 KEX（旧名） |
| `chacha20-poly1305@openssh.com` | ChaCha20-Poly1305 认证加密算法 |
| `aes128-gcm@openssh.com` / `aes256-gcm@openssh.com` | AES-GCM 认证加密算法 |
| `hmac-sha1-etm@openssh.com` | Encrypt-then-MAC HMAC-SHA1 |
| `hmac-sha1-96-etm@openssh.com` | Encrypt-then-MAC HMAC-SHA1-96 |
| `hmac-sha2-256-etm@openssh.com` | Encrypt-then-MAC HMAC-SHA2-256 |
| `hmac-sha2-512-etm@openssh.com` | Encrypt-then-MAC HMAC-SHA2-512 |
| `hmac-md5-etm@openssh.com` | Encrypt-then-MAC HMAC-MD5 |
| `hmac-md5-96-etm@openssh.com` | Encrypt-then-MAC HMAC-MD5-96 |
| `umac-64-etm@openssh.com` | Encrypt-then-MAC UMAC-64 |
| `umac-128-etm@openssh.com` | Encrypt-then-MAC UMAC-128 |
| `umac-64@openssh.com` | UMAC-64 MAC 算法 |
| `umac-128@openssh.com` | UMAC-128 MAC 算法 |

### 认证层扩展

| 扩展名称 | 说明 |
|----------|------|
| `publickey-hostbound-v00@openssh.com` | 主机绑定公钥认证方法 |
| `publickey-hostbound@openssh.com` | 通过 `SSH_MSG_EXT_INFO` 广告的主机绑定公钥支持 |

### 连接层扩展 - 全局请求

| 扩展名称 | 说明 |
|----------|------|
| `streamlocal-forward@openssh.com` | Unix 域套接字远程转发请求 |
| `cancel-streamlocal-forward@openssh.com` | 取消 Unix 域套接字远程转发 |
| `no-more-sessions@openssh.com` | 客户端声明不再请求 session |
| `keepalive@openssh.com` | 连接保活请求 |
| `hostkeys-00@openssh.com` | 服务器通知客户端所有主机密钥 |
| `hostkeys-prove-00@openssh.com` | 客户端要求服务器证明拥有主机密钥 |

### 连接层扩展 - Channel Types

| 扩展名称 | 说明 |
|----------|------|
| `direct-streamlocal@openssh.com` | Unix 域套接字本地转发 |
| `forwarded-streamlocal@openssh.com` | Unix 域套接字远程转发 |
| `tun@openssh.com` | Layer 2/3 隧道转发 |
| `auth-agent@openssh.com` | SSH agent 转发（旧名称） |

### 连接层扩展 - Channel Requests

| 扩展名称 | 说明 |
|----------|------|
| `eow@openssh.com` | End Of Write，通知对端停止发送数据 |
| `INFO@openssh.com` | SIGINFO 信号扩展（BSD 系统） |
| `SIG@openssh.com` | 未知信号回退名（exit-signal 中使用） |
| `auth-agent-req@openssh.com` | 请求 agent 转发（旧名称） |

---

# 9. 参考 RFC / 注册表

## IETF RFC

* RFC 4250 — The Secure Shell (SSH) Protocol Assigned Numbers
* RFC 4251 — The Secure Shell (SSH) Protocol Architecture
* RFC 4252 — The Secure Shell (SSH) Authentication Protocol
* RFC 4253 — The Secure Shell (SSH) Transport Layer Protocol
* RFC 4254 — The Secure Shell (SSH) Connection Protocol
* RFC 4256 — Generic Message Exchange Authentication for SSH
* RFC 4335 — SSH Session Channel Break Extension
* RFC 4419 — Diffie-Hellman Group Exchange for SSH
* RFC 4432 — RSA Key Exchange for SSH
* RFC 4462 — GSS-API Authentication and Key Exchange for SSH
* RFC 5656 — Elliptic Curve Algorithm Integration in SSH
* RFC 8308 — Extension Negotiation in SSH
* RFC 8731 — Curve25519 and Curve448 KEX for SSH
* RFC 9941 — Hybrid sntrup761 and X25519 KEX for SSH
* RFC 9987 — SSH Agent Protocol
* IANA — Secure Shell (SSH) Protocol Parameters

## OpenSSH 文档

* OpenSSH PROTOCOL — OpenSSH 协议扩展与偏差
* OpenSSH PROTOCOL.agent — OpenSSH Agent 协议扩展
* OpenSSH PROTOCOL.mux — OpenSSH 连接多路复用协议
* OpenSSH PROTOCOL.u2f — OpenSSH FIDO/U2F 安全密钥支持
* OpenSSH PROTOCOL.key — OpenSSH 私钥格式
* OpenSSH PROTOCOL.krl — OpenSSH 密钥撤销列表格式

---

# 10. UTF-8 字段编码说明

## 10.1 明确为 UTF-8 的 `string` 字段

以下字段按 RFC 明确要求为 ISO-10646 UTF-8 编码：

### Transport Layer

| 消息 | 字段 | 说明 |
|------|------|------|
| `SSH_MSG_DISCONNECT` | `description` | 断开连接描述 |
| `SSH_MSG_DEBUG` | `message` | 调试消息 |

### User Authentication

| 消息 | 字段 | 说明 |
|------|------|------|
| `SSH_MSG_USERAUTH_REQUEST` | `user name` | 用户名（所有认证方法通用） |
| `SSH_MSG_USERAUTH_BANNER` | `message` | 认证横幅消息 |
| `SSH_MSG_USERAUTH_PASSWD_CHANGEREQ` | `prompt` | 密码更改提示 |
| `SSH_MSG_USERAUTH_INFO_REQUEST` | `name`, `instruction`, `prompt[i]` | 键盘交互提示 |
| `SSH_MSG_USERAUTH_INFO_RESPONSE` | `response[i]` | 键盘交互响应 |
| `SSH_MSG_USERAUTH_GSSAPI_ERROR` | `message` | GSS-API 错误消息 |
| `password` 认证 | `plaintext password` | 明文密码 |
| `password` 更改 | `plaintext old/new password` | 旧/新密码 |
| `hostbased` 认证 | `user name on the client host` | 客户端主机上的用户名 |
| `keyboard-interactive` | `submethods` | 子方法列表 |

### Connection Protocol

| 消息 | 字段 | 说明 |
|------|------|------|
| `SSH_MSG_CHANNEL_OPEN_FAILURE` | `description` | Channel 打开失败描述 |
| `exit-signal` 请求 | `error message` | 退出信号错误消息 |

### GSS-API Key Exchange

| 消息 | 字段 | 说明 |
|------|------|------|
| `SSH_MSG_KEXGSS_ERROR` | `message` | GSS KEX 错误消息 |

## 10.2 明确不是 UTF-8 文本的常见 `string` 字段

以下字段虽然类型也是 `string`，但不应直接按 UTF-8 文本处理：

| 字段类型/位置 | 建议标注 | 说明 |
|--------------|----------|------|
| `payload` | `bytes` | SSH binary packet payload |
| `SSH_MSG_IGNORE.data` | `string[bytes]` | 任意忽略数据 |
| `service name` | `string[ascii]` | 如 `ssh-userauth`、`ssh-connection` |
| `method name` | `string[ascii]` | 如 `publickey`、`password` |
| algorithm names | `string[ascii]` / `name-list[ascii]` | 算法标识符 |
| request name / request type | `string[ascii]` | 如 `tcpip-forward`、`pty-req`、`exec` |
| channel type | `string[ascii]` | 如 `session`、`x11`、`direct-tcpip` |
| language tag | `string[langtag]` | RFC language tag，不是普通用户文本 |
| public key blob | `string[bytes]` | 公钥/证书二进制编码 |
| signature | `string[bytes]` | 签名字节 |
| GSS token / MIC / OID | `string[bytes]` | GSS-API/ASN.1/DER 或 MIC 数据 |
| `SSH_MSG_CHANNEL_DATA.data` | `string[bytes]` | channel 原始数据；编码由上层程序/终端决定 |
| `SSH_MSG_CHANNEL_EXTENDED_DATA.data` | `string[bytes]` | stderr 等扩展数据，也是原始字节 |
| `exec.command` | `string[bytes]` 或实现相关 | RFC 4254 未明确规定 UTF-8 |
| `env.variable name/value` | `string[bytes]` 或实现相关 | RFC 4254 未明确规定 UTF-8 |
| `pty-req` 的 TERM value | `string[bytes]` 或 ASCII-ish | RFC 4254 未明确规定 UTF-8 |
| `subsystem name` | `string[ascii]` | 注册名/协议名，不是用户文本 |
| TCP/IP forwarding addresses | `string[bytes]` / ASCII-ish | IP 地址或域名字符串，RFC 未标为 UTF-8 |

## 10.3 server banner line

这不是 SSH binary packet 内的 `string` 字段，但 RFC 4253 提到：服务器在发送 `SSH-...` identification string 之前额外发送的文本行 SHOULD 使用 ISO-10646 UTF-8。

```text
pre-identification line: UTF-8 text, CR LF terminated
```
