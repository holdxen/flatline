//! SSH-2 协议常量定义
//!
//! 依据 RFC 4250 / 4252 / 4253 / 4254 整理，
//! 并含 RFC 4256（keyboard-interactive）与 RFC 4419（DH 群交换）的方法专用消息。
//!
//! 约定：
//! - 消息编号（message number）是 1..=255 的字节值，类型为 `u8`。
//! - 断开原因码 / 通道打开失败原因码 / 扩展数据类型码均为 `uint32`，类型为 `u32`。
//! - 编号 30..=49 与 60..=79 为“方法专用”，可被不同方法复用，故存在数值重叠
//!   （常量名各不相同，可正常编译）。

#![allow(dead_code)]

#[derive(Clone, Default, Debug, Copy, PartialEq, Eq)]
pub struct SFTPExtension {
    pub key: &'static str,
    pub value: &'static [u8],
}

impl SFTPExtension {
    pub const fn new(key: &'static str, value: &'static [u8]) -> Self {
        Self { key, value }
    }
}

// =============================================================================
// 消息编号 —— 传输层协议通用消息 (RFC 4253, 1..=6)
// =============================================================================

/// 断开连接（RFC 4253）
pub const SSH_MSG_DISCONNECT: u8 = 1;
/// 可忽略消息（RFC 4253）
pub const SSH_MSG_IGNORE: u8 = 2;
/// 未实现/无法识别的消息编号（RFC 4253）
pub const SSH_MSG_UNIMPLEMENTED: u8 = 3;
/// 调试信息（RFC 4253）
pub const SSH_MSG_DEBUG: u8 = 4;
/// 请求启动服务（RFC 4253）
pub const SSH_MSG_SERVICE_REQUEST: u8 = 5;
/// 接受服务请求（RFC 4253）
pub const SSH_MSG_SERVICE_ACCEPT: u8 = 6;
/// 定义的扩展协商消息 (RFC 8308)
pub const SSH_MSG_EXT_INFO: u8 = 7;

// =============================================================================
// 消息编号 —— 算法协商与密钥更新 (RFC 4253, 20..=21)
// =============================================================================

/// 密钥交换初始化 / 算法协商（RFC 4253）
pub const SSH_MSG_KEXINIT: u8 = 20;
/// 启用新密钥（RFC 4253）
pub const SSH_MSG_NEWKEYS: u8 = 21;

// =============================================================================
// 消息编号 —— 密钥交换方法专用 (30..=49，可被不同方法复用)
// =============================================================================

// --- Diffie-Hellman 固定群：diffie-hellman-group*-sha* (RFC 4253 §8) ---
/// 客户端 DH 公开值（RFC 4253）
pub const SSH_MSG_KEXDH_INIT: u8 = 30;
/// 服务端主机公钥 + DH 公开值 + 签名（RFC 4253）
pub const SSH_MSG_KEXDH_REPLY: u8 = 31;

// --- Diffie-Hellman 群交换：diffie-hellman-group-exchange-* (RFC 4419) ---
// 注意：与上面的 KEXDH_INIT/REPLY 复用编号 30/31。
/// （旧版）请求指定位数的群（RFC 4419）
pub const SSH_MSG_KEX_DH_GEX_REQUEST_OLD: u8 = 30;
/// 服务端返回素数 p 与生成元 g（RFC 4419）
pub const SSH_MSG_KEX_DH_GEX_GROUP: u8 = 31;
/// 客户端 DH 公开值（RFC 4419）
pub const SSH_MSG_KEX_DH_GEX_INIT: u8 = 32;
/// 服务端主机公钥 + f + 签名（RFC 4419）
pub const SSH_MSG_KEX_DH_GEX_REPLY: u8 = 33;
/// （新版）请求群（min/n/max）（RFC 4419）
pub const SSH_MSG_KEX_DH_GEX_REQUEST: u8 = 34;

pub const SSH_MSG_KEX_ECDH_INIT: u8 = 30;
pub const SSH_MSG_KEX_ECDH_REPLY: u8 = 31;

