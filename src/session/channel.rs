//! SSH connection-layer channels (RFC 4254) and terminal modes (RFC 4254 §8).
//!
//! A channel is an independent, flow-controlled stream of bytes carried over
//! an established SSH session. Each side labels the channel with its own
//! number (see [`IdentityPair`]); either side may send data, signal
//! end-of-file, or close the channel.
//!
//! # Obtaining a channel
//!
//! Open one with
//! [`Session::channel_open_default`](crate::session::Session::channel_open_default)
//! (or [`Session::channel_open`](crate::session::Session::channel_open) with
//! explicit window sizes); SFTP and forwarding helpers open channels of their
//! own.
//!
//! # Using a channel
//!
//! Ask the server to start something with the `request_*` methods:
//! [`Channel::request_exec`] runs a single command, [`Channel::request_shell`]
//! starts an interactive shell, [`Channel::request_pty`] allocates a
//! pseudo-terminal (its settings are described by [`TtyOpcode`]), and the
//! remaining requests ([`Channel::request_env`], [`Channel::request_signal`],
//! [`Channel::request_window_change`], [`Channel::request_x11`],
//! [`Channel::request_agent`], [`Channel::request_break`]) send the other
//! RFC 4254 channel requests. Send bytes with [`Channel::send`] and read the
//! resulting [`Message`]s with [`Channel::receive`] (waits for the next
//! message) or [`Channel::try_receive`] (returns immediately). Dropping a
//! [`Channel`] closes it.
//!
//! This module is re-exported at the crate root as [`channel`].
use super::{Event, UnexpectedReceivingError, UnexpectedSendingError, channel};
use crate::{error, ssh::msg::Signal};
use bytes::{Buf, BytesMut};
use snafu::OptionExt;
use tokio::sync::mpsc::error::{TryRecvError, TrySendError};
use tokio::sync::{mpsc, oneshot};

/// The exit status of the remote command or shell, as reported by the server.
///
/// Received in a [`Message::Exit`].
#[derive(Debug, Clone)]
pub enum ExitStatus {
    /// The process terminated normally with the given exit code.
    Normal(u32),
    /// The process was killed by a signal instead of exiting on its own.
    Interrupt {
        /// The signal that killed the process (e.g. `TERM`).
        signal: Signal,
        /// Whether the process produced a core dump.
        core_dumped: bool,
        /// A human-readable error message supplied by the server.
        error_message: String,
    },
}

impl ExitStatus {
    /// Returns `true` only for [`ExitStatus::Normal`] with an exit code of `0`.
    pub fn success(&self) -> bool {
        matches!(self, Self::Normal(0))
    }
}

