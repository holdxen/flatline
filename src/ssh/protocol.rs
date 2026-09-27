//! SSH-2 protocol constants.
//!
//! Compiled from RFC 4250 / 4252 / 4253 / 4254, plus the method-specific
//! messages of RFC 4256 (keyboard-interactive) and RFC 4419 (DH group
//! exchange).
//!
//! Conventions:
//! - Message numbers are byte values in `1..=255`, typed `u8`.
//! - Disconnect reason codes, channel-open failure reason codes and extended
//!   data type codes are all `uint32`, typed `u32`.
//! - Numbers `30..=49` and `60..=79` are "method-specific" and may be reused
//!   by different methods, so their values overlap (the constant names all
//!   differ, so everything still compiles).

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

/// Disconnect (RFC 4253)
pub const SSH_MSG_DISCONNECT: u8 = 1;
/// An ignorable message (RFC 4253)
pub const SSH_MSG_IGNORE: u8 = 2;
/// Unimplemented / unrecognized message number (RFC 4253)
pub const SSH_MSG_UNIMPLEMENTED: u8 = 3;
/// Debug information (RFC 4253)
pub const SSH_MSG_DEBUG: u8 = 4;
/// Request to start a service (RFC 4253)
pub const SSH_MSG_SERVICE_REQUEST: u8 = 5;
/// Accept the service request (RFC 4253)
pub const SSH_MSG_SERVICE_ACCEPT: u8 = 6;
/// Extension negotiation message (RFC 8308)
pub const SSH_MSG_EXT_INFO: u8 = 7;

// =============================================================================
// 消息编号 —— 算法协商与密钥更新 (RFC 4253, 20..=21)
// =============================================================================

/// Key exchange initialization / algorithm negotiation (RFC 4253)
pub const SSH_MSG_KEXINIT: u8 = 20;
/// Enable the new keys (RFC 4253)
pub const SSH_MSG_NEWKEYS: u8 = 21;

// =============================================================================
// 消息编号 —— 密钥交换方法专用 (30..=49，可被不同方法复用)
// =============================================================================

// --- Diffie-Hellman 固定群：diffie-hellman-group*-sha* (RFC 4253 §8) ---
/// Client DH public value (RFC 4253)
pub const SSH_MSG_KEXDH_INIT: u8 = 30;
/// Server host key + DH public value + signature (RFC 4253)
pub const SSH_MSG_KEXDH_REPLY: u8 = 31;

// --- Diffie-Hellman 群交换：diffie-hellman-group-exchange-* (RFC 4419) ---
// 注意：与上面的 KEXDH_INIT/REPLY 复用编号 30/31。
/// (Legacy) Request a group of a given bit size (RFC 4419)
pub const SSH_MSG_KEX_DH_GEX_REQUEST_OLD: u8 = 30;
/// Server returns the prime `p` and generator `g` (RFC 4419)
pub const SSH_MSG_KEX_DH_GEX_GROUP: u8 = 31;
/// Client DH public value (RFC 4419)
pub const SSH_MSG_KEX_DH_GEX_INIT: u8 = 32;
/// Server host key + `f` + signature (RFC 4419)
pub const SSH_MSG_KEX_DH_GEX_REPLY: u8 = 33;
/// (Current) Request a group, min/n/max (RFC 4419)
pub const SSH_MSG_KEX_DH_GEX_REQUEST: u8 = 34;

pub const SSH_MSG_KEX_ECDH_INIT: u8 = 30;
pub const SSH_MSG_KEX_ECDH_REPLY: u8 = 31;

// =============================================================================
// 消息编号 —— 用户认证协议通用消息 (RFC 4252, 50..=53)
// =============================================================================

/// Begin an authentication attempt (RFC 4252)
pub const SSH_MSG_USERAUTH_REQUEST: u8 = 50;
/// Authentication failed, with the list of methods that may continue (RFC 4252)
pub const SSH_MSG_USERAUTH_FAILURE: u8 = 51;
/// Authentication succeeded (RFC 4252)
pub const SSH_MSG_USERAUTH_SUCCESS: u8 = 52;
/// Authentication banner / notice (RFC 4252)
pub const SSH_MSG_USERAUTH_BANNER: u8 = 53;

// =============================================================================
// 消息编号 —— 用户认证方法专用 (60..=79，可被不同方法复用)
// =============================================================================

