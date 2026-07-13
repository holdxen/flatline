//! SSH protocol message values shared by the transport and connection layers.
//!
//! This module exposes the [`ChannelOpenFailureReason`], [`DisconnectReason`]
//! and [`Signal`] newtypes together with their protocol constants (RFC 4250,
//! RFC 4253 and RFC 4254). Helpers for parsing binary packets and decoded SSH
//! messages also live here, but they are crate-internal and not part of the
//! public API.

use std::collections::HashMap;

use super::*;
use crate::error;
use crate::ssh::buffer::Consumer;
use protocol::*;
use snafu::ResultExt;

/// A reason code for an `SSH_MSG_CHANNEL_OPEN_FAILURE` message (RFC 4254, section 5.1).
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct ChannelOpenFailureReason(pub u32);

impl ChannelOpenFailureReason {
    /// Opening the channel is administratively prohibited.
    pub const ADMINISTRATIVELY_PROHIBITED: Self = Self(SSH_OPEN_ADMINISTRATIVELY_PROHIBITED);
    /// The connection to the requested destination failed.
    pub const CONNECT_FAILED: Self = Self(SSH_OPEN_CONNECT_FAILED);
    /// The requested channel type is unknown or unsupported.
    pub const UNKNOWN_CHANNEL_TYPE: Self = Self(SSH_OPEN_UNKNOWN_CHANNEL_TYPE);
    /// The peer is short on resources, such as channels or memory.
    pub const RESOURCE_SHORTAGE: Self = Self(SSH_OPEN_RESOURCE_SHORTAGE);
}

/// A reason code for an `SSH_MSG_DISCONNECT` message (RFC 4253, section 11.1).
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct DisconnectReason(pub u32);
impl DisconnectReason {
    /// The host does not allow this client to connect.
    pub const HOST_NOT_ALLOWED_TO_CONNECT: Self = Self(SSH_DISCONNECT_HOST_NOT_ALLOWED_TO_CONNECT);
    /// A protocol error occurred.
    pub const PROTOCOL_ERROR: Self = Self(SSH_DISCONNECT_PROTOCOL_ERROR);
    /// Key exchange failed.
    pub const KEY_EXCHANGE_FAILED: Self = Self(SSH_DISCONNECT_KEY_EXCHANGE_FAILED);
    /// Reserved; not to be used by implementations.
    pub const RESERVED: Self = Self(SSH_DISCONNECT_RESERVED);
    /// A message authentication code (MAC) error occurred.
    pub const MAC_ERROR: Self = Self(SSH_DISCONNECT_MAC_ERROR);
    /// A compression error occurred.
    pub const COMPRESSION_ERROR: Self = Self(SSH_DISCONNECT_COMPRESSION_ERROR);
    /// The requested service is not available.
    pub const SERVICE_NOT_AVAILABLE: Self = Self(SSH_DISCONNECT_SERVICE_NOT_AVAILABLE);
    /// The protocol version is not supported by the remote peer.
    pub const PROTOCOL_VERSION_NOT_SUPPORTED: Self =
        Self(SSH_DISCONNECT_PROTOCOL_VERSION_NOT_SUPPORTED);
    /// The server's host key could not be verified.
    pub const HOST_KEY_NOT_VERIFIABLE: Self = Self(SSH_DISCONNECT_HOST_KEY_NOT_VERIFIABLE);
    /// The connection was lost.
    pub const CONNECTION_LOST: Self = Self(SSH_DISCONNECT_CONNECTION_LOST);
    /// The application at the other end of the connection disconnected.
    pub const BY_APPLICATION: Self = Self(SSH_DISCONNECT_BY_APPLICATION);
    /// Too many connections are already open.
    pub const TOO_MANY_CONNECTIONS: Self = Self(SSH_DISCONNECT_TOO_MANY_CONNECTIONS);
    /// Authentication was cancelled by the user.
    pub const AUTH_CANCELLED_BY_USER: Self = Self(SSH_DISCONNECT_AUTH_CANCELLED_BY_USER);
    /// No more authentication methods are available.
    pub const NO_MORE_AUTH_METHODS_AVAILABLE: Self =
        Self(SSH_DISCONNECT_NO_MORE_AUTH_METHODS_AVAILABLE);
    /// The supplied user name is illegal.
    pub const ILLEGAL_USER_NAME: Self = Self(SSH_DISCONNECT_ILLEGAL_USER_NAME);
}