/// A message received from the server for a [`Channel`].
#[derive(derive_more::Debug)]
pub enum Message {
    /// The server closed the channel; it can no longer be read from or written to.
    Close,
    /// The server sent end-of-file and will send no more data (the channel
    /// stays open for sending).
    Eof,
    /// Standard output of the remote process, as raw bytes (what `println!` writes).
    Stdout(#[debug(skip)] Vec<u8>),
    /// Standard error of the remote process, as raw bytes (what `eprintln!` writes).
    Stderr(#[debug(skip)] Vec<u8>),
    /// The exit status of the remote process, sent when it terminates; it may
    /// arrive before [`Message::Eof`].
    Exit(ExitStatus),
    /// The server asked to switch XON/XOFF flow control on or off (the
    /// `xon-xoff` channel request).
    FlowControl {
        /// Whether XON/XOFF flow control should be turned on.
        on: bool,
    },
    /// The server enlarged this channel's send window by `size` bytes
    /// (`SSH_MSG_CHANNEL_WINDOW_ADJUST`), so that much more data may be sent;
    /// this is a window update, not a terminal-size change (report a new size
    /// with [`Channel::request_window_change`]).
    WindowChange {
        /// The number of bytes added to the send window.
        size: u32,
    },
}

/// Definition of the SSH terminal-mode opcodes.
///
/// Based on RFC 4254 Section 8 and OpenSSH extensions.
///
/// References:
/// - <https://tools.ietf.org/html/rfc4254#section-8>
/// - <https://www.iana.org/assignments/ssh-parameters/ssh-parameters.xhtml#ssh-parameters-16>
///   (Terminal Modes Opcode enum)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TtyOpcode {
    // ========== 结束标记 ==========
    /// Marks the end of the terminal modes data.
    TtyOpEnd = 0,

    // ========== 特殊字符类 (1-18) ==========
    /// Interrupt signal character (usually `Ctrl+C`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VIntr = 1,

    /// Quit signal character (usually `Ctrl+\`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VQuit = 2,

    /// Erase character (usually `Backspace`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VErase = 3,

    /// Kill (erase the whole line) character (usually `Ctrl+U`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VKill = 4,

    /// End-of-file character (usually `Ctrl+D`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VEOF = 5,

    /// Additional end-of-line character.
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VEOL = 6,

    /// Second additional end-of-line character.
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VEOL2 = 7,

    /// Resume-output character (usually `Ctrl+Q`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VStart = 8,

    /// Stop-output character (usually `Ctrl+S`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VStop = 9,

    /// Suspend signal character (usually `Ctrl+Z`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VSusp = 10,

    /// Delayed suspend character (usually `Ctrl+Y`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VDSusp = 11,

    /// Reprint-line character (usually `Ctrl+R`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VReprint = 12,

    /// Word-erase character (usually `Ctrl+W`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VWerase = 13,

    /// Literal-next character (usually `Ctrl+V`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VLNext = 14,

    /// Flush-output character (OpenSSH extension).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VFlush = 15,

    /// Switch shell-layer character (OpenSSH extension).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VSwitch = 16,

    /// Status-request character (usually `Ctrl+T`).
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VStatus = 17,

    /// Discard-output character.
    ///
    /// Value: 0-127 (ASCII character), 255 disables it.
    VDiscard = 18,

    // ========== 输入标志类 (30-42) ==========
    /// Ignore parity and framing errors.
    ///
    /// Value: 0 (do not ignore) or 1 (ignore).
    IGNPAR = 30,

    /// Mark parity and framing errors.
    ///
    /// Value: 0 (do not mark) or 1 (mark).
    PARMRK = 31,

    /// Enable input parity checking.
    ///
    /// Value: 0 (off) or 1 (on).
    INPCK = 32,

    /// Strip the 8th (high) bit from input.
    ///
    /// Value: 0 (do not strip) or 1 (strip).
    ISTRIP = 33,

    /// Map input NL to CR.
    ///
    /// Value: 0 (do not convert) or 1 (convert).
    INLCR = 34,

    /// Ignore input CR.
    ///
    /// Value: 0 (do not ignore) or 1 (ignore).
    IGNCR = 35,

    /// Map input CR to NL.
    ///
    /// Value: 0 (do not convert) or 1 (convert).
    ICRNL = 36,

    /// Map uppercase input characters to lowercase.
    ///
    /// Value: 0 (do not convert) or 1 (convert).
    IUCLC = 37,

    /// Enable XON/XOFF flow control on output.
    ///
    /// Value: 0 (off) or 1 (on).
    IXON = 38,

    /// Any character restarts output.
    ///
    /// Value: 0 (only XON) or 1 (any character).
    IXANY = 39,

    /// Enable XON/XOFF flow control on input.
    ///
    /// Value: 0 (off) or 1 (on).
    IXOFF = 40,

    /// Ring the bell when the input queue is full.
    ///
    /// Value: 0 (do not ring) or 1 (ring).
    IMAXBEL = 41,

    /// Input is UTF-8 encoded.
    ///
    /// Value: 0 (not UTF-8) or 1 (UTF-8).
    IUTF8 = 42,

    // ========== 本地标志类 (50-62) ==========
    /// Enable the signal characters (VINTR, VQUIT, VSUSP).
    ///
    /// Value: 0 (off) or 1 (on).
    ISIG = 50,

    /// Enable canonical mode (line buffering).
    ///
    /// Value: 0 (non-canonical mode) or 1 (canonical mode).
    ICANON = 51,

    /// Enable case conversion.
    ///
    /// Value: 0 (do not convert) or 1 (convert).
    XCASE = 52,

    /// Echo input characters.
    ///
    /// Value: 0 (do not echo) or 1 (echo).
    ECHO = 53,

    /// Echo the erase character.
    ///
    /// Value: 0 (do not echo) or 1 (echo).
    ECHOE = 54,

    /// Echo the kill character.
    ///
    /// Value: 0 (do not echo) or 1 (echo).
    ECHOK = 55,

    /// Echo the newline character even when ECHO is off.
    ///
    /// Value: 0 (do not echo) or 1 (echo).
    ECHONL = 56,

    /// Do not flush the input/output queues on receipt of a signal.
    ///
    /// Value: 0 (flush) or 1 (do not flush).
    NOFLSH = 57,

    /// Send `SIGTTOU` when a background process writes to the terminal.
    ///
    /// Value: 0 (allow) or 1 (stop).
    TOSTOP = 58,

    /// Enable extended input processing.
    ///
    /// Value: 0 (off) or 1 (on).
    IEXTEN = 59,

    /// Echo control characters in `^X` form.
    ///
    /// Value: 0 (as-is) or 1 (as `^X`).
    ECHOCTL = 60,

    /// The kill character erases the whole echoed line.
    ///
    /// Value: 0 (do not erase) or 1 (erase).
    ECHOKE = 61,

    /// There is input waiting to be reprinted.
    ///
    /// Value: 0 (none) or 1 (pending).
    PENDIN = 62,

    // ========== 输出标志类 (70-75) ==========
    /// Enable output post-processing.
    ///
    /// Value: 0 (off) or 1 (on).
    OPOST = 70,

    /// Map lowercase output characters to uppercase.
    ///
    /// Value: 0 (do not convert) or 1 (convert).
    OLCUC = 71,

    /// Map output NL to CR-NL.
    ///
    /// Value: 0 (do not convert) or 1 (convert).
    ONLCR = 72,

    /// Map output CR to NL.
    ///
    /// Value: 0 (do not convert) or 1 (convert).
    OCRNL = 73,

    /// Do not output CR at column 0.
    ///
    /// Value: 0 (output) or 1 (do not output).
    ONOCR = 74,

    /// NL also performs the CR function.
    ///
    /// Value: 0 (do not perform) or 1 (perform).
    ONLRET = 75,

    // ========== 控制标志类 (90-93) ==========
    /// Use 7 data bits.
    ///
    /// Value: 0 (do not use) or 1 (use).
    CS7 = 90,

    /// Use 8 data bits.
    ///
    /// Value: 0 (do not use) or 1 (use).
    CS8 = 91,

    /// Enable parity.
    ///
    /// Value: 0 (off) or 1 (on).
    PARENB = 92,

    /// Odd parity instead of even parity.
    ///
    /// Value: 0 (even parity) or 1 (odd parity).
    PARODD = 93,

    // ========== 波特率类 (128-129) ==========
    /// Input baud rate.
    ///
    /// Value: baud rate value (0-230400).
    TtyOpISpeed = 128,

    /// Output baud rate.
    ///
    /// Value: baud rate value (0-230400).
    TtyOpOSpeed = 129,
}

impl TtyOpcode {
    /// Creates a `TtyOpcode` from its raw `u8` value, or `None` if the value
    /// is not a known opcode.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::TtyOpEnd),
            1 => Some(Self::VIntr),
            2 => Some(Self::VQuit),
            3 => Some(Self::VErase),
            4 => Some(Self::VKill),
            5 => Some(Self::VEOF),
            6 => Some(Self::VEOL),
            7 => Some(Self::VEOL2),
            8 => Some(Self::VStart),
            9 => Some(Self::VStop),
            10 => Some(Self::VSusp),
            11 => Some(Self::VDSusp),
            12 => Some(Self::VReprint),
            13 => Some(Self::VWerase),
            14 => Some(Self::VLNext),
            15 => Some(Self::VFlush),
            16 => Some(Self::VSwitch),
            17 => Some(Self::VStatus),
            18 => Some(Self::VDiscard),
            30 => Some(Self::IGNPAR),
            31 => Some(Self::PARMRK),
            32 => Some(Self::INPCK),
            33 => Some(Self::ISTRIP),
            34 => Some(Self::INLCR),
            35 => Some(Self::IGNCR),
            36 => Some(Self::ICRNL),
            37 => Some(Self::IUCLC),
            38 => Some(Self::IXON),
            39 => Some(Self::IXANY),
            40 => Some(Self::IXOFF),
            41 => Some(Self::IMAXBEL),
            42 => Some(Self::IUTF8),
            50 => Some(Self::ISIG),
            51 => Some(Self::ICANON),
            52 => Some(Self::XCASE),
            53 => Some(Self::ECHO),
            54 => Some(Self::ECHOE),
            55 => Some(Self::ECHOK),
            56 => Some(Self::ECHONL),
            57 => Some(Self::NOFLSH),
            58 => Some(Self::TOSTOP),
            59 => Some(Self::IEXTEN),
            60 => Some(Self::ECHOCTL),
            61 => Some(Self::ECHOKE),
            62 => Some(Self::PENDIN),
            70 => Some(Self::OPOST),
            71 => Some(Self::OLCUC),
            72 => Some(Self::ONLCR),
            73 => Some(Self::OCRNL),
            74 => Some(Self::ONOCR),
            75 => Some(Self::ONLRET),
            90 => Some(Self::CS7),
            91 => Some(Self::CS8),
            92 => Some(Self::PARENB),
            93 => Some(Self::PARODD),
            128 => Some(Self::TtyOpISpeed),
            129 => Some(Self::TtyOpOSpeed),
            _ => None,
        }
    }

    /// Returns the canonical name of the opcode (e.g. `"VINTR"`,
    /// `"TTY_OP_END"`).
    pub fn name(&self) -> &'static str {
        match self {
            Self::TtyOpEnd => "TTY_OP_END",
            Self::VIntr => "VINTR",
            Self::VQuit => "VQUIT",
            Self::VErase => "VERASE",
            Self::VKill => "VKILL",
            Self::VEOF => "VEOF",
            Self::VEOL => "VEOL",
            Self::VEOL2 => "VEOL2",
            Self::VStart => "VSTART",
            Self::VStop => "VSTOP",
            Self::VSusp => "VSUSP",
            Self::VDSusp => "VDSUSP",
            Self::VReprint => "VREPRINT",
            Self::VWerase => "VWERASE",
            Self::VLNext => "VLNEXT",
            Self::VFlush => "VFLUSH",
            Self::VSwitch => "VSWTCH",
            Self::VStatus => "VSTATUS",
            Self::VDiscard => "VDISCARD",
            Self::IGNPAR => "IGNPAR",
            Self::PARMRK => "PARMRK",
            Self::INPCK => "INPCK",
            Self::ISTRIP => "ISTRIP",
            Self::INLCR => "INLCR",
            Self::IGNCR => "IGNCR",
            Self::ICRNL => "ICRNL",
            Self::IUCLC => "IUCLC",
            Self::IXON => "IXON",
            Self::IXANY => "IXANY",
            Self::IXOFF => "IXOFF",
            Self::IMAXBEL => "IMAXBEL",
            Self::IUTF8 => "IUTF8",
            Self::ISIG => "ISIG",
            Self::ICANON => "ICANON",
            Self::XCASE => "XCASE",
            Self::ECHO => "ECHO",
            Self::ECHOE => "ECHOE",
            Self::ECHOK => "ECHOK",
            Self::ECHONL => "ECHONL",
            Self::NOFLSH => "NOFLSH",
            Self::TOSTOP => "TOSTOP",
            Self::IEXTEN => "IEXTEN",
            Self::ECHOCTL => "ECHOCTL",
            Self::ECHOKE => "ECHOKE",
            Self::PENDIN => "PENDIN",
            Self::OPOST => "OPOST",
            Self::OLCUC => "OLCUC",
            Self::ONLCR => "ONLCR",
            Self::OCRNL => "OCRNL",
            Self::ONOCR => "ONOCR",
            Self::ONLRET => "ONLRET",
            Self::CS7 => "CS7",
            Self::CS8 => "CS8",
            Self::PARENB => "PARENB",
            Self::PARODD => "PARODD",
            Self::TtyOpISpeed => "TTY_OP_ISPEED",
            Self::TtyOpOSpeed => "TTY_OP_OSPEED",
        }
    }

    /// Returns a short human-readable (Chinese) description of the opcode.
    pub fn description(&self) -> &'static str {
        match self {
            Self::TtyOpEnd => "结束标记",
            Self::VIntr => "中断信号 (Ctrl+C)",
            Self::VQuit => "退出信号 (Ctrl+\\)",
            Self::VErase => "擦除字符 (Backspace)",
            Self::VKill => "删除整行 (Ctrl+U)",
            Self::VEOF => "文件结束 (Ctrl+D)",
            Self::VEOL => "额外行结束",
            Self::VEOL2 => "第二个额外行结束",
            Self::VStart => "恢复输出 (Ctrl+Q)",
            Self::VStop => "停止输出 (Ctrl+S)",
            Self::VSusp => "挂起信号 (Ctrl+Z)",
            Self::VDSusp => "延迟挂起 (Ctrl+Y)",
            Self::VReprint => "重新打印行 (Ctrl+R)",
            Self::VWerase => "删除单词 (Ctrl+W)",
            Self::VLNext => "字面量下一个 (Ctrl+V)",
            Self::VFlush => "刷新输出",
            Self::VSwitch => "切换 shell 层",
            Self::VStatus => "状态请求 (Ctrl+T)",
            Self::VDiscard => "丢弃输出",
            Self::IGNPAR => "忽略奇偶校验错误",
            Self::PARMRK => "标记奇偶校验错误",
            Self::INPCK => "启用输入奇偶校验",
            Self::ISTRIP => "剥除第 8 位",
            Self::INLCR => "输入 NL→CR",
            Self::IGNCR => "忽略输入 CR",
            Self::ICRNL => "输入 CR→NL",
            Self::IUCLC => "输入大写→小写",
            Self::IXON => "启用输出 XON/XOFF 流控",
            Self::IXANY => "任意字符恢复输出",
            Self::IXOFF => "启用输入 XON/XOFF 流控",
            Self::IMAXBEL => "输入队列满时响铃",
            Self::IUTF8 => "输入为 UTF-8",
            Self::ISIG => "启用信号",
            Self::ICANON => "规范模式（行缓冲）",
            Self::XCASE => "大小写转换",
            Self::ECHO => "回显输入字符",
            Self::ECHOE => "回显擦除字符",
            Self::ECHOK => "回显 kill 字符",
            Self::ECHONL => "回显换行符",
            Self::NOFLSH => "不清空队列",
            Self::TOSTOP => "后台写入停止",
            Self::IEXTEN => "扩展功能",
            Self::ECHOCTL => "回显控制字符为 ^X",
            Self::ECHOKE => "回显 kill 擦除",
            Self::PENDIN => "待处理输入",
            Self::OPOST => "输出后处理",
            Self::OLCUC => "输出小写→大写",
            Self::ONLCR => "输出 NL→CR-NL",
            Self::OCRNL => "输出 CR→NL",
            Self::ONOCR => "第 0 列无 CR",
            Self::ONLRET => "NL 执行 CR",
            Self::CS7 => "7 位数据位",
            Self::CS8 => "8 位数据位",
            Self::PARENB => "启用奇偶校验",
            Self::PARODD => "奇校验",
            Self::TtyOpISpeed => "输入波特率",
            Self::TtyOpOSpeed => "输出波特率",
        }
    }

    /// Returns `true` if this is a special-character opcode (values 1-18).
    pub fn is_special_char(&self) -> bool {
        (*self as u8) >= 1 && (*self as u8) <= 18
    }

    /// Returns `true` if this is an input-flag opcode (values 30-42).
    pub fn is_input_flag(&self) -> bool {
        (*self as u8) >= 30 && (*self as u8) <= 42
    }

    /// Returns `true` if this is a local-flag opcode (values 50-62).
    pub fn is_local_flag(&self) -> bool {
        (*self as u8) >= 50 && (*self as u8) <= 62
    }

    /// Returns `true` if this is an output-flag opcode (values 70-75).
    pub fn is_output_flag(&self) -> bool {
        (*self as u8) >= 70 && (*self as u8) <= 75
    }

    /// Returns `true` if this is a control-flag opcode (values 90-93).
    pub fn is_control_flag(&self) -> bool {
        (*self as u8) >= 90 && (*self as u8) <= 93
    }

    /// Returns `true` if this is a baud-rate opcode (values 128-129).
    pub fn is_speed(&self) -> bool {
        (*self as u8) >= 128 && (*self as u8) <= 129
    }

    /// Returns a human-readable (Chinese) description of the value type
    /// expected for this opcode.
    pub fn value_type(&self) -> &'static str {
        match self {
            Self::TtyOpEnd => "无",
            _ if self.is_special_char() => "0-127 (ASCII), 255 (禁用)",
            _ if self.is_input_flag() => "0 或 1 (布尔值)",
            _ if self.is_local_flag() => "0 或 1 (布尔值)",
            _ if self.is_output_flag() => "0 或 1 (布尔值)",
            _ if self.is_control_flag() => "0 或 1 (布尔值)",
            _ if self.is_speed() => "0-230400 (波特率)",
            _ => "未知",
        }
    }
}