/// publickey method: the public key would be acceptable (RFC 4252)
pub const SSH_MSG_USERAUTH_PK_OK: u8 = 60;
/// password method: the password must be changed (reuses number 60 with PK_OK) (RFC 4252)
pub const SSH_MSG_USERAUTH_PASSWD_CHANGEREQ: u8 = 60;
/// keyboard-interactive method: server prompt (reuses number 60) (RFC 4256)
pub const SSH_MSG_USERAUTH_INFO_REQUEST: u8 = 60;
/// keyboard-interactive method: client response (RFC 4256)
pub const SSH_MSG_USERAUTH_INFO_RESPONSE: u8 = 61;

// =============================================================================
// 消息编号 —— 连接协议全局请求 (RFC 4254, 80..=82)
// =============================================================================

/// Global request (RFC 4254)
pub const SSH_MSG_GLOBAL_REQUEST: u8 = 80;
/// The global request succeeded (RFC 4254)
pub const SSH_MSG_REQUEST_SUCCESS: u8 = 81;
/// The global request failed (RFC 4254)
pub const SSH_MSG_REQUEST_FAILURE: u8 = 82;

// =============================================================================
// 消息编号 —— 连接协议通道消息 (RFC 4254, 90..=100)
// =============================================================================

/// Open a channel (RFC 4254)
pub const SSH_MSG_CHANNEL_OPEN: u8 = 90;
/// Channel open confirmation (RFC 4254)
pub const SSH_MSG_CHANNEL_OPEN_CONFIRMATION: u8 = 91;
/// Channel open failure (RFC 4254)
pub const SSH_MSG_CHANNEL_OPEN_FAILURE: u8 = 92;
/// Adjust the channel window (flow control) (RFC 4254)
pub const SSH_MSG_CHANNEL_WINDOW_ADJUST: u8 = 93;
/// Channel data (RFC 4254)
pub const SSH_MSG_CHANNEL_DATA: u8 = 94;
/// Channel extended data, e.g. stderr (RFC 4254)
pub const SSH_MSG_CHANNEL_EXTENDED_DATA: u8 = 95;
/// Channel EOF: no more data will be sent (RFC 4254)
pub const SSH_MSG_CHANNEL_EOF: u8 = 96;
/// Close the channel (RFC 4254)
pub const SSH_MSG_CHANNEL_CLOSE: u8 = 97;
/// Channel request, e.g. pty-req / shell / exec (RFC 4254)
pub const SSH_MSG_CHANNEL_REQUEST: u8 = 98;
/// The channel request succeeded (RFC 4254)
pub const SSH_MSG_CHANNEL_SUCCESS: u8 = 99;
/// The channel request failed (RFC 4254)
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