/// A signal name for the `signal` and `exit-signal` channel requests (RFC 4254, sections 6.9 and 6.10).
///
/// The wrapped string is the bare signal name without the `SIG` prefix.
/// [`Display`](std::fmt::Display) prints the wrapped name, and the
/// `PartialEq<&str>` implementation allows comparing a `Signal` directly with a
/// string slice such as `Signal::ABRT`. The associated constants are plain
/// `&'static str` values rather than `Signal`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Signal(pub String);

/// Writes the wrapped signal name.
impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// Compares the wrapped signal name with a string slice.
impl PartialEq<&str> for Signal {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl Signal {
    /// The abort signal (`SIGABRT`).
    pub const ABRT: &'static str = "ABRT";
    /// The floating-point exception signal (`SIGFPE`).
    pub const FPE: &'static str = "FPE";
    /// The hangup signal (`SIGHUP`).
    pub const HUP: &'static str = "HUP";
    /// The illegal instruction signal (`SIGILL`).
    pub const ILL: &'static str = "ILL";
    /// The interrupt signal (`SIGINT`).
    pub const INT: &'static str = "INT";
    /// The kill signal (`SIGKILL`), which cannot be caught or ignored.
    pub const KILL: &'static str = "KILL";
    /// The broken pipe signal (`SIGPIPE`).
    pub const PIPE: &'static str = "PIPE";
    /// The quit signal (`SIGQUIT`).
    pub const QUIT: &'static str = "QUIT";
    /// The segmentation fault signal (`SIGSEGV`).
    pub const SEGV: &'static str = "SEGV";
    /// The termination signal (`SIGTERM`).
    pub const TERM: &'static str = "TERM";
    /// User-defined signal 1 (`SIGUSR1`).
    pub const USR1: &'static str = "USR1";
    /// User-defined signal 2 (`SIGUSR2`).
    pub const USR2: &'static str = "USR2";
}

#[derive(Debug, Default)]
pub(crate) struct Packet {
    pub payload: Vec<u8>,
    pub padding: Vec<u8>,
}

impl Packet {
    pub fn parse(data: &[u8]) -> error::Result<Self> {
        let mut consumer = Consumer::new(data);

        let padding_len = consumer.consume_u8()?;

        let payload_len = consumer.peek().len() - padding_len as usize;

        if data.len() <= padding_len as usize + 1 {
            return Err(crate::error::builder::InvalidFormat {
                detail: "Unexpected padding length",
            }
            .build());
        }

        let payload = consumer.consume_bytes(payload_len)?.to_vec();

        let padding = consumer.consume_bytes(padding_len as usize)?.to_vec();

        Ok(Packet { payload, padding })
    }
}

#[derive(Clone, derive_more::Debug)]
pub(crate) enum Message<'a> {
    Debug {
        always_display: bool,
        message: &'a str,
        language: &'a str,
    },
    ExtInfo {
        extensions: HashMap<&'a str, &'a [u8]>,
    },
    Ignore {
        #[debug(skip)]
        data: &'a [u8],
    },
    ServiceAccept {
        service: &'a str,
    },
    Disconnect {
        reason: DisconnectReason,
        description: &'a str,
        language: &'a str,
    },
    Unimplemented {
        sequence_number: u32,
    },
    AuthenticationSuccess,
    AuthenticationFailure {
        allow_methods: Vec<&'a str>,
        partial_success: bool,
    },
    AuthenticationBanner {
        message: &'a str,
        language: &'a str,
    },
    ChannelOpenConfirmation {
        recipient_channel: u32,
        sender_channel: u32,
        initial_window_size: u32,
        maximum_packet_size: u32,
    },
    ChannelOpenFailure {
        recipient_channel: u32,
        reason_code: u32,
        description: &'a str,
        language: &'a str,
    },
    ChannelSuccess {
        recipient_channel: u32,
    },
    ChannelFailure {
        recipient_channel: u32,
    },
    ChannelData {
        recipient_channel: u32,
        #[debug(skip)]
        data: &'a [u8],
    },
    ChannelExtendedData {
        recipient_channel: u32,
        data_type: u32,
        #[debug(skip)]
        data: &'a [u8],
    },
    ChannelWindowAdjust {
        recipient_channel: u32,
        count: u32,
    },
    ChannelFlowControl {
        recipient_channel: u32,
        want_reply: bool,
        on: bool,
    },
    ChannelExitStatus {
        recipient_channel: u32,
        want_reply: bool,
        exit_status: u32,
    },
    ChannelExitSignal {
        recipient_channel: u32,
        want_reply: bool,
        signal: &'a str,
        core_dumped: bool,
        error_message: &'a str,
        language: &'a str,
    },
    ChannelEof {
        recipient_channel: u32,
    },
    ChannelClose {
        recipient_channel: u32,
    },
    ChannelOpenForwardedTcpIp {
        sender_channel: u32,
        initial_window_size: u32,
        maximum_packet_size: u32,
        connected_address: &'a str,
        connected_port: u32,
        originator_address: &'a str,
        originator_port: u32,
    },
    ChannelOpenAgentConnect {
        sender_channel: u32,
        initial_window_size: u32,
        maximum_packet_size: u32,
    },
    ChannelOpenX11 {
        sender_channel: u32,
        initial_window_size: u32,
        maximum_packet_size: u32,
        originator_address: &'a str,
        originator_port: u32,
    },
    ChannelOpenForwardedStreamLocal {
        sender_channel: u32,
        initial_window_size: u32,
        maximum_packet_size: u32,
        path: &'a str,
        reserved: &'a str,
    },
    ChannelOpenUnknown {
        sender_channel: u32,
        r#type: &'a str,
    },
    ChannelUnknownRequest {
        recipient_channel: u32,
        r#type: &'a str,
        want_reply: bool,
    },
    GlobalRequestKeepAliveOpenSSH {
        want_reply: bool,
    },
    GlobalRequestHostKeysOpenSSH {
        want_reply: bool,
        host_keys: Vec<&'a [u8]>,
    },
    GlobalUnknownRequest {
        want_reply: bool,
        r#type: &'a str,
    },
    RequestSuccess,
    RequestFailure,
    Ping {
        #[debug(skip)]
        data: &'a [u8],
    },
    Pong {
        #[debug(skip)]
        data: &'a [u8],
    },
    Unrecognized {
        code: u8,
        data: &'a [u8],
    },
}