/// Common values of the terminal special characters (the `V*` opcodes), as
/// ASCII control codes.
pub mod special_chars {
    /// `Ctrl+C` (interrupt).
    pub const CTRL_C: u32 = 3;
    /// `Ctrl+\` (quit).
    pub const CTRL_BACKSLASH: u32 = 28;
    /// `Ctrl+D` (end of file).
    pub const CTRL_D: u32 = 4;
    /// `Ctrl+U` (kill the whole line).
    pub const CTRL_U: u32 = 21;
    /// `Ctrl+Z` (suspend).
    pub const CTRL_Z: u32 = 26;
    /// `Ctrl+Q` (resume output).
    pub const CTRL_Q: u32 = 17;
    /// `Ctrl+S` (stop output).
    pub const CTRL_S: u32 = 19;
    /// `Ctrl+R` (reprint the line).
    pub const CTRL_R: u32 = 18;
    /// `Ctrl+W` (erase a word).
    pub const CTRL_W: u32 = 23;
    /// `Ctrl+V` (literal next character).
    pub const CTRL_V: u32 = 22;
    /// `Ctrl+Y` (delayed suspend).
    pub const CTRL_Y: u32 = 25;
    /// `Ctrl+T` (status request).
    pub const CTRL_T: u32 = 20;
    /// `Ctrl+O` (discard output).
    pub const CTRL_O: u32 = 15;
    /// `Backspace`.
    pub const BACKSPACE: u32 = 127;
    /// `Ctrl+H` (alternative backspace).
    pub const CTRL_H: u32 = 8;
    /// The value that disables a special character.
    pub const DISABLED: u32 = 255;
}

