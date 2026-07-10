use super::{Event, UnexpectedReceivingError, UnexpectedSendingError, channel};
use crate::{error, ssh::msg::Signal};
use bytes::{Buf, BytesMut};
use snafu::OptionExt;
use tokio::sync::mpsc::error::{TryRecvError, TrySendError};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone)]
pub enum ExitStatus {
    Normal(u32),
    Interrupt {
        signal: Signal,
        core_dumped: bool,
        error_message: String,
    },
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        matches!(self, Self::Normal(0))
    }
}

#[derive(derive_more::Debug)]
pub enum Message {
    /// It means the channel was closed by server, it can't be read or written;
    Close,
    /// It means no more data will be sent by server;
    Eof,
    /// Obviously this is the standard output data, println!() in rust;
    Stdout(#[debug(skip)] Vec<u8>),
    /// Obviously this is the standard error data, eprintln!() in rust;
    Stderr(#[debug(skip)] Vec<u8>),
    /// When the channel::exec is called and the process ends, the server will send this to the client;
    /// it may be sent before the Eof
    Exit(ExitStatus),
    FlowControl {
        on: bool,
    },
    WindowChange {
        size: u32,
    },
}

/// SSH Terminal Modes Opcode 定义
///
/// 基于 RFC 4254 Section 8 和 OpenSSH 扩展
///
/// 参考文档：
/// - https://tools.ietf.org/html/rfc4254#section-8
/// - https://www.iana.org/assignments/ssh-parameters/ssh-parameters.xhtml#ssh-parameters-16
///   Terminal Modes Opcode 枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TtyOpcode {
    // ========== 结束标记 ==========
    /// 标记 terminal modes 数据的结束
    TtyOpEnd = 0,

    // ========== 特殊字符类 (1-18) ==========
    /// 中断信号字符 (通常为 Ctrl+C)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VIntr = 1,

    /// 退出信号字符 (通常为 Ctrl+\)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VQuit = 2,

    /// 擦除字符 (通常为 Backspace)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VErase = 3,

    /// 删除整行字符 (通常为 Ctrl+U)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VKill = 4,

    /// 文件结束字符 (通常为 Ctrl+D)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VEOF = 5,

    /// 额外的行结束字符
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VEOL = 6,

    /// 第二个额外的行结束字符
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VEOL2 = 7,

    /// 恢复输出字符 (通常为 Ctrl+Q)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VStart = 8,

    /// 停止输出字符 (通常为 Ctrl+S)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VStop = 9,

    /// 挂起信号字符 (通常为 Ctrl+Z)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VSusp = 10,

    /// 延迟挂起字符 (通常为 Ctrl+Y)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VDSusp = 11,

    /// 重新打印行字符 (通常为 Ctrl+R)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VReprint = 12,

    /// 删除单词字符 (通常为 Ctrl+W)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VWerase = 13,

    /// 字面量下一个字符 (通常为 Ctrl+V)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VLNext = 14,

    /// 刷新输出字符 (OpenSSH 扩展)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VFlush = 15,

    /// 切换 shell 层字符 (OpenSSH 扩展)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VSwitch = 16,

    /// 状态请求字符 (通常为 Ctrl+T)
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VStatus = 17,

    /// 丢弃输出字符
    /// Value: 0-127 (ASCII 字符值), 255 表示禁用
    VDiscard = 18,

    // ========== 输入标志类 (30-42) ==========
    /// 忽略奇偶校验错误
    /// Value: 0 (不忽略) 或 1 (忽略)
    IGNPAR = 30,

    /// 标记奇偶校验和帧错误
    /// Value: 0 (不标记) 或 1 (标记)
    PARMRK = 31,

    /// 启用输入奇偶校验
    /// Value: 0 (禁用) 或 1 (启用)
    INPCK = 32,

    /// 剥除第 8 位
    /// Value: 0 (不剥除) 或 1 (剥除)
    ISTRIP = 33,

    /// 将输入的 NL 转换为 CR
    /// Value: 0 (不转换) 或 1 (转换)
    INLCR = 34,

    /// 忽略输入的 CR
    /// Value: 0 (不忽略) 或 1 (忽略)
    IGNCR = 35,

    /// 将输入的 CR 转换为 NL
    /// Value: 0 (不转换) 或 1 (转换)
    ICRNL = 36,

    /// 将输入的大写转换为小写
    /// Value: 0 (不转换) 或 1 (转换)
    IUCLC = 37,

    /// 启用输出的 XON/XOFF 流控
    /// Value: 0 (禁用) 或 1 (启用)
    IXON = 38,

    /// 任意字符恢复输出
    /// Value: 0 (仅 XON) 或 1 (任意字符)
    IXANY = 39,

    /// 启用输入的 XON/XOFF 流控
    /// Value: 0 (禁用) 或 1 (启用)
    IXOFF = 40,

    /// 输入队列满时响铃
    /// Value: 0 (不响铃) 或 1 (响铃)
    IMAXBEL = 41,

    /// 输入为 UTF-8 编码
    /// Value: 0 (非 UTF-8) 或 1 (UTF-8)
    IUTF8 = 42,

    // ========== 本地标志类 (50-62) ==========
    /// 启用信号字符 (VINTR, VQUIT, VSUSP)
    /// Value: 0 (禁用) 或 1 (启用)
    ISIG = 50,

    /// 启用规范模式（行缓冲）
    /// Value: 0 (非规范模式) 或 1 (规范模式)
    ICANON = 51,

    /// 启用大小写转换
    /// Value: 0 (不转换) 或 1 (转换)
    XCASE = 52,

    /// 回显输入字符
    /// Value: 0 (不回显) 或 1 (回显)
    ECHO = 53,

    /// 回显擦除字符
    /// Value: 0 (不回显) 或 1 (回显)
    ECHOE = 54,

    /// 回显 kill 字符
    /// Value: 0 (不回显) 或 1 (回显)
    ECHOK = 55,

    /// ECHO 关闭时也回显换行符
    /// Value: 0 (不回显) 或 1 (回显)
    ECHONL = 56,

    /// 收到信号后不清空输入输出队列
    /// Value: 0 (清空) 或 1 (不清空)
    NOFLSH = 57,

    /// 后台进程写入终端时发送 SIGTTOU
    /// Value: 0 (允许) 或 1 (停止)
    TOSTOP = 58,

    /// 启用扩展输入处理
    /// Value: 0 (禁用) 或 1 (启用)
    IEXTEN = 59,

    /// 将控制字符回显为 ^X 形式
    /// Value: 0 (原样) 或 1 (^X 形式)
    ECHOCTL = 60,

    /// kill 字符回显行擦除
    /// Value: 0 (不擦除) 或 1 (擦除)
    ECHOKE = 61,

    /// 有待重新打印的输入
    /// Value: 0 (无) 或 1 (有)
    PENDIN = 62,

    // ========== 输出标志类 (70-75) ==========
    /// 启用输出后处理
    /// Value: 0 (禁用) 或 1 (启用)
    OPOST = 70,

    /// 将输出的小写转换为大写
    /// Value: 0 (不转换) 或 1 (转换)
    OLCUC = 71,

    /// 将输出的 NL 转换为 CR-NL
    /// Value: 0 (不转换) 或 1 (转换)
    ONLCR = 72,

    /// 将输出的 CR 转换为 NL
    /// Value: 0 (不转换) 或 1 (转换)
    OCRNL = 73,

    /// 在第 0 列不输出 CR
    /// Value: 0 (输出) 或 1 (不输出)
    ONOCR = 74,

    /// NL 同时执行 CR
    /// Value: 0 (不执行) 或 1 (执行)
    ONLRET = 75,

    // ========== 控制标志类 (90-93) ==========
    /// 使用 7 位数据位
    /// Value: 0 (不使用) 或 1 (使用)
    CS7 = 90,

    /// 使用 8 位数据位
    /// Value: 0 (不使用) 或 1 (使用)
    CS8 = 91,

    /// 启用奇偶校验
    /// Value: 0 (禁用) 或 1 (启用)
    PARENB = 92,

    /// 奇校验（否则为偶校验）
    /// Value: 0 (偶校验) 或 1 (奇校验)
    PARODD = 93,

    // ========== 波特率类 (128-129) ==========
    /// 输入波特率
    /// Value: 波特率数值 (0-230400)
    TtyOpISpeed = 128,

    /// 输出波特率
    /// Value: 波特率数值 (0-230400)
    TtyOpOSpeed = 129,
}

impl TtyOpcode {
    /// 从 u8 值创建 TtyOpcode
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