// =============================================================================
// 消息编号 —— 用户认证协议通用消息 (RFC 4252, 50..=53)
// =============================================================================

/// 发起认证尝试（RFC 4252）
pub const SSH_MSG_USERAUTH_REQUEST: u8 = 50;
/// 认证失败（含可继续的方法列表）（RFC 4252）
pub const SSH_MSG_USERAUTH_FAILURE: u8 = 51;
/// 认证成功（RFC 4252）
pub const SSH_MSG_USERAUTH_SUCCESS: u8 = 52;
/// 认证横幅/告示（RFC 4252）
pub const SSH_MSG_USERAUTH_BANNER: u8 = 53;

// =============================================================================
// 消息编号 —— 用户认证方法专用 (60..=79，可被不同方法复用)
// =============================================================================

/// publickey 方法：公钥可被接受（RFC 4252）
pub const SSH_MSG_USERAUTH_PK_OK: u8 = 60;
/// password 方法：要求修改密码（与 PK_OK 复用编号 60）（RFC 4252）
pub const SSH_MSG_USERAUTH_PASSWD_CHANGEREQ: u8 = 60;
/// keyboard-interactive 方法：服务端提示（复用编号 60）（RFC 4256）
pub const SSH_MSG_USERAUTH_INFO_REQUEST: u8 = 60;
/// keyboard-interactive 方法：客户端应答（RFC 4256）
pub const SSH_MSG_USERAUTH_INFO_RESPONSE: u8 = 61;

// =============================================================================
// 消息编号 —— 连接协议全局请求 (RFC 4254, 80..=82)
// =============================================================================

/// 全局请求（RFC 4254）
pub const SSH_MSG_GLOBAL_REQUEST: u8 = 80;
/// 全局请求成功（RFC 4254）
pub const SSH_MSG_REQUEST_SUCCESS: u8 = 81;
/// 全局请求失败（RFC 4254）
pub const SSH_MSG_REQUEST_FAILURE: u8 = 82;

// =============================================================================
// 消息编号 —— 连接协议通道消息 (RFC 4254, 90..=100)
// =============================================================================

/// 打开通道（RFC 4254）
pub const SSH_MSG_CHANNEL_OPEN: u8 = 90;
/// 通道打开确认（RFC 4254）
pub const SSH_MSG_CHANNEL_OPEN_CONFIRMATION: u8 = 91;
/// 通道打开失败（RFC 4254）
pub const SSH_MSG_CHANNEL_OPEN_FAILURE: u8 = 92;
/// 调整通道窗口（流控）（RFC 4254）
pub const SSH_MSG_CHANNEL_WINDOW_ADJUST: u8 = 93;
/// 通道数据（RFC 4254）
pub const SSH_MSG_CHANNEL_DATA: u8 = 94;
/// 通道扩展数据（如 stderr）（RFC 4254）
pub const SSH_MSG_CHANNEL_EXTENDED_DATA: u8 = 95;
/// 通道 EOF（不再发送数据）（RFC 4254）
pub const SSH_MSG_CHANNEL_EOF: u8 = 96;
/// 关闭通道（RFC 4254）
pub const SSH_MSG_CHANNEL_CLOSE: u8 = 97;
/// 通道请求（pty-req / shell / exec 等）（RFC 4254）
pub const SSH_MSG_CHANNEL_REQUEST: u8 = 98;
/// 通道请求成功（RFC 4254）
pub const SSH_MSG_CHANNEL_SUCCESS: u8 = 99;
/// 通道请求失败（RFC 4254）
pub const SSH_MSG_CHANNEL_FAILURE: u8 = 100;

// =============================================================================
// 断开原因码 —— 用于 SSH_MSG_DISCONNECT 的 reason code (RFC 4250 §4.2)
// =============================================================================