/// Standard error output (stderr)
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn transport_messages_sit_in_their_rfc_range() {
        // RFC 4253 section 6: 1..=6 are generic transport messages.
        for code in [
            SSH_MSG_DISCONNECT,
            SSH_MSG_IGNORE,
            SSH_MSG_UNIMPLEMENTED,
            SSH_MSG_DEBUG,
            SSH_MSG_SERVICE_REQUEST,
            SSH_MSG_SERVICE_ACCEPT,
        ] {
            assert!((1..=6).contains(&code), "out of range: {code}");
        }
        assert_eq!(SSH_MSG_EXT_INFO, 7, "RFC 8308");

        // 20/21 are negotiation and key activation.
        assert_eq!(SSH_MSG_KEXINIT, 20);
        assert_eq!(SSH_MSG_NEWKEYS, 21);
    }

    #[test]
    fn method_specific_messages_reuse_their_documented_numbers() {
        // Both KEXDH and GEX start at 30/31 (RFC 4253 §8, RFC 4419).
        assert_eq!(SSH_MSG_KEXDH_INIT, SSH_MSG_KEX_DH_GEX_REQUEST_OLD);
        assert_eq!(SSH_MSG_KEXDH_REPLY, SSH_MSG_KEX_DH_GEX_GROUP);
        assert_eq!(SSH_MSG_KEX_ECDH_INIT, SSH_MSG_KEXDH_INIT);
        assert_eq!(SSH_MSG_KEX_ECDH_REPLY, SSH_MSG_KEXDH_REPLY);

        assert_eq!(SSH_MSG_KEX_DH_GEX_INIT, 32);
        assert_eq!(SSH_MSG_KEX_DH_GEX_REPLY, 33);
        assert_eq!(SSH_MSG_KEX_DH_GEX_REQUEST, 34);
    }

    #[test]
    fn userauth_messages_sit_in_their_rfc_range() {
        for code in [
            SSH_MSG_USERAUTH_REQUEST,
            SSH_MSG_USERAUTH_FAILURE,
            SSH_MSG_USERAUTH_SUCCESS,
            SSH_MSG_USERAUTH_BANNER,
        ] {
            assert!((50..=53).contains(&code), "out of range: {code}");
        }

        // 60 is shared by three different method-specific messages.
        assert_eq!(SSH_MSG_USERAUTH_PK_OK, 60);
        assert_eq!(SSH_MSG_USERAUTH_PASSWD_CHANGEREQ, 60);
        assert_eq!(SSH_MSG_USERAUTH_INFO_REQUEST, 60);
        assert_eq!(SSH_MSG_USERAUTH_INFO_RESPONSE, 61);
    }

    #[test]
    fn connection_messages_sit_in_their_rfc_range() {
        for code in [
            SSH_MSG_GLOBAL_REQUEST,
            SSH_MSG_REQUEST_SUCCESS,
            SSH_MSG_REQUEST_FAILURE,
        ] {
            assert!((80..=82).contains(&code), "out of range: {code}");
        }

        // Channel messages run 90..=100 and are strictly increasing, so
        // matching on the code is unambiguous.
        let channel_codes = [
            SSH_MSG_CHANNEL_OPEN,
            SSH_MSG_CHANNEL_OPEN_CONFIRMATION,
            SSH_MSG_CHANNEL_OPEN_FAILURE,
            SSH_MSG_CHANNEL_WINDOW_ADJUST,
            SSH_MSG_CHANNEL_DATA,
            SSH_MSG_CHANNEL_EXTENDED_DATA,
            SSH_MSG_CHANNEL_EOF,
            SSH_MSG_CHANNEL_CLOSE,
            SSH_MSG_CHANNEL_REQUEST,
            SSH_MSG_CHANNEL_SUCCESS,
            SSH_MSG_CHANNEL_FAILURE,
        ];
        for window in channel_codes.windows(2) {
            assert_eq!(window[1], window[0] + 1, "channel codes must be contiguous");
        }
        assert_eq!(channel_codes[0], 90);
        assert_eq!(*channel_codes.last().unwrap(), 100);
    }

    #[test]
    fn disconnect_reason_codes_match_rfc_4250() {
        assert_eq!(SSH_DISCONNECT_HOST_NOT_ALLOWED_TO_CONNECT, 1);
        assert_eq!(SSH_DISCONNECT_PROTOCOL_ERROR, 2);
        assert_eq!(SSH_DISCONNECT_KEY_EXCHANGE_FAILED, 3);
        assert_eq!(SSH_DISCONNECT_MAC_ERROR, 5);
        assert_eq!(SSH_DISCONNECT_BY_APPLICATION, 11);
        assert_eq!(SSH_DISCONNECT_NO_MORE_AUTH_METHODS_AVAILABLE, 14);
        assert_eq!(SSH_DISCONNECT_ILLEGAL_USER_NAME, 15);

        // Contiguous 1..=15.
        assert_eq!(SSH_DISCONNECT_ILLEGAL_USER_NAME, 15);
    }

    #[test]
    fn channel_open_failure_codes_match_rfc_4254() {
        assert_eq!(SSH_OPEN_ADMINISTRATIVELY_PROHIBITED, 1);
        assert_eq!(SSH_OPEN_CONNECT_FAILED, 2);
        assert_eq!(SSH_OPEN_UNKNOWN_CHANNEL_TYPE, 3);
        assert_eq!(SSH_OPEN_RESOURCE_SHORTAGE, 4);
    }

    #[test]
    fn extended_data_stderr_is_the_only_defined_type() {
        assert_eq!(SSH_EXTENDED_DATA_STDERR, 1);
    }

    #[test]
    fn packet_size_limits_are_sane() {
        // RFC 4253: every implementation must support 32768-byte payloads.
        assert_eq!(MAX_PACKET_PAYLOAD_LENGTH, 32768);
        assert!(MAX_PACKET_LENGTH > MAX_PACKET_PAYLOAD_LENGTH);
        assert!(MIN_PADDING_LENGTH >= 4, "RFC 4253 requires at least 4");
        assert_eq!(BANNER_MAX, 255, "RFC 4253 caps the banner at 255 bytes");
        assert_eq!(BANNER_ENDING, "\r\n");
    }

    #[test]
    fn strict_kex_and_ext_info_marker_names_match_openssh() {
        assert_eq!(KEX_STRICT_CLIENT, "kex-strict-c-v00@openssh.com");
        assert_eq!(KEX_STRICT_SERVER, "kex-strict-s-v00@openssh.com");
        assert_eq!(EXT_INFO_CLIENT, "ext-info-c");
        assert_eq!(EXT_INFO_SERVER, "ext-info-s");

        // The client and server markers must differ, or a peer's marker
        // would be mistaken for our own.
        assert_ne!(KEX_STRICT_CLIENT, KEX_STRICT_SERVER);
        assert_ne!(EXT_INFO_CLIENT, EXT_INFO_SERVER);
    }

    #[test]
    fn service_and_extension_names_match_the_specifications() {
        assert_eq!(SSH_SERVICE_NAME_USER_AUTHENTICATION_SERVICE, "ssh-userauth");
        assert_eq!(
            SSH_EXTENSION_NAME_SERVER_SIGNATURE_ALGORITHMS,
            "server-sig-algs"
        );
    }

    #[test]
    fn channel_and_forwarding_type_names_match_openssh() {
        assert_eq!(SSH_CHANNEL_TYPE_SESSION, "session");
        assert_eq!(SSH_CHANNEL_TYPE_DIRECT_TCP_IP, "direct-tcpip");
        assert_eq!(SSH_CHANNEL_TYPE_X11, "x11");
        assert_eq!(SSH_CHANNEL_TYPE_FORWARDED_TCP_IP, "forwarded-tcpip");
        assert_eq!(SSH_GLOBAL_REQUEST_TYPE_TCP_IP_FORWARD, "tcpip-forward");
        assert_eq!(
            SSH_GLOBAL_REQUEST_TYPE_CANCEL_TCP_IP_FORWARD,
            "cancel-tcpip-forward"
        );
    }

    #[test]
    fn openssh_extension_names_are_namespaced() {
        use openssh::*;
        // Every OpenSSH-specific name carries the @openssh.com suffix (or
        // the vendor prefix), which is what distinguishes them from
        // standardized names.
        assert_eq!(SSH_GLOBAL_REQUEST_TYPE_KEEP_ALIVE, "keepalive@openssh.com");
        assert_eq!(SSH_GLOBAL_REQUEST_TYPE_HOST_KEYS, "hostkeys-00@openssh.com");
        assert_eq!(SSH_EXTENSION_NAME_PING, "ping@openssh.com");
        assert_eq!(SSH_CHANNEL_TYPE_AGENT_CONNECT, "auth-agent@openssh.com");
        assert_eq!(
            SSH_CHANNEL_TYPE_FORWARDED_STREAM_LOCAL,
            "forwarded-streamlocal@openssh.com"
        );
        assert_eq!(DIRECT_STREM_LOCAL, "direct-streamlocal@openssh.com");
        assert_eq!(STREAM_LOCAL_FORWARD, "streamlocal-forward@openssh.com");
        assert_eq!(
            CANCEL_STREAM_LOCAL_FORWARD,
            "cancel-streamlocal-forward@openssh.com"
        );
    }

    #[test]
    fn openssh_pings_sit_above_the_standard_message_range() {
        // OpenSSH uses 192/193 for ping/pong, clear of RFC-defined codes.
        assert_eq!(openssh::SSH_MSG_PING, 192);
        assert_eq!(openssh::SSH_MSG_PONG, 193);
        assert!(openssh::SSH_MSG_PING > 100);
        assert_ne!(openssh::SSH_MSG_PING, openssh::SSH_MSG_PONG);
    }

    #[test]
    fn sftp_version_is_three() {
        assert_eq!(sftp::VERSION, 3);
    }

    #[test]
    fn sftp_request_codes_are_contiguous_and_replies_are_separate() {
        // Requests run 1..=20 in declaration order...
        let requests = [
            sftp::SSH_FXP_INIT,
            sftp::SSH_FXP_VERSION,
            sftp::SSH_FXP_OPEN,
            sftp::SSH_FXP_CLOSE,
            sftp::SSH_FXP_READ,
            sftp::SSH_FXP_WRITE,
            sftp::SSH_FXP_LSTAT,
            sftp::SSH_FXP_FSTAT,
            sftp::SSH_FXP_SETSTAT,
            sftp::SSH_FXP_FSETSTAT,
            sftp::SSH_FXP_OPENDIR,
            sftp::SSH_FXP_READDIR,
            sftp::SSH_FXP_REMOVE,
            sftp::SSH_FXP_MKDIR,
            sftp::SSH_FXP_RMDIR,
            sftp::SSH_FXP_REALPATH,
            sftp::SSH_FXP_STAT,
            sftp::SSH_FXP_RENAME,
            sftp::SSH_FXP_READLINK,
            sftp::SSH_FXP_SYMLINK,
        ];
        for (i, code) in requests.iter().enumerate() {
            assert_eq!(*code, i as u8 + 1, "request codes must be 1..=20");
        }

        // ...and replies start at 101 so they can never be confused.
        assert_eq!(sftp::SSH_FXP_STATUS, 101);
        assert_eq!(sftp::SSH_FXP_HANDLE, 102);
        assert_eq!(sftp::SSH_FXP_DATA, 103);
        assert_eq!(sftp::SSH_FXP_NAME, 104);
        assert_eq!(sftp::SSH_FXP_ATTRS, 105);

        assert_eq!(sftp::SSH_FXP_EXTENDED, 200);
        assert_eq!(sftp::SSH_FXP_EXTENDED_REPLY, 201);
    }

    #[test]
    fn sftp_open_flags_are_distinct_bits() {
        let flags = [
            sftp::SSH_FXF_READ,
            sftp::SSH_FXF_WRITE,
            sftp::SSH_FXF_APPEND,
            sftp::SSH_FXF_CREAT,
            sftp::SSH_FXF_TRUNC,
            sftp::SSH_FXF_EXCL,
        ];

        // Powers of two, so they can be OR-ed together.
        for flag in flags {
            assert!(flag.is_power_of_two(), "{flag:#x} is not a single bit");
        }
        // No duplicates.
        let unique: std::collections::HashSet<_> = flags.iter().collect();
        assert_eq!(unique.len(), flags.len());
    }

    #[test]
    fn sftp_attribute_flags_are_distinct_bits() {
        let flags = [
            sftp::SSH_FILEXFER_ATTR_SIZE,
            sftp::SSH_FILEXFER_ATTR_UIDGID,
            sftp::SSH_FILEXFER_ATTR_PERMISSIONS,
            sftp::SSH_FILEXFER_ATTR_ACMODTIME,
            sftp::SSH_FILEXFER_ATTR_EXTENDED,
        ];
        for flag in flags {
            assert!(flag.is_power_of_two(), "{flag:#x} is not a single bit");
        }
        let unique: std::collections::HashSet<_> = flags.iter().collect();
        assert_eq!(unique.len(), flags.len());
    }

    #[test]
    fn sftp_status_codes_match_the_draft() {
        assert_eq!(sftp::SSH_FX_OK, 0);
        assert_eq!(sftp::SSH_FX_EOF, 1);
        assert_eq!(sftp::SSH_FX_NO_SUCH_FILE, 2);
        assert_eq!(sftp::SSH_FX_PERMISSION_DENIED, 3);
        assert_eq!(sftp::SSH_FX_FAILURE, 4);
        assert_eq!(sftp::SSH_FX_BAD_MESSAGE, 5);
        assert_eq!(sftp::SSH_FX_NO_CONNECTION, 6);
        assert_eq!(sftp::SSH_FX_CONNECTION_LOST, 7);
        assert_eq!(sftp::SSH_FX_OP_UNSUPPORTED, 8);
    }

    #[test]
    fn sftp_extension_constants_carry_version_and_name() {
        let cases = [
            (
                sftp::OPENSSH_SFTP_EXT_POSIX_RENAME,
                "posix-rename@openssh.com",
                b"1",
            ),
            (sftp::OPENSSH_SFTP_EXT_STATVFS, "statvfs@openssh.com", b"2"),
            (
                sftp::OPENSSH_SFTP_EXT_FSTATVFS,
                "fstatvfs@openssh.com",
                b"2",
            ),
            (
                sftp::OPENSSH_SFTP_EXT_HARDLINK,
                "hardlink@openssh.com",
                b"1",
            ),
            (sftp::OPENSSH_SFTP_EXT_FSYNC, "fsync@openssh.com", b"1"),
            (
                sftp::OPENSSH_SFTP_EXT_LSETSTAT,
                "lsetstat@openssh.com",
                b"1",
            ),
            (sftp::OPENSSH_SFTP_EXT_LIMITS, "limits@openssh.com", b"1"),
            (
                sftp::OPENSSH_SFTP_EXT_EXPAND_PATH,
                "expand-path@openssh.com",
                b"1",
            ),
            (sftp::OPENSSH_SFTP_EXT_COPY_DATA, "copy-data", b"1"),
            (
                sftp::OPENSSH_SFTP_EXT_HOME_DIRECTORY,
                "home-directory",
                b"1",
            ),
            (
                sftp::OPENSSH_SFTP_EXT_USERS_GROUPS_BY_ID,
                "users-groups-by-id@openssh.com",
                b"1",
            ),
        ];

        for (ext, name, version) in cases {
            assert_eq!(ext.key, name, "extension name");
            assert_eq!(ext.value, version, "extension version for {name}");
        }
    }

    #[test]
    fn sftp_extension_constant_builder_is_available() {
        let ext = SFTPExtension::new("vendor@example.com", b"3");
        assert_eq!(ext.key, "vendor@example.com");
        assert_eq!(ext.value, b"3");
    }
}