    /// 获取 Opcode 的名称
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

    /// 获取 Opcode 的描述
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

    /// 判断是否是特殊字符类型
    pub fn is_special_char(&self) -> bool {
        (*self as u8) >= 1 && (*self as u8) <= 18
    }

    /// 判断是否是输入标志类型
    pub fn is_input_flag(&self) -> bool {
        (*self as u8) >= 30 && (*self as u8) <= 42
    }

    /// 判断是否是本地标志类型
    pub fn is_local_flag(&self) -> bool {
        (*self as u8) >= 50 && (*self as u8) <= 62
    }

    /// 判断是否是输出标志类型
    pub fn is_output_flag(&self) -> bool {
        (*self as u8) >= 70 && (*self as u8) <= 75
    }

    /// 判断是否是控制标志类型
    pub fn is_control_flag(&self) -> bool {
        (*self as u8) >= 90 && (*self as u8) <= 93
    }

    /// 判断是否是波特率类型
    pub fn is_speed(&self) -> bool {
        (*self as u8) >= 128 && (*self as u8) <= 129
    }

    /// 获取 Value 的类型描述
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

/// 特殊字符的常用值
pub mod special_chars {
    /// Ctrl+C (中断)
    pub const CTRL_C: u32 = 3;
    /// Ctrl+\ (退出)
    pub const CTRL_BACKSLASH: u32 = 28;
    /// Ctrl+D (文件结束)
    pub const CTRL_D: u32 = 4;
    /// Ctrl+U (删除整行)
    pub const CTRL_U: u32 = 21;
    /// Ctrl+Z (挂起)
    pub const CTRL_Z: u32 = 26;
    /// Ctrl+Q (恢复输出)
    pub const CTRL_Q: u32 = 17;
    /// Ctrl+S (停止输出)
    pub const CTRL_S: u32 = 19;
    /// Ctrl+R (重新打印)
    pub const CTRL_R: u32 = 18;
    /// Ctrl+W (删除单词)
    pub const CTRL_W: u32 = 23;
    /// Ctrl+V (字面量下一个)
    pub const CTRL_V: u32 = 22;
    /// Ctrl+Y (延迟挂起)
    pub const CTRL_Y: u32 = 25;
    /// Ctrl+T (状态请求)
    pub const CTRL_T: u32 = 20;
    /// Ctrl+O (丢弃输出)
    pub const CTRL_O: u32 = 15;
    /// Backspace (退格)
    pub const BACKSPACE: u32 = 127;
    /// Ctrl+H (退格备用)
    pub const CTRL_H: u32 = 8;
    /// 禁用特殊字符
    pub const DISABLED: u32 = 255;
}

/// 标准波特率值
pub mod baud_rates {
    pub const B0: u32 = 0;
    pub const B50: u32 = 50;
    pub const B75: u32 = 75;
    pub const B110: u32 = 110;
    pub const B134: u32 = 134;
    pub const B150: u32 = 150;
    pub const B200: u32 = 200;
    pub const B300: u32 = 300;
    pub const B600: u32 = 600;
    pub const B1200: u32 = 1200;
    pub const B1800: u32 = 1800;
    pub const B2400: u32 = 2400;
    pub const B4800: u32 = 4800;
    pub const B9600: u32 = 9600;
    pub const B19200: u32 = 19200;
    pub const B38400: u32 = 38400;
    pub const B57600: u32 = 57600;
    pub const B115200: u32 = 115200;
    pub const B230400: u32 = 230400;
}

/// Terminal Modes 解析器
pub struct TtyModesParser;

impl TtyModesParser {
    /// 解析 terminal modes 数据
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