pub const SSH_DISCONNECT_HOST_NOT_ALLOWED_TO_CONNECT: u32 = 1;
pub const SSH_DISCONNECT_PROTOCOL_ERROR: u32 = 2;
pub const SSH_DISCONNECT_KEY_EXCHANGE_FAILED: u32 = 3;
pub const SSH_DISCONNECT_RESERVED: u32 = 4;
pub const SSH_DISCONNECT_MAC_ERROR: u32 = 5;
pub const SSH_DISCONNECT_COMPRESSION_ERROR: u32 = 6;
pub const SSH_DISCONNECT_SERVICE_NOT_AVAILABLE: u32 = 7;
pub const SSH_DISCONNECT_PROTOCOL_VERSION_NOT_SUPPORTED: u32 = 8;
pub const SSH_DISCONNECT_HOST_KEY_NOT_VERIFIABLE: u32 = 9;
pub const SSH_DISCONNECT_CONNECTION_LOST: u32 = 10;
pub const SSH_DISCONNECT_BY_APPLICATION: u32 = 11;
pub const SSH_DISCONNECT_TOO_MANY_CONNECTIONS: u32 = 12;
pub const SSH_DISCONNECT_AUTH_CANCELLED_BY_USER: u32 = 13;
pub const SSH_DISCONNECT_NO_MORE_AUTH_METHODS_AVAILABLE: u32 = 14;
pub const SSH_DISCONNECT_ILLEGAL_USER_NAME: u32 = 15;

// =============================================================================
// 通道打开失败原因码 —— 用于 SSH_MSG_CHANNEL_OPEN_FAILURE (RFC 4254 §5.1)
// =============================================================================

pub const SSH_OPEN_ADMINISTRATIVELY_PROHIBITED: u32 = 1;
pub const SSH_OPEN_CONNECT_FAILED: u32 = 2;
pub const SSH_OPEN_UNKNOWN_CHANNEL_TYPE: u32 = 3;
pub const SSH_OPEN_RESOURCE_SHORTAGE: u32 = 4;

// =============================================================================
// 扩展数据类型码 —— 用于 SSH_MSG_CHANNEL_EXTENDED_DATA (RFC 4250 §4.4)
// =============================================================================

/// 标准错误输出（stderr）
pub const SSH_EXTENDED_DATA_STDERR: u32 = 1;

pub const MAX_PACKET_PAYLOAD_LENGTH: usize = 32768;
pub const MAX_PACKET_LENGTH: usize = 256 * 1024;
pub const MIN_PADDING_LENGTH: usize = 4;
pub const BANNER_MAX: usize = 255;
pub const BANNER_ENDING: &str = "\r\n";

pub const KEX_STRICT_CLIENT: &str = "kex-strict-c-v00@openssh.com";
pub const EXT_INFO_CLIENT: &str = "ext-info-c";

pub const KEX_STRICT_SERVER: &str = "kex-strict-s-v00@openssh.com";
pub const EXT_INFO_SERVER: &str = "ext-info-s";

pub const SSH_SERVICE_NAME_USER_AUTHENTICATION_SERVICE: &str = "ssh-userauth";
pub const SSH_EXTENSION_NAME_SERVER_SIGNATURE_ALGORITHMS: &str = "server-sig-algs";

pub const SSH_GLOBAL_REQUEST_TYPE_CANCEL_TCP_IP_FORWARD: &str = "cancel-tcpip-forward";

pub const SSH_GLOBAL_REQUEST_TYPE_TCP_IP_FORWARD: &str = "tcpip-forward";

pub const SSH_CHANNEL_TYPE_FORWARDED_TCP_IP: &str = "forwarded-tcpip";
pub const SSH_CHANNEL_TYPE_AGENT_CONNECT: &str = "agent-connect";

pub const SSH_CHANNEL_TYPE_SESSION: &str = "session";
pub const SSH_CHANNEL_TYPE_DIRECT_TCP_IP: &str = "direct-tcpip";
pub const SSH_CHANNEL_TYPE_X11: &str = "x11";

pub mod openssh {
    pub const SSH_GLOBAL_REQUEST_TYPE_KEEP_ALIVE: &str = "keepalive@openssh.com";

