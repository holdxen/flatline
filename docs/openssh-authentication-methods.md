# OpenSSH 登录认证方式（Authentication Methods）

> 基于 OpenSSH Portable 源码分析，涵盖 SSH 协议 v2 支持的全部登录认证方式、客户端/服务端配置与工作原理。

---

## 目录

1. [全部支持的认证方式](#1-全部支持的认证方式)
2. [客户端默认尝试顺序](#2-客户端默认尝试顺序)
3. [各认证方式详解](#3-各认证方式详解)
4. [服务端配置参考](#4-服务端配置参考)
5. [客户端配置参考](#5-客户端配置参考)
6. [多因素认证（AuthenticationMethods）](#6-多因素认证authenticationmethods)
7. [相关源码文件](#7-相关源码文件)

---

## 1. 全部支持的认证方式

OpenSSH 在 `auth2-methods.c` 中注册了所有已实现的认证方式，共 **6 种**（含 1 种编译时可选）：

| # | 方法名 | 说明 | 服务端配置项 | 客户端配置项 |
|---|--------|------|-------------|-------------|
| 1 | `publickey` | 公钥认证（RSA/ECDSA/Ed25519/证书等） | `PubkeyAuthentication` | `PubkeyAuthentication` |
| 2 | `password` | 明文密码认证（通过 SSH 加密隧道传输） | `PasswordAuthentication` | `PasswordAuthentication` |
| 3 | `keyboard-interactive` | 键盘交互认证（服务端发起提示，客户端回显） | `KbdInteractiveAuthentication` | `KbdInteractiveAuthentication` |
| 4 | `hostbased` | 基于主机信任关系的认证 | `HostbasedAuthentication` | `HostbasedAuthentication` |
| 5 | `gssapi-with-mic` | Kerberos/GSSAPI 认证（需编译时 `-DGSSAPI`） | `GSSAPIAuthentication` | `GSSAPIAuthentication` |
| 6 | `none` | 无认证（空密码，服务端特殊配置时才允许） | `PermitEmptyPasswords` | — |

> `publickey` 方式有一个内部子变体 `publickey-hostbound-v00@openssh.com`，用于 agent-forwarding 场景下的主机绑定认证。

---

## 2. 客户端默认尝试顺序

客户端在 `sshconnect2.c` 的 `authmethods[]` 数组中定义了默认尝试顺序：

```
gssapi-with-mic → hostbased → publickey → keyboard-interactive → password → none
```

可通过客户端配置文件 `ssh_config` 中的 `PreferredAuthentications` 指令覆盖此顺序，例如：

```
PreferredAuthentications publickey,password
```

---

## 3. 各认证方式详解

### 3.1 publickey — 公钥认证

**源文件**：`auth2-pubkey.c`

**工作原理**：

1. 客户端将本地私钥对应的**公钥**发送给服务端
2. 服务端检查该公钥是否存在于目标用户的 `~/.ssh/authorized_keys` 文件中
3. 服务端发送一个**随机挑战数据（challenge）**
4. 客户端使用本地**私钥对挑战数据签名**，将签名发回
5. 服务端用公钥验证签名，验证通过则认证成功

**支持的密钥类型**：RSA、ECDSA（P-256/P-384/P-521）、Ed25519、DSA（已废弃）、以及所有类型的 OpenSSH 证书（`-cert-v01@openssh.com`）。

**特点**：
- 生产环境**首选认证方式**，私钥永不落网
- 支持 `~/.ssh/authorized_keys` 文件级别的细粒度授权控制（`from=`、`command=`、`environment=` 等选项）
- 支持 OpenSSH 证书（CA 签发），适用于大规模主机管理

**配置**：
```
# sshd_config
PubkeyAuthentication yes
AuthorizedKeysFile .ssh/authorized_keys
```

---

### 3.2 password — 密码认证

**源文件**：`auth2-passwd.c`

**工作原理**：

1. 客户端将用户输入的密码（经 SSH 加密隧道）发送给服务端
2. 服务端调用系统密码验证接口（如 `pam_authenticate()` 或 `crypt()`）进行验证
3. 验证通过则认证成功

**特点**：
- 最直观、最简单的认证方式
- 密码在网络上传输时已被 SSH 传输层加密，但**服务端需要接收明文密码**（与公钥认证相比，安全性略低）
- 易受暴力破解攻击，生产环境通常建议禁用
- 支持密码修改（通过 `SSH2_MSG_USERAUTH_PASSWD_CHANGEREQ` 消息）

**配置**：
```
# sshd_config
PasswordAuthentication yes
PermitEmptyPasswords no    # 是否允许空密码
```

---

### 3.3 keyboard-interactive — 键盘交互认证

**源文件**：`auth2-kbdint.c`

**工作原理**：

1. 客户端发送认证请求
2. 服务端发送一个或多个**文本提示（prompt）**，指示用户输入信息
3. 客户端显示提示，将用户输入回传给服务端
4. 服务端验证响应，可多轮交互（challenge-response）

**特点**：
- 是一种**通用框架**，服务端可对接任意认证后端
- 常见后端实现：
  - **PAM**（Pluggable Authentication Modules）— 对接系统 PAM 模块
  - **BSD auth**（`auth-bsdauth.c`）— OpenBSD 原生认证
  - **SIA**（`auth-sia.c`）— Tru64 安全集成架构（历史遗留）
- 支持**多轮对话**，适合 OTP（一次性密码）、双因素认证等场景
- 客户端在 `BatchMode` 下不会尝试此方法（因需要用户交互）

**配置**：
```
# sshd_config
KbdInteractiveAuthentication yes
# 若使用 PAM，还需：
UsePAM yes
```

---

### 3.4 hostbased — 基于主机的认证

**源文件**：`auth2-hostbased.c`

**工作原理**：

1. 客户端以**本地主机身份**（使用 `/etc/ssh/ssh_host_*_key`）向服务端证明"我来自可信主机"
2. 服务端检查客户端主机名/IP 是否存在于信任列表中（`/etc/ssh/shosts.equiv`、`~/.shosts`）
3. 验证通过则允许客户端主机上的**任意用户**以目标用户身份登录

**特点**：
- 信任粒度是**主机级别**，而非用户级别（某主机被信任后，该主机上的所有用户均可登录）
- 安全性较低，仅在高度受控的内网中使用
- 需要客户端主机的私钥签名，防止 IP/主机名欺骗
- 历史上对应 rsh/rlogin 的信任机制，但加上了密码学保护

**配置**：
```
# sshd_config
HostbasedAuthentication yes
# 信任列表文件：/etc/ssh/shosts.equiv 或 ~/.shosts
```

---

### 3.5 gssapi-with-mic — Kerberos/GSSAPI 认证

**源文件**：`auth2-gss.c`

**前提**：编译时需启用 GSSAPI 支持（`-DGSSAPI`），运行时需有可用的 Kerberos KDC。

**工作原理**：

1. 客户端使用已获取的 **Kerberos TGT**（票据授权票据）向服务端请求服务票据
2. 客户端将服务票据发送给服务端
3. 服务端验证票据，并验证消息完整性码（MIC，Message Integrity Code）
4. 验证通过则认证成功

**特点**：
- **单点登录（SSO）**：用户 `kinit` 一次后，即可免密登录所有加入 Kerberos 域的主机
- 适合企业/学术机构的大规模统一认证环境
- 依赖外部基础设施（KDC），单机部署不适用
- 支持用户身份映射（通过 `.k5login` 文件）

**配置**：
```
# sshd_config
GSSAPIAuthentication yes
GSSAPICleanupCredentials yes
```

---

### 3.6 none — 无认证

**源文件**：`auth2-none.c`

**工作原理**：

服务端收到 `none` 认证请求后，若同时满足以下条件则允许登录：
- `PermitEmptyPasswords yes`（允许空密码）
- `PasswordAuthentication yes`（密码认证已启用）
- 目标用户密码确实为空

**特点**：
- **极度危险**，仅用于特殊测试或隔离环境
- 服务端 `none_enabled` 标志只允许使用一次（每次连接仅第一次 `none` 请求有效）
- 客户端不会主动优先尝试（在默认顺序中排最后）

---

## 4. 服务端配置参考

`sshd_config` 中与认证相关的主要配置项：

```
# 启用/禁用各认证方式
PubkeyAuthentication yes              # 默认 yes
PasswordAuthentication yes            # 默认 yes
KbdInteractiveAuthentication yes      # 默认 yes
HostbasedAuthentication no            # 默认 no
GSSAPIAuthentication no               # 默认 no

# 密码相关
PermitEmptyPasswords no               # 默认 no，禁止空密码登录
PermitRootLogin prohibit-password     # root 登录策略

# 公钥相关
AuthorizedKeysFile .ssh/authorized_keys
AuthorizedPrincipalsFile none         # 证书认证时使用的 principals 文件

# 最大认证尝试次数
MaxAuthTries 6                        # 默认 6，超过后断开连接

# 多因素认证（组合要求）
AuthenticationMethods publickey,password
```

---

## 5. 客户端配置参考

`ssh_config` 中与认证相关的主要配置项：

```
# 指定认证方式及顺序（覆盖默认）
PreferredAuthentications publickey,keyboard-interactive,password

# 启用/禁用各认证方式
PubkeyAuthentication yes
PasswordAuthentication yes
KbdInteractiveAuthentication yes
HostbasedAuthentication no
GSSAPIAuthentication no

# 批处理模式（抑制交互提示，跳过 keyboard-interactive/password）
BatchMode no

# 指定私钥文件
IdentityFile ~/.ssh/id_ed25519
```

---

## 6. 多因素认证（AuthenticationMethods）

`sshd_config` 支持 `AuthenticationMethods` 指令，可要求用户通过**多种认证方式的组合**才能登录：

```
# 要求先通过公钥认证，再通过密码认证
AuthenticationMethods publickey,password

# 公钥认证 OR 密码认证（任一即可）
AuthenticationMethods publickey password

# 公钥认证 + (密码 OR 键盘交互)
AuthenticationMethods publickey,password publickey,keyboard-interactive
```

多因素认证中，方法之间用逗号分隔表示"且"（AND），空格分隔表示"或"（OR）。

---

## 7. 相关源码文件

| 文件 | 职责 |
|------|------|
| `auth2.c` | 服务端认证主调度器，注册所有 `authmethods[]`，处理 `SSH2_MSG_USERAUTH_REQUEST` |
| `auth2-methods.c` | 服务端认证方法配置注册，定义 `authmethod_cfg` 结构体 |
| `auth2-pubkey.c` | 服务端公钥认证实现 |
| `auth2-passwd.c` | 服务端密码认证实现 |
| `auth2-kbdint.c` | 服务端键盘交互认证实现 |
| `auth2-hostbased.c` | 服务端主机认证实现 |
| `auth2-gss.c` | 服务端 GSSAPI/Kerberos 认证实现 |
| `auth2-none.c` | 服务端无认证方式实现 |
| `auth-pam.c` | PAM 认证后端（供 keyboard-interactive 调用） |
| `auth-bsdauth.c` | BSD auth 认证后端（供 keyboard-interactive 调用） |
| `sshconnect2.c` | 客户端认证调度器，定义客户端 `authmethods[]` 及尝试顺序 |
| `authfd.c` | SSH agent 通信（公钥认证时获取私钥签名） |
| `authfile.c` | 私钥文件读取与解密 |