impl<'a> Message<'a> {
    pub fn parse(data: &'a [u8]) -> error::Result<Self> {
        let mut consumer = Consumer::new(data);
        match consumer.consume_u8()? {
            SSH_MSG_DEBUG => {
                let always_display = consumer.consume_u8()? == 1;
                let message =
                    std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                let language =
                    std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                Ok(Self::Debug {
                    message,
                    language,
                    always_display,
                })
            }
            SSH_MSG_EXT_INFO => {
                let mut extensions = HashMap::new();
                let count = consumer.consume_u32()?;
                for _ in 0..count {
                    let name =
                        std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                    let value = consumer.consume_one()?;
                    extensions.insert(name, value);
                }
                // while !consumer.is_empty() {
                //     let name =
                //         std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                //     let value = consumer.consume_one()?;
                //     extensions.insert(name, value);
                // }
                Ok(Self::ExtInfo { extensions })
            }
            SSH_MSG_SERVICE_ACCEPT => {
                let service =
                    std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                Ok(Self::ServiceAccept { service })
            }
            SSH_MSG_IGNORE => {
                let data = consumer.consume_one()?;
                Ok(Self::Ignore { data })
            }
            SSH_MSG_DISCONNECT => {
                let reason = DisconnectReason(consumer.consume_u32()?);
                let description =
                    std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                let language =
                    std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                Ok(Self::Disconnect {
                    reason,
                    description,
                    language,
                })
            }
            SSH_MSG_UNIMPLEMENTED => {
                let sequence_number = consumer.consume_u32()?;
                Ok(Self::Unimplemented { sequence_number })
            }
            SSH_MSG_USERAUTH_BANNER => {
                let message =
                    std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                let language =
                    std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                Ok(Self::AuthenticationBanner { message, language })
            }
            SSH_MSG_USERAUTH_FAILURE => {
                let allow_methods = std::str::from_utf8(consumer.consume_one()?)
                    .context(ExpectStringSnafu)?
                    .split(',')
                    .collect();
                let partial_success = consumer.consume_u8()? != 0;
                Ok(Self::AuthenticationFailure {
                    allow_methods,
                    partial_success,
                })
            }
            SSH_MSG_USERAUTH_SUCCESS => Ok(Self::AuthenticationSuccess),
            SSH_MSG_CHANNEL_OPEN_CONFIRMATION => {
                let recipient_channel = consumer.consume_u32()?;
                let sender_channel = consumer.consume_u32()?;
                let initial_window_size = consumer.consume_u32()?;
                let maximum_packet_size = consumer.consume_u32()?;
                Ok(Self::ChannelOpenConfirmation {
                    recipient_channel,
                    sender_channel,
                    initial_window_size,
                    maximum_packet_size,
                })
            }
            SSH_MSG_CHANNEL_OPEN_FAILURE => {
                let recipient_channel = consumer.consume_u32()?;
                let reason_code = consumer.consume_u32()?;
                let description =
                    std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                let language =
                    std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                Ok(Self::ChannelOpenFailure {
                    recipient_channel,
                    reason_code,
                    description,
                    language,
                })
            }
            SSH_MSG_CHANNEL_SUCCESS => {
                let recipient_channel = consumer.consume_u32()?;
                Ok(Self::ChannelSuccess { recipient_channel })
            }
            SSH_MSG_CHANNEL_FAILURE => {
                let recipient_channel = consumer.consume_u32()?;
                Ok(Self::ChannelFailure { recipient_channel })
            }
            SSH_MSG_CHANNEL_DATA => {
                let recipient_channel = consumer.consume_u32()?;
                let data = consumer.consume_one()?;
                Ok(Self::ChannelData {
                    recipient_channel,
                    data,
                })
            }
            SSH_MSG_CHANNEL_EXTENDED_DATA => {
                let recipient_channel = consumer.consume_u32()?;
                let data_type = consumer.consume_u32()?;
                let data = consumer.consume_one()?;
                Ok(Self::ChannelExtendedData {
                    recipient_channel,
                    data_type,
                    data,
                })
            }
            SSH_MSG_CHANNEL_WINDOW_ADJUST => {
                let recipient_channel = consumer.consume_u32()?;
                let count = consumer.consume_u32()?;
                Ok(Self::ChannelWindowAdjust {
                    recipient_channel,
                    count,
                })
            }
            SSH_MSG_CHANNEL_REQUEST => {
                let recipient_channel = consumer.consume_u32()?;
                let r#type = consumer.consume_one()?;
                if r#type == b"xon-xoff" {
                    let want_reply = consumer.consume_u8()? != 0;
                    let on = consumer.consume_u8()? != 0;
                    Ok(Message::ChannelFlowControl {
                        recipient_channel,
                        want_reply,
                        on,
                    })
                } else if r#type == b"exit-status" {
                    let want_reply = consumer.consume_u8()? != 0;
                    let exit_status = consumer.consume_u32()?;
                    Ok(Message::ChannelExitStatus {
                        recipient_channel,
                        want_reply,
                        exit_status,
                    })
                } else if r#type == b"exit-signal" {
                    let want_reply = consumer.consume_u8()? != 0;
                    let signal = consumer.consume_one()?;
                    let signal = std::str::from_utf8(signal).context(ExpectStringSnafu)?;
                    let core_dumped = consumer.consume_u8()? != 0;
                    let error_message =
                        std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                    let language =
                        std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;

                    Ok(Self::ChannelExitSignal {
                        recipient_channel,
                        want_reply,
                        signal,
                        core_dumped,
                        error_message,
                        language,
                    })
                } else {
                    let want_reply = consumer.consume_u8()? != 0;
                    let r#type = std::str::from_utf8(r#type).context(ExpectStringSnafu)?;
                    Ok(Self::ChannelUnknownRequest {
                        recipient_channel,
                        want_reply,
                        r#type,
                    })
                }
            }
            SSH_MSG_GLOBAL_REQUEST => {
                let r#type =
                    std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;
                let want_reply = consumer.consume_u8()? != 0;

                match r#type {
                    openssh::SSH_GLOBAL_REQUEST_TYPE_KEEP_ALIVE => {
                        Ok(Self::GlobalRequestKeepAliveOpenSSH { want_reply })
                    }
                    openssh::SSH_GLOBAL_REQUEST_TYPE_HOST_KEYS => {
                        let mut host_keys = Vec::with_capacity(16);
                        while !consumer.peek().is_empty() {
                            host_keys.push(consumer.consume_one()?);
                        }
                        Ok(Self::GlobalRequestHostKeysOpenSSH {
                            want_reply,
                            host_keys,
                        })
                    }
                    _ => Ok(Self::GlobalUnknownRequest { want_reply, r#type }),
                }
            }
            SSH_MSG_CHANNEL_OPEN => {
                let r#type =
                    std::str::from_utf8(consumer.consume_one()?).context(ExpectStringSnafu)?;

                match r#type {
                    SSH_CHANNEL_TYPE_FORWARDED_TCP_IP => {
                        let sender_channel = consumer.consume_u32()?;
                        let initial_window_size = consumer.consume_u32()?;
                        let maximum_packet_size = consumer.consume_u32()?;
                        let connected_address = consumer.consume_one()?;
                        let connected_address =
                            std::str::from_utf8(connected_address).context(ExpectStringSnafu)?;
                        let connected_port = consumer.consume_u32()?;
                        let originator_address = consumer.consume_one()?;
                        let originator_address =
                            std::str::from_utf8(originator_address).context(ExpectStringSnafu)?;
                        let originator_port = consumer.consume_u32()?;

                        Ok(Self::ChannelOpenForwardedTcpIp {
                            sender_channel,
                            initial_window_size,
                            maximum_packet_size,
                            connected_address,
                            connected_port,
                            originator_address,
                            originator_port,
                        })
                    }
                    SSH_CHANNEL_TYPE_AGENT_CONNECT | openssh::SSH_CHANNEL_TYPE_AGENT_CONNECT => {
                        let sender_channel = consumer.consume_u32()?;
                        let initial_window_size = consumer.consume_u32()?;
                        let maximum_packet_size = consumer.consume_u32()?;

                        Ok(Self::ChannelOpenAgentConnect {
                            sender_channel,
                            initial_window_size,
                            maximum_packet_size,
                        })
                    }
                    SSH_CHANNEL_TYPE_X11 => {
                        let sender_channel = consumer.consume_u32()?;
                        let initial_window_size = consumer.consume_u32()?;
                        let maximum_packet_size = consumer.consume_u32()?;

                        let originator_address = std::str::from_utf8(consumer.consume_one()?)
                            .context(ExpectStringSnafu)?;
                        let originator_port = consumer.consume_u32()?;
                        Ok(Self::ChannelOpenX11 {
                            sender_channel,
                            initial_window_size,
                            maximum_packet_size,
                            originator_address,
                            originator_port,
                        })
                    }
                    openssh::SSH_CHANNEL_TYPE_FORWARDED_STREAM_LOCAL => {
                        let sender_channel = consumer.consume_u32()?;
                        let initial_window_size = consumer.consume_u32()?;
                        let maximum_packet_size = consumer.consume_u32()?;
                        let path = std::str::from_utf8(consumer.consume_one()?)
                            .context(ExpectStringSnafu)?;
                        let reserved = std::str::from_utf8(consumer.consume_one()?)
                            .context(ExpectStringSnafu)?;
                        Ok(Self::ChannelOpenForwardedStreamLocal {
                            sender_channel,
                            initial_window_size,
                            maximum_packet_size,
                            path,
                            reserved,
                        })
                    }
                    _ => {
                        let sender_channel = consumer.consume_u32()?;
                        Ok(Self::ChannelOpenUnknown {
                            sender_channel,
                            r#type,
                        })
                    }
                }
            }
            openssh::SSH_MSG_PING => {
                let data = consumer.consume_one()?;
                Ok(Self::Ping { data })
            }
            openssh::SSH_MSG_PONG => {
                let data = consumer.consume_one()?;
                Ok(Self::Pong { data })
            }
            SSH_MSG_CHANNEL_CLOSE => {
                let recipient_channel = consumer.consume_u32()?;
                Ok(Self::ChannelClose { recipient_channel })
            }
            SSH_MSG_CHANNEL_EOF => {
                let recipient_channel = consumer.consume_u32()?;
                Ok(Self::ChannelEof { recipient_channel })
            }
            SSH_MSG_REQUEST_SUCCESS => Ok(Self::RequestSuccess),
            SSH_MSG_REQUEST_FAILURE => Ok(Self::RequestFailure),
            code => Ok(Message::Unrecognized { code, data }),
        }
    }
}