    pub const SSH_EXTENSION_NAME_PING: &str = "ping@openssh.com";

    pub const SSH_GLOBAL_REQUEST_TYPE_HOST_KEYS: &str = "hostkeys-00@openssh.com";

    pub const SSH_CHANNEL_TYPE_AGENT_CONNECT: &str = "auth-agent@openssh.com";
    pub const SSH_CHANNEL_TYPE_FORWARDED_STREAM_LOCAL: &str = "forwarded-streamlocal@openssh.com";

    pub const DIRECT_STREM_LOCAL: &str = "direct-streamlocal@openssh.com";
    pub const STREAM_LOCAL_FORWARD: &str = "streamlocal-forward@openssh.com";
    pub const CANCEL_STREAM_LOCAL_FORWARD: &str = "cancel-streamlocal-forward@openssh.com";

    // OpenSSH 扩展消息
    pub const SSH_MSG_PING: u8 = 192;
    pub const SSH_MSG_PONG: u8 = 193;
}

pub mod sftp {
    use super::SFTPExtension;

    pub const VERSION: u32 = 3;

    pub const SSH_FXP_INIT: u8 = 1;
    pub const SSH_FXP_VERSION: u8 = 2;
    pub const SSH_FXP_OPEN: u8 = 3;
    pub const SSH_FXP_CLOSE: u8 = 4;
    pub const SSH_FXP_READ: u8 = 5;
    pub const SSH_FXP_WRITE: u8 = 6;
    pub const SSH_FXP_LSTAT: u8 = 7;
    pub const SSH_FXP_FSTAT: u8 = 8;
    pub const SSH_FXP_SETSTAT: u8 = 9;
    pub const SSH_FXP_FSETSTAT: u8 = 10;
    pub const SSH_FXP_OPENDIR: u8 = 11;
    pub const SSH_FXP_READDIR: u8 = 12;
    pub const SSH_FXP_REMOVE: u8 = 13;
    pub const SSH_FXP_MKDIR: u8 = 14;
    pub const SSH_FXP_RMDIR: u8 = 15;
    pub const SSH_FXP_REALPATH: u8 = 16;
    pub const SSH_FXP_STAT: u8 = 17;
    pub const SSH_FXP_RENAME: u8 = 18;
    pub const SSH_FXP_READLINK: u8 = 19;
    pub const SSH_FXP_SYMLINK: u8 = 20;

    pub const SSH_FXP_STATUS: u8 = 101;
    pub const SSH_FXP_HANDLE: u8 = 102;
    pub const SSH_FXP_DATA: u8 = 103;
    pub const SSH_FXP_NAME: u8 = 104;
    pub const SSH_FXP_ATTRS: u8 = 105;

    pub const SSH_FXP_EXTENDED: u8 = 200;
    pub const SSH_FXP_EXTENDED_REPLY: u8 = 201;

    // pub const SSH_FXF_ACCESS_DISPOSITION: u32 = 0x00000007;
    // pub const SSH_FXF_CREATE_NEW: u32 = 0x00000000;
    // pub const SSH_FXF_CREATE_TRUNCATE: u32 = 0x00000001;
    // pub const SSH_FXF_OPEN_EXISTING: u32 = 0x00000002;
    // pub const SSH_FXF_OPEN_OR_CREATE: u32 = 0x00000003;
    // pub const SSH_FXF_TRUNCATE_EXISTING: u32 = 0x00000004;
    // pub const SSH_FXF_APPEND_DATA: u32 = 0x00000008;
    // pub const SSH_FXF_APPEND_DATA_ATOMIC: u32 = 0x00000010;
    // pub const SSH_FXF_TEXT_MODE: u32 = 0x00000020;
    // pub const SSH_FXF_BLOCK_READ: u32 = 0x00000040;
    // pub const SSH_FXF_BLOCK_WRITE: u32 = 0x00000080;
    // pub const SSH_FXF_BLOCK_DELETE: u32 = 0x00000100;
    // pub const SSH_FXF_BLOCK_ADVISORY: u32 = 0x00000200;
    // pub const SSH_FXF_NOFOLLOW: u32 = 0x00000400;
    // pub const SSH_FXF_DELETE_ON_CLOSE: u32 = 0x00000800;
    // pub const SSH_FXF_ACCESS_AUDIT_ALARM_INFO: u32 = 0x00001000;
    // pub const SSH_FXF_ACCESS_BACKUP: u32 = 0x00002000;
    // pub const SSH_FXF_BACKUP_STREAM: u32 = 0x00004000;
    // pub const SSH_FXF_OVERRIDE_OWNER: u32 = 0x00008000;