/// Standard baud rate values for the [`TtyOpcode::TtyOpISpeed`] and
/// [`TtyOpcode::TtyOpOSpeed`] terminal modes.
pub mod baud_rates {
    /// 0 baud (hang-up).
    pub const B0: u32 = 0;
    /// 50 baud.
    pub const B50: u32 = 50;
    /// 75 baud.
    pub const B75: u32 = 75;
    /// 110 baud.
    pub const B110: u32 = 110;
    /// 134.5 baud.
    pub const B134: u32 = 134;
    /// 150 baud.
    pub const B150: u32 = 150;
    /// 200 baud.
    pub const B200: u32 = 200;
    /// 300 baud.
    pub const B300: u32 = 300;
    /// 600 baud.
    pub const B600: u32 = 600;
    /// 1200 baud.
    pub const B1200: u32 = 1200;
    /// 1800 baud.
    pub const B1800: u32 = 1800;
    /// 2400 baud.
    pub const B2400: u32 = 2400;
    /// 4800 baud.
    pub const B4800: u32 = 4800;
    /// 9600 baud.
    pub const B9600: u32 = 9600;
    /// 19200 baud.
    pub const B19200: u32 = 19200;
    /// 38400 baud.
    pub const B38400: u32 = 38400;
    /// 57600 baud.
    pub const B57600: u32 = 57600;
    /// 115200 baud.
    pub const B115200: u32 = 115200;
    /// 230400 baud.
    pub const B230400: u32 = 230400;
}

/// Parser and encoder for the terminal modes payload of a `pty-req` channel
/// request.
pub struct TtyModesParser;

impl TtyModesParser {
    /// Parses terminal modes from their wire format: a sequence of
    /// `byte opcode` + `uint32 value` entries (the value in network byte
    /// order), terminated by opcode 0.
    ///
    /// Unknown opcodes are skipped (after printing a warning), and parsing
    /// also stops at the end of `data` or at an entry whose value is
    /// truncated.
    ///
    /// # Example
    ///
    /// ```
    /// use flatline::session::channel::{TtyModesParser, TtyOpcode};
    ///
    /// let data = [
    ///     0x01, 0x00, 0x00, 0x00, 0x03, // VINTR = 3
    ///     0x35, 0x00, 0x00, 0x00, 0x01, // ECHO = 1
    ///     0x00,                         // TTY_OP_END
    /// ];
    /// let modes = TtyModesParser::parse(&data);
    /// assert_eq!(modes.len(), 2);
    /// assert_eq!(modes[0], (TtyOpcode::VIntr, 3));
    /// assert_eq!(modes[1], (TtyOpcode::ECHO, 1));
    /// ```
    pub fn parse(data: &[u8]) -> Vec<(TtyOpcode, u32)> {
        let mut result = Vec::new();
        let mut i = 0;

        while i < data.len() {
            let opcode = data[i];
            i += 1;

            // 结束标记
            if opcode == 0 {
                break;
            }

            // 读取 value (4 字节，大端序)
            if i + 4 > data.len() {
                break;
            }
            let value = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
            i += 4;

            // 尝试转换为 TtyOpcode
            if let Some(op) = TtyOpcode::from_u8(opcode) {
                result.push((op, value));
            } else {
                // 未知 opcode，跳过
                eprintln!("Unknown TTY opcode: {}", opcode);
            }
        }

        result
    }