    /// 编码 terminal modes 数据
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

/// 终端模式预设
pub mod presets {
    use super::*;

    /// 交互式终端模式
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

    /// 密码输入模式
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

    /// 原始模式 (Raw Mode)
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

    /// 串口通信模式
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

#[derive(Clone, Copy, Hash, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct IdentityPair {
    pub client: u32,
    pub server: u32,
}

impl IdentityPair {
    pub(super) fn new(client: u32, server: u32) -> IdentityPair {
        Self { client, server }
    }
}

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

    pub fn identity(&self) -> IdentityPair {
        self.id
    }

    pub async fn eof(&self) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelEof {
            channel_id: self.id,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

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

#[derive(derive_more::Debug)]
pub struct BufferChannel {
    channel: channel::Channel,
    #[debug(skip)]
    write_buf: BytesMut,
    #[debug(skip)]
    read_buf: BytesMut,
}

impl BufferChannel {
    pub fn new(channel: channel::Channel) -> Self {
        Self {
            channel,
            write_buf: Default::default(),
            read_buf: Default::default(),
        }
    }

    pub fn channel_mut(&mut self) -> &mut channel::Channel {
        &mut self.channel
    }

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

    pub fn consumer_read_buffer(&mut self, len: usize) {
        if len == 0 || self.read_buf.is_empty() {
            return;
        }

        assert!(self.read_buf.len() >= len);

        self.read_buf.advance(len);
    }

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

    pub async fn fill(&mut self) -> error::Result<&[u8]> {
        while self.read_buf.is_empty() {
            self.fill_once().await?;
        }
        Ok(&self.read_buf[..])
    }

    pub async fn fill_exact(&mut self, len: usize) -> error::Result<&[u8]> {
        while self.read_buf.len() < len {
            self.fill_once().await?;
        }

        Ok(&self.read_buf[..len])
    }

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