    pub const SSH_FXF_READ: u32 = 0x00000001;
    pub const SSH_FXF_WRITE: u32 = 0x00000002;
    pub const SSH_FXF_APPEND: u32 = 0x00000004;
    pub const SSH_FXF_CREAT: u32 = 0x00000008;
    pub const SSH_FXF_TRUNC: u32 = 0x00000010;
    pub const SSH_FXF_EXCL: u32 = 0x00000020;

    pub const SSH_FILEXFER_ATTR_SIZE: u32 = 0x00000001;
    pub const SSH_FILEXFER_ATTR_UIDGID: u32 = 0x00000002;
    pub const SSH_FILEXFER_ATTR_PERMISSIONS: u32 = 0x00000004;
    pub const SSH_FILEXFER_ATTR_ACMODTIME: u32 = 0x00000008;
    pub const SSH_FILEXFER_ATTR_EXTENDED: u32 = 0x80000000;

    pub const SSH_FX_OK: u32 = 0;
    pub const SSH_FX_EOF: u32 = 1;
    pub const SSH_FX_NO_SUCH_FILE: u32 = 2;
    pub const SSH_FX_PERMISSION_DENIED: u32 = 3;
    pub const SSH_FX_FAILURE: u32 = 4;
    pub const SSH_FX_BAD_MESSAGE: u32 = 5;
    pub const SSH_FX_NO_CONNECTION: u32 = 6;
    pub const SSH_FX_CONNECTION_LOST: u32 = 7;
    pub const SSH_FX_OP_UNSUPPORTED: u32 = 8;

    pub const OPENSSH_SFTP_EXT_POSIX_RENAME: SFTPExtension =
        SFTPExtension::new("posix-rename@openssh.com", b"1");
    pub const OPENSSH_SFTP_EXT_STATVFS: SFTPExtension =
        SFTPExtension::new("statvfs@openssh.com", b"2");
    pub const OPENSSH_SFTP_EXT_FSTATVFS: SFTPExtension =
        SFTPExtension::new("fstatvfs@openssh.com", b"2");
    pub const OPENSSH_SFTP_EXT_HARDLINK: SFTPExtension =
        SFTPExtension::new("hardlink@openssh.com", b"1");
    pub const OPENSSH_SFTP_EXT_FSYNC: SFTPExtension = SFTPExtension::new("fsync@openssh.com", b"1");
    pub const OPENSSH_SFTP_EXT_LSETSTAT: SFTPExtension =
        SFTPExtension::new("lsetstat@openssh.com", b"1");
    pub const OPENSSH_SFTP_EXT_LIMITS: SFTPExtension =
        SFTPExtension::new("limits@openssh.com", b"1");
    pub const OPENSSH_SFTP_EXT_EXPAND_PATH: SFTPExtension =
        SFTPExtension::new("expand-path@openssh.com", b"1");
    pub const OPENSSH_SFTP_EXT_COPY_DATA: SFTPExtension = SFTPExtension::new("copy-data", b"1");
    pub const OPENSSH_SFTP_EXT_HOME_DIRECTORY: SFTPExtension =
        SFTPExtension::new("home-directory", b"1");
    pub const OPENSSH_SFTP_EXT_USERS_GROUPS_BY_ID: SFTPExtension =
        SFTPExtension::new("users-groups-by-id@openssh.com", b"1");
}