    /// Encodes terminal modes into their wire format: each entry becomes a
    /// `byte opcode` followed by a big-endian `uint32 value`, and the result
    /// is terminated by opcode 0.
    pub fn encode(modes: &[(TtyOpcode, u32)]) -> Vec<u8> {
        let mut result = Vec::new();

        for (opcode, value) in modes {
            result.push(*opcode as u8);
            result.extend_from_slice(&value.to_be_bytes());
        }

        // 结束标记
        result.push(0);

        result
    }
}

/// Terminal mode presets that can be passed to [`Channel::request_pty`].
pub mod presets {
    use super::*;

    /// Interactive terminal mode: echoing and output post-processing enabled,
    /// with the usual control characters.
    pub fn interactive_terminal() -> Vec<(TtyOpcode, u32)> {
        vec![
            (TtyOpcode::TtyOpOSpeed, 9600),
            (TtyOpcode::TtyOpISpeed, 9600),
            (TtyOpcode::VIntr, special_chars::CTRL_C),
            (TtyOpcode::VQuit, special_chars::CTRL_BACKSLASH),
            (TtyOpcode::VErase, special_chars::BACKSPACE),
            (TtyOpcode::VKill, special_chars::CTRL_U),
            (TtyOpcode::VEOF, special_chars::CTRL_D),
            (TtyOpcode::VStart, special_chars::CTRL_Q),
            (TtyOpcode::VStop, special_chars::CTRL_S),
            (TtyOpcode::VSusp, special_chars::CTRL_Z),
            (TtyOpcode::VReprint, special_chars::CTRL_R),
            (TtyOpcode::VWerase, special_chars::CTRL_W),
            (TtyOpcode::VLNext, special_chars::CTRL_V),
            (TtyOpcode::ISIG, 1),
            (TtyOpcode::ICANON, 1),
            (TtyOpcode::ECHO, 1),
            (TtyOpcode::ECHOE, 1),
            (TtyOpcode::ECHOK, 1),
            (TtyOpcode::IEXTEN, 1),
            (TtyOpcode::OPOST, 1),
            (TtyOpcode::ONLCR, 1),
        ]
    }

    /// Password input mode (echoing disabled).
    pub fn password_input() -> Vec<(TtyOpcode, u32)> {
        vec![
            (TtyOpcode::ISIG, 1),
            (TtyOpcode::ICANON, 1),
            (TtyOpcode::ECHO, 0), // 不回显
            (TtyOpcode::IEXTEN, 1),
            (TtyOpcode::OPOST, 1),
            (TtyOpcode::ONLCR, 1),
        ]
    }

    /// Raw mode (no canonical processing, no echoing, no flow control).
    pub fn raw_mode() -> Vec<(TtyOpcode, u32)> {
        vec![
            (TtyOpcode::ISIG, 0),   // 禁用信号
            (TtyOpcode::ICANON, 0), // 非规范模式
            (TtyOpcode::ECHO, 0),   // 不回显
            (TtyOpcode::IXON, 0),   // 禁用流控
            (TtyOpcode::ICRNL, 0),  // 不转换 CR
            (TtyOpcode::OPOST, 0),  // 不处理输出
            (TtyOpcode::ONLCR, 0),  // 不转换 NL
        ]
    }

    /// Serial communication mode (9600 baud, 8 data bits, even parity).
    pub fn serial_communication() -> Vec<(TtyOpcode, u32)> {
        vec![
            (TtyOpcode::TtyOpOSpeed, 9600),
            (TtyOpcode::TtyOpISpeed, 9600),
            (TtyOpcode::CS8, 1),    // 8 位数据位
            (TtyOpcode::PARENB, 1), // 启用奇偶校验
            (TtyOpcode::PARODD, 0), // 偶校验
        ]
    }
}

/// The local and remote channel numbers that identify a channel on the wire.
///
/// Each side picks its own number for a channel, so both numbers are needed
/// to address it in SSH messages.
#[derive(Clone, Copy, Hash, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct IdentityPair {
    /// The channel number assigned by this client (the local number).
    pub client: u32,
    /// The channel number assigned by the server (the remote number).
    pub server: u32,
}

impl IdentityPair {
    pub(super) fn new(client: u32, server: u32) -> IdentityPair {
        Self { client, server }
    }
}

/// A handle to an SSH connection-layer channel.
///
/// Obtain one from
/// [`Session::channel_open_default`](crate::session::Session::channel_open_default)
/// or [`Session::channel_open`](crate::session::Session::channel_open).
/// Outgoing data is written with [`Channel::send`] and incoming data arrives
/// as [`Message`]s read with [`Channel::receive`] or [`Channel::try_receive`].
///
/// Dropping the handle closes the channel (best effort); use [`Channel::close`]
/// to wait until the close has been carried out, or [`Channel::eof`] to only
/// signal that no more data will be sent.
#[derive(derive_more::Debug)]
pub struct Channel {
    id: IdentityPair,
    #[debug(skip)]
    receiver: mpsc::Receiver<Message>,
    #[debug(skip)]
    sender: mpsc::Sender<Event>,
    closed: bool,
}

impl Drop for Channel {
    fn drop(&mut self) {
        if self.closed {
            return;
        }

        let (sender, mut receiver) = oneshot::channel();
        let event = Event::ChannelClose {
            channel_id: self.id,
            back: sender,
        };

        if let Err(err) = self.sender.try_send(event) {
            match err {
                TrySendError::Full(_) => {
                    tracing::warn!("Failed to close channel")
                }
                TrySendError::Closed(_) => {
                    tracing::warn!("Channel is shutdown")
                }
            }
            return;
        }

        self.closed = true;

        if receiver.try_recv().is_err() {
            tracing::debug!("Failed to wait for channel closed")
        }
    }
}

impl Channel {
    pub(super) fn new(
        id: IdentityPair,
        receiver: mpsc::Receiver<Message>,
        sender: mpsc::Sender<super::Event>,
    ) -> Self {
        Self {
            id,
            receiver,
            sender,
            closed: false,
        }
    }

    /// Returns the local and remote channel numbers of this channel.
    pub fn identity(&self) -> IdentityPair {
        self.id
    }

    /// Signals that this side will send no more data (`SSH_MSG_CHANNEL_EOF`).
    ///
    /// The channel stays open: this peer may still send data, and this side
    /// may still read it. Returns an error if the channel is unknown or the
    /// session is shutting down.
    pub async fn eof(&self) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelEof {
            channel_id: self.id,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Closes the channel (`SSH_MSG_CHANNEL_CLOSE`), consuming the handle.
    ///
    /// Closing is final; use [`Channel::eof`] to only signal that no more
    /// data will be sent. Returns an error if the channel is unknown or the
    /// session is shutting down.
    pub async fn close(mut self) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelClose {
            channel_id: self.id,
            back: sender,
        };

        self.sender.send_next(event).await?;

        self.closed = true;

        receiver.receive_next().await?
    }

    /// Returns the next message without blocking.
    ///
    /// Yields `Ok(None)` when no message is pending, and an error when the
    /// session has shut down.
    pub fn try_receive(&mut self) -> error::Result<Option<Message>> {
        match self.receiver.try_recv() {
            Ok(v) => Ok(Some(v)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(super::UnexpectedBehaviourSnafu {
                detail: "Maybe session was shutdown",
            }
            .build()
            .into()),
        }
    }

    /// Waits for and returns the next message from the server.
    ///
    /// Returns an error when the session has shut down.
    pub async fn receive(&mut self) -> error::Result<Message> {
        let msg = self
            .receiver
            .recv()
            .await
            .context(super::UnexpectedBehaviourSnafu {
                detail: "Maybe session was shutdown",
            })?;
        Ok(msg)
    }

    /// Sends `data` on the channel, returning how many bytes were accepted.
    ///
    /// How much can be written in one call is limited by the peer's receive
    /// window and maximum packet size, so the returned count may be smaller
    /// than `data.len()`; callers must handle this partial write by sending
    /// the remaining bytes again. Empty input returns `Ok(0)` without sending
    /// anything. Returns an error if the channel is unknown or the session is
    /// shutting down.
    pub async fn send(&self, data: impl Into<Vec<u8>>) -> error::Result<usize> {
        let data = data.into();
        if data.is_empty() {
            return Ok(0);
        }
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelSendData {
            channel_id: self.id,
            data,
            back: sender,
        };
        self.sender.send_next(event).await?;
        let size = receiver.receive_next().await??;
        Ok(size)
    }

    /// Sends an `x11-req` channel request, asking the server to forward X11
    /// connections from the remote display to this client.
    ///
    /// `want_reply` indicates whether the peer should answer the channel
    /// request; when `true`, the call waits for `SSH_MSG_CHANNEL_SUCCESS` or
    /// `SSH_MSG_CHANNEL_FAILURE` and returns an error for the latter (and if
    /// the channel is closed while waiting). `single_connection` asks that
    /// only one connection be forwarded, `protocol` names the authorisation
    /// protocol (e.g. `"MIT-MAGIC-COOKIE-1"`), `cookie` carries its cookie,
    /// and `screen` selects the screen on the remote display.
    pub async fn request_x11(
        &mut self,
        want_reply: bool,
        single_connection: bool,
        protocol: impl Into<String>,
        cookie: impl Into<String>,
        screen: u32,
    ) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelRequestX11 {
            channel_id: self.id,
            want_reply,
            single_connection,
            protocol: protocol.into(),
            cookie: cookie.into(),
            screen,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Sends an `env` channel request, setting the environment variable
    /// `name` to `value` for the shell or command started on this channel.
    ///
    /// `want_reply` indicates whether the peer should answer the channel
    /// request; when `true`, the call waits for `SSH_MSG_CHANNEL_SUCCESS` or
    /// `SSH_MSG_CHANNEL_FAILURE` and returns an error for the latter (and if
    /// the channel is closed while waiting). Servers are free to ignore `env`
    /// requests.
    pub async fn request_env(
        &mut self,
        want_reply: bool,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelRequestEnv {
            channel_id: self.id,
            want_reply,
            name: name.into(),
            value: value.into(),
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Sends a `signal` channel request, asking the server to deliver
    /// `signal` (a name such as `"TERM"` or `"INT"`, without the `SIG`
    /// prefix) to the process running on this channel.
    ///
    /// `want_reply` indicates whether the peer should answer the channel
    /// request; when `true`, the call waits for `SSH_MSG_CHANNEL_SUCCESS` or
    /// `SSH_MSG_CHANNEL_FAILURE` and returns an error for the latter (and if
    /// the channel is closed while waiting).
    pub async fn request_signal(&self, want_reply: bool, signal: Signal) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelRequestSignal {
            channel_id: self.id,
            want_reply,
            signal: signal.0,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Sends a `window-change` channel request reporting a new terminal size.
    ///
    /// `columns` and `rows` are the size in characters, `width` and `height`
    /// the size in pixels. RFC 4254 specifies `want_reply = FALSE` for this
    /// request, so no reply is awaited and only transport errors surface.
    pub async fn request_window_change(
        &self,
        columns: u32,
        rows: u32,
        width: u32,
        height: u32,
    ) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelRequestWindowChange {
            channel_id: self.id,
            columns,
            rows,
            width,
            height,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Sends an `exec` channel request asking the server to run `command`.
    ///
    /// `want_reply` indicates whether the peer should answer the channel
    /// request; when `true`, the call waits for `SSH_MSG_CHANNEL_SUCCESS` or
    /// `SSH_MSG_CHANNEL_FAILURE` and returns an error for the latter (and if
    /// the channel is closed while waiting). The command's output then
    /// arrives as [`Message::Stdout`] and [`Message::Stderr`] messages,
    /// usually followed by [`Message::Exit`].
    pub async fn request_exec(
        &self,
        want_reply: bool,
        command: impl Into<String>,
    ) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelRequestExec {
            channel_id: self.id,
            want_reply,
            command: command.into(),
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Sends a `shell` channel request asking the server to start an
    /// interactive shell on this channel.
    ///
    /// `want_reply` indicates whether the peer should answer the channel
    /// request; when `true`, the call waits for `SSH_MSG_CHANNEL_SUCCESS` or
    /// `SSH_MSG_CHANNEL_FAILURE` and returns an error for the latter (and if
    /// the channel is closed while waiting).
    pub async fn request_shell(&self, want_reply: bool) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelRequestShell {
            channel_id: self.id,
            want_reply,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Sends an `agent-req` channel request, asking the server to forward an
    /// ssh-agent connection to this client (an OpenSSH extension).
    ///
    /// `want_reply` indicates whether the peer should answer the channel
    /// request; when `true`, the call waits for `SSH_MSG_CHANNEL_SUCCESS` or
    /// `SSH_MSG_CHANNEL_FAILURE` and returns an error for the latter (and if
    /// the channel is closed while waiting).
    pub async fn request_agent(&self, want_reply: bool) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelRequestAgent {
            channel_id: self.id,
            want_reply,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Sends a `break` channel request asking the server to signal a break
    /// to the remote application, for at most `milliseconds` (0 means "as
    /// long as possible").
    ///
    /// `want_reply` indicates whether the peer should answer the channel
    /// request; when `true`, the call waits for `SSH_MSG_CHANNEL_SUCCESS` or
    /// `SSH_MSG_CHANNEL_FAILURE` and returns an error for the latter (and if
    /// the channel is closed while waiting).
    pub async fn request_break(&self, want_reply: bool, milliseconds: u32) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelRequestBreak {
            channel_id: self.id,
            want_reply,
            milliseconds,
            back: sender,
        };
        self.sender.send_next(event).await?;
        receiver.receive_next().await?
    }

    /// Sends a `pty-req` channel request, asking the server to allocate a
    /// pseudo-terminal for this channel.
    ///
    /// `terminal` is the TERM value (e.g. `"xterm-256color"`), `columns` and
    /// `rows` the window size in characters, `width` and `height` in pixels,
    /// and `modes` the initial terminal modes (RFC 4254 §8), encoded as
    /// opcode/value pairs terminated by [`TtyOpcode::TtyOpEnd`].
    /// `want_reply` indicates whether the peer should answer the channel
    /// request; when `true`, the call waits for `SSH_MSG_CHANNEL_SUCCESS` or
    /// `SSH_MSG_CHANNEL_FAILURE` and returns an error for the latter (and if
    /// the channel is closed while waiting).
    pub async fn request_pty(
        &self,
        terminal: impl Into<String>,
        want_reply: bool,
        columns: u32,
        rows: u32,
        width: u32,
        height: u32,
        modes: Vec<(TtyOpcode, u32)>,
    ) -> error::Result<()> {
        let terminal = terminal.into();
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelRequestPty {
            channel_id: self.id,
            terminal,
            columns,
            rows,
            width,
            height,
            modes,
            back: sender,
            want_reply,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }
}

/// A buffering adapter around a [`Channel`].
///
/// It batches outgoing writes in an internal buffer (taking care of the
/// partial writes of [`Channel::send`]) and accumulates incoming stdout data
/// in an internal read buffer, so callers can work with lines and fixed-size
/// chunks without tracking the channel's state themselves.
#[derive(derive_more::Debug)]
pub struct BufferChannel {
    channel: channel::Channel,
    #[debug(skip)]
    write_buf: BytesMut,
    #[debug(skip)]
    read_buf: BytesMut,
}

impl BufferChannel {
    /// Wraps the given [`Channel`] with empty read and write buffers.
    pub fn new(channel: channel::Channel) -> Self {
        Self {
            channel,
            write_buf: Default::default(),
            read_buf: Default::default(),
        }
    }

    /// Returns a mutable reference to the wrapped [`Channel`], e.g. to send
    /// channel requests or read messages directly.
    pub fn channel_mut(&mut self) -> &mut channel::Channel {
        &mut self.channel
    }

    /// Sends the write buffer until it is fully drained.
    ///
    /// While the peer's window prevents a full write, incoming messages are
    /// read to make progress (stdout data is appended to the read buffer);
    /// this may therefore block. Returns `Ok(())` immediately when the write
    /// buffer is already empty, and an error on transport failures or an
    /// unexpected [`Message::Close`].
    pub async fn flush(&mut self) -> error::Result<()> {
        if self.write_buf.is_empty() {
            return Ok(());
        }

        let mut pos = 0;
        loop {
            let written = self.channel.send(&self.write_buf[pos..]).await?;

            pos += written;

            if pos == self.write_buf.len() {
                self.write_buf.clear();
                break;
            } else {
                loop {
                    match self.channel.receive().await? {
                        Message::Close => {
                            return Err(super::UnexpectedMessageSnafu {
                                detail: "Unexpected close message",
                            }
                            .build()
                            .into());
                        }
                        Message::Eof => {}
                        Message::Stdout(data) => {
                            self.read_buf.extend_from_slice(&data[..]);
                        }
                        Message::Stderr(_) => {
                            tracing::warn!("Unexpected stderr message");
                        }
                        Message::Exit(status) => {
                            tracing::info!("Unexpected exit status: {:?}", status);
                        }
                        Message::FlowControl { .. } => {}
                        Message::WindowChange { .. } => {
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Queues `data` for sending, forwarding it to the channel right away
    /// when possible.
    ///
    /// Bytes the peer's window could not accept remain in the write buffer
    /// and go out on the next [`BufferChannel::flush`] (or `send`).
    pub async fn send(&mut self, data: &[u8]) -> error::Result<()> {
        if self.write_buf.is_empty() {
            let size = self.channel.send(data).await?;
            if size < data.len() {
                self.write_buf.extend_from_slice(&data[size..]);
            }
        } else {
            self.write_buf.extend_from_slice(data);

            let written = self.channel.send(&self.write_buf[..]).await?;

            self.write_buf.advance(written);
        }

        Ok(())
    }

    /// Drops the first `len` bytes of the read buffer, marking them as
    /// consumed.
    ///
    /// Does nothing when `len` is `0` or the read buffer is empty; otherwise
    /// panics if the buffer holds fewer than `len` bytes.
    pub fn consumer_read_buffer(&mut self, len: usize) {
        if len == 0 || self.read_buf.is_empty() {
            return;
        }

        assert!(self.read_buf.len() >= len);

        self.read_buf.advance(len);
    }

    /// Waits until new stdout data has arrived and appends it to the read
    /// buffer.
    ///
    /// Other messages are skipped (and logged), while [`Message::Close`] and
    /// [`Message::Eof`] are reported as errors, as are transport failures.
    pub async fn fill_once(&mut self) -> error::Result<()> {
        loop {
            let msg = self.channel.receive().await?;
            match msg {
                Message::Close => {
                    tracing::warn!("Unexpected close message");
                    return Err(super::UnexpectedMessageSnafu {
                        detail: "Unexpected close message",
                    }
                    .build()
                    .into());
                }
                Message::Eof => {
                    tracing::warn!("Unexpected eof message");
                    return Err(super::UnexpectedMessageSnafu {
                        detail: "Unexpected eof message",
                    }
                    .build()
                    .into());
                }
                Message::Stdout(data) => {
                    self.read_buf.extend_from_slice(&data[..]);
                    break Ok(());
                }
                Message::Stderr(_) => {
                    tracing::warn!("Unexpected stderr message");
                }
                Message::Exit(status) => {
                    tracing::info!("Unexpected exit message: {:?}", status);
                }
                Message::FlowControl { .. } => {
                    tracing::info!("Unexpected flow control message");
                }
                Message::WindowChange { .. } => {}
            }
        }
    }

    /// Waits until the read buffer holds any data and returns it, without
    /// consuming it.
    ///
    /// Propagates errors from [`BufferChannel::fill_once`], e.g. when the
    /// channel closes or reaches EOF before data arrives.
    pub async fn fill(&mut self) -> error::Result<&[u8]> {
        while self.read_buf.is_empty() {
            self.fill_once().await?;
        }
        Ok(&self.read_buf[..])
    }

    /// Waits until the read buffer holds at least `len` bytes and returns
    /// the first `len` of them, without consuming them.
    ///
    /// Propagates errors from [`BufferChannel::fill_once`], e.g. when the
    /// channel closes or reaches EOF before enough data arrives.
    pub async fn fill_exact(&mut self, len: usize) -> error::Result<&[u8]> {
        while self.read_buf.len() < len {
            self.fill_once().await?;
        }

        Ok(&self.read_buf[..len])
    }

    /// Waits until the read buffer contains a `\n` and returns everything up
    /// to and including it, without consuming it.
    ///
    /// Propagates errors from [`BufferChannel::fill_once`], e.g. when the
    /// channel closes or reaches EOF before a newline arrives.
    pub async fn read_line_lf(&mut self) -> error::Result<&[u8]> {
        let mut pos = 0;
        loop {
            for i in pos..self.read_buf.len() {
                if self.read_buf[i] == b'\n' {
                    return Ok(&self.read_buf[..=i]);
                }
            }
            pos = self.read_buf.len();
            self.fill_once().await?;
        }
    }

    /// Closes the underlying channel, consuming this handle.
    ///
    /// Data still sitting in the write buffer is not sent. Returns an error
    /// if the channel is unknown or the session is shutting down.
    pub async fn close(self) -> error::Result<()> {
        self.channel.close().await
    }
}
#[cfg(test)]

mod test {
    use super::*;

    #[test]
    fn test_opcode_from_u8() {
        assert_eq!(TtyOpcode::from_u8(0), Some(TtyOpcode::TtyOpEnd));
        assert_eq!(TtyOpcode::from_u8(1), Some(TtyOpcode::VIntr));
        assert_eq!(TtyOpcode::from_u8(53), Some(TtyOpcode::ECHO));
        assert_eq!(TtyOpcode::from_u8(129), Some(TtyOpcode::TtyOpOSpeed));
        assert_eq!(TtyOpcode::from_u8(200), None); // 未知 opcode
    }

    #[test]
    fn test_opcode_name() {
        assert_eq!(TtyOpcode::VIntr.name(), "VINTR");
        assert_eq!(TtyOpcode::ECHO.name(), "ECHO");
        assert_eq!(TtyOpcode::TtyOpEnd.name(), "TTY_OP_END");
    }

    #[test]
    fn test_opcode_type_check() {
        assert!(TtyOpcode::VIntr.is_special_char());
        assert!(!TtyOpcode::ECHO.is_special_char());

        assert!(TtyOpcode::ICRNL.is_input_flag());
        assert!(!TtyOpcode::ECHO.is_input_flag());

        assert!(TtyOpcode::ECHO.is_local_flag());
        assert!(!TtyOpcode::ICRNL.is_local_flag());

        assert!(TtyOpcode::OPOST.is_output_flag());
        assert!(TtyOpcode::CS8.is_control_flag());
        assert!(TtyOpcode::TtyOpOSpeed.is_speed());
    }

    #[test]
    fn test_parse_modes() {
        // 构造测试数据
        let data = vec![
            0x01, 0x00, 0x00, 0x00, 0x03, // VINTR = 3
            0x35, 0x00, 0x00, 0x00, 0x01, // ECHO = 1
            0x00, // TTY_OP_END
        ];

        let modes = TtyModesParser::parse(&data);
        assert_eq!(modes.len(), 2);
        assert_eq!(modes[0], (TtyOpcode::VIntr, 3));
        assert_eq!(modes[1], (TtyOpcode::ECHO, 1));
    }

    #[test]
    fn test_encode_modes() {
        let modes = vec![(TtyOpcode::VIntr, 3), (TtyOpcode::ECHO, 1)];

        let data = TtyModesParser::encode(&modes);
        assert_eq!(
            data,
            vec![
                0x01, 0x00, 0x00, 0x00, 0x03, // VINTR = 3
                0x35, 0x00, 0x00, 0x00, 0x01, // ECHO = 1
                0x00, // TTY_OP_END
            ]
        );
    }

    #[test]
    fn test_presets() {
        let interactive = presets::interactive_terminal();
        assert!(!interactive.is_empty());

        let password = presets::password_input();
        // 密码模式应该禁用 ECHO
        let echo = password.iter().find(|(op, _)| *op == TtyOpcode::ECHO);
        assert!(echo.is_some());
        assert_eq!(echo.unwrap().1, 0);

        let raw = presets::raw_mode();
        // 原始模式应该禁用 ICANON
        let icanon = raw.iter().find(|(op, _)| *op == TtyOpcode::ICANON);
        assert!(icanon.is_some());
        assert_eq!(icanon.unwrap().1, 0);
    }

    // 使用示例
    #[test]
    fn test() {
        // 解析 terminal modes 数据
        let data = vec![
            0x01, 0x00, 0x00, 0x00, 0x03, // VINTR = 3 (Ctrl+C)
            0x35, 0x00, 0x00, 0x00, 0x01, // ECHO = 1
            0x33, 0x00, 0x00, 0x00, 0x01, // ICANON = 1
            0x00, // TTY_OP_END
        ];

        let modes = TtyModesParser::parse(&data);
        println!("Parsed terminal modes:");
        for (opcode, value) in &modes {
            println!(
                "  {} ({}): {} - {}",
                opcode.name(),
                *opcode as u8,
                value,
                opcode.description()
            );
        }

        // 使用预设
        println!("\nInteractive terminal preset:");
        let preset = presets::interactive_terminal();
        for (opcode, value) in &preset {
            println!("  {} = {}", opcode.name(), value);
        }

        // 编码
        let encoded = TtyModesParser::encode(&modes);
        println!("\nEncoded data: {:?}", encoded);
    }
}
