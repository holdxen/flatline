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

        // Validate `padding_len` before subtracting it: it is read straight
        // off the wire, so checking first keeps a malformed packet from
        // underflowing the subtraction below.
        if data.len() <= padding_len as usize + 1 {
            return Err(crate::error::builder::InvalidFormat {
                detail: "Unexpected padding length",
            }
            .build());
        }

        // The check above guarantees `peek().len() > padding_len`, so this
        // cannot underflow and the payload is at least one byte long.
        let payload_len = consumer.peek().len() - padding_len as usize;

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

#[cfg(test)]
mod test {
    use super::*;

    /// Builds a message payload: one message-code byte followed by `body`.
    fn payload(code: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![code];
        out.extend_from_slice(body);
        out
    }

    /// Encodes an SSH `string`: a 4-byte big-endian length followed by bytes.
    fn ssh_string(bytes: &[u8]) -> Vec<u8> {
        let mut out = (bytes.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(bytes);
        out
    }

    #[test]
    fn parse_disconnect() {
        let mut body = Vec::new();
        body.extend_from_slice(&11u32.to_be_bytes()); // SSH_DISCONNECT_BY_APPLICATION
        body.extend_from_slice(&ssh_string(b"bye"));
        body.extend_from_slice(&ssh_string(b"en"));

        let data = payload(SSH_MSG_DISCONNECT, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::Disconnect {
            reason,
            description,
            language,
        } = msg
        else {
            panic!("expected Disconnect, got {msg:?}");
        };
        assert_eq!(reason, DisconnectReason(11));
        assert_eq!(description, "bye");
        assert_eq!(language, "en");
    }

    #[test]
    fn parse_ignore_carries_data() {
        let mut body = Vec::new();
        body.extend_from_slice(&ssh_string(b"junk"));

        let data = payload(SSH_MSG_IGNORE, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::Ignore { data } = msg else {
            panic!("expected Ignore, got {msg:?}");
        };
        assert_eq!(data, b"junk");
    }

    #[test]
    fn parse_service_accept() {
        let mut body = Vec::new();
        body.extend_from_slice(&ssh_string(b"ssh-userauth"));

        let data = payload(SSH_MSG_SERVICE_ACCEPT, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::ServiceAccept { service } = msg else {
            panic!("expected ServiceAccept, got {msg:?}");
        };
        assert_eq!(service, "ssh-userauth");
    }

    #[test]
    fn parse_userauth_success_and_failure() {
        let data = payload(SSH_MSG_USERAUTH_SUCCESS, &[]);
        let msg = Message::parse(&data).unwrap();
        assert!(matches!(msg, Message::AuthenticationSuccess));

        let mut body = Vec::new();
        body.extend_from_slice(&ssh_string(b"publickey,password"));
        body.push(1); // partial success

        let data = payload(SSH_MSG_USERAUTH_FAILURE, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::AuthenticationFailure {
            allow_methods,
            partial_success,
        } = msg
        else {
            panic!("expected AuthenticationFailure, got {msg:?}");
        };
        assert_eq!(allow_methods, vec!["publickey", "password"]);
        assert!(partial_success);
    }

    #[test]
    fn parse_userauth_banner() {
        let mut body = Vec::new();
        body.extend_from_slice(&ssh_string(b"hello"));
        body.extend_from_slice(&ssh_string(b""));

        let data = payload(SSH_MSG_USERAUTH_BANNER, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::AuthenticationBanner { message, language } = msg else {
            panic!("expected AuthenticationBanner, got {msg:?}");
        };
        assert_eq!(message, "hello");
        assert_eq!(language, "");
    }

    #[test]
    fn parse_channel_open_confirmation() {
        let mut body = Vec::new();
        for value in [0u32, 7, 1024, 32768] {
            body.extend_from_slice(&value.to_be_bytes());
        }

        let data = payload(SSH_MSG_CHANNEL_OPEN_CONFIRMATION, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::ChannelOpenConfirmation {
            recipient_channel,
            sender_channel,
            initial_window_size,
            maximum_packet_size,
        } = msg
        else {
            panic!("expected ChannelOpenConfirmation, got {msg:?}");
        };
        assert_eq!((recipient_channel, sender_channel), (0, 7));
        assert_eq!((initial_window_size, maximum_packet_size), (1024, 32768));
    }

    #[test]
    fn parse_channel_data_and_extended_data() {
        let mut body = Vec::new();
        body.extend_from_slice(&3u32.to_be_bytes());
        body.extend_from_slice(&4u32.to_be_bytes());
        body.extend_from_slice(b"data");

        let data = payload(SSH_MSG_CHANNEL_DATA, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::ChannelData {
            recipient_channel,
            data,
        } = msg
        else {
            panic!("expected ChannelData, got {msg:?}");
        };
        assert_eq!(recipient_channel, 3);
        assert_eq!(data, b"data");

        let mut body = Vec::new();
        body.extend_from_slice(&3u32.to_be_bytes());
        body.extend_from_slice(&1u32.to_be_bytes()); // SSH_EXTENDED_DATA_STDERR
        body.extend_from_slice(&3u32.to_be_bytes());
        body.extend_from_slice(b"err");

        let data = payload(SSH_MSG_CHANNEL_EXTENDED_DATA, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::ChannelExtendedData {
            recipient_channel,
            data_type,
            data,
        } = msg
        else {
            panic!("expected ChannelExtendedData, got {msg:?}");
        };
        assert_eq!(recipient_channel, 3);
        assert_eq!(data_type, 1);
        assert_eq!(data, b"err");
    }

    #[test]
    fn parse_channel_window_adjust() {
        let mut body = Vec::new();
        body.extend_from_slice(&5u32.to_be_bytes());
        body.extend_from_slice(&65536u32.to_be_bytes());

        let data = payload(SSH_MSG_CHANNEL_WINDOW_ADJUST, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::ChannelWindowAdjust {
            recipient_channel,
            count,
        } = msg
        else {
            panic!("expected ChannelWindowAdjust, got {msg:?}");
        };
        assert_eq!((recipient_channel, count), (5, 65536));
    }

    /// Builds an `SSH_MSG_CHANNEL_REQUEST` body for `type_name`.
    fn channel_request_body(type_name: &str, want_reply: bool, tail: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&ssh_string(type_name.as_bytes()));
        body.push(want_reply as u8);
        body.extend_from_slice(tail);
        body
    }

    #[test]
    fn parse_channel_request_exit_status() {
        let mut tail = Vec::new();
        tail.extend_from_slice(&42u32.to_be_bytes());

        let data = payload(
            SSH_MSG_CHANNEL_REQUEST,
            &channel_request_body("exit-status", true, &tail),
        );
        let msg = Message::parse(&data).unwrap();
        let Message::ChannelExitStatus {
            recipient_channel,
            want_reply,
            exit_status,
        } = msg
        else {
            panic!("expected ChannelExitStatus, got {msg:?}");
        };
        assert_eq!(recipient_channel, 1);
        assert!(want_reply);
        assert_eq!(exit_status, 42);
    }

    #[test]
    fn parse_channel_request_xon_xoff() {
        let data = payload(
            SSH_MSG_CHANNEL_REQUEST,
            &channel_request_body("xon-xoff", false, &[1]),
        );
        let msg = Message::parse(&data).unwrap();
        let Message::ChannelFlowControl {
            recipient_channel,
            want_reply,
            on,
        } = msg
        else {
            panic!("expected ChannelFlowControl, got {msg:?}");
        };
        assert_eq!(recipient_channel, 1);
        assert!(!want_reply);
        assert!(on);
    }

    #[test]
    fn parse_channel_request_exit_signal() {
        let mut tail = Vec::new();
        tail.extend_from_slice(&ssh_string(b"TERM"));
        tail.push(0); // core_dumped
        tail.extend_from_slice(&ssh_string(b"killed"));
        tail.extend_from_slice(&ssh_string(b"en"));

        let data = payload(
            SSH_MSG_CHANNEL_REQUEST,
            &channel_request_body("exit-signal", true, &tail),
        );
        let msg = Message::parse(&data).unwrap();
        let Message::ChannelExitSignal {
            recipient_channel,
            want_reply,
            signal,
            core_dumped,
            error_message,
            language,
        } = msg
        else {
            panic!("expected ChannelExitSignal, got {msg:?}");
        };
        assert_eq!(recipient_channel, 1);
        assert!(want_reply);
        assert_eq!(signal, "TERM");
        assert!(!core_dumped);
        assert_eq!(error_message, "killed");
        assert_eq!(language, "en");
    }

    #[test]
    fn parse_channel_request_unknown_type() {
        let data = payload(
            SSH_MSG_CHANNEL_REQUEST,
            &channel_request_body("shell", true, &[]),
        );
        let msg = Message::parse(&data).unwrap();
        let Message::ChannelUnknownRequest {
            recipient_channel,
            want_reply,
            r#type,
        } = msg
        else {
            panic!("expected ChannelUnknownRequest, got {msg:?}");
        };
        assert_eq!(recipient_channel, 1);
        assert!(want_reply);
        assert_eq!(r#type, "shell");
    }

    #[test]
    fn parse_channel_open_unknown_type() {
        let mut body = Vec::new();
        body.extend_from_slice(&ssh_string(b"bogus!"));
        body.extend_from_slice(&9u32.to_be_bytes()); // sender_channel
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&2u32.to_be_bytes());

        let data = payload(SSH_MSG_CHANNEL_OPEN, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::ChannelOpenUnknown {
            sender_channel,
            r#type,
        } = msg
        else {
            panic!("expected ChannelOpenUnknown, got {msg:?}");
        };
        assert_eq!(sender_channel, 9);
        assert_eq!(r#type, "bogus!");
    }

    #[test]
    fn parse_channel_close_and_eof() {
        let mut body = Vec::new();
        body.extend_from_slice(&4u32.to_be_bytes());

        let data = payload(SSH_MSG_CHANNEL_CLOSE, &body);
        let msg = Message::parse(&data).unwrap();
        assert!(matches!(
            msg,
            Message::ChannelClose {
                recipient_channel: 4
            }
        ));

        let data = payload(SSH_MSG_CHANNEL_EOF, &body);
        let msg = Message::parse(&data).unwrap();
        assert!(matches!(
            msg,
            Message::ChannelEof {
                recipient_channel: 4
            }
        ));
    }

    #[test]
    fn parse_channel_success_and_failure() {
        let mut body = Vec::new();
        body.extend_from_slice(&8u32.to_be_bytes());

        let data = payload(SSH_MSG_CHANNEL_SUCCESS, &body);
        let msg = Message::parse(&data).unwrap();
        assert!(matches!(
            msg,
            Message::ChannelSuccess {
                recipient_channel: 8
            }
        ));

        let data = payload(SSH_MSG_CHANNEL_FAILURE, &body);
        let msg = Message::parse(&data).unwrap();
        assert!(matches!(
            msg,
            Message::ChannelFailure {
                recipient_channel: 8
            }
        ));
    }

    #[test]
    fn parse_request_success_and_failure_have_no_body() {
        let data = payload(SSH_MSG_REQUEST_SUCCESS, &[]);
        let msg = Message::parse(&data).unwrap();
        assert!(matches!(msg, Message::RequestSuccess));

        let data = payload(SSH_MSG_REQUEST_FAILURE, &[]);
        let msg = Message::parse(&data).unwrap();
        assert!(matches!(msg, Message::RequestFailure));
    }

    #[test]
    fn parse_global_request_keepalive_and_unknown() {
        let mut body = Vec::new();
        body.extend_from_slice(&ssh_string(
            openssh::SSH_GLOBAL_REQUEST_TYPE_KEEP_ALIVE.as_bytes(),
        ));
        body.push(1);

        let data = payload(SSH_MSG_GLOBAL_REQUEST, &body);
        let msg = Message::parse(&data).unwrap();
        assert!(matches!(
            msg,
            Message::GlobalRequestKeepAliveOpenSSH { want_reply: true }
        ));

        let mut body = Vec::new();
        let rtype = "tcpip-forward";
        body.extend_from_slice(&ssh_string(rtype.as_bytes()));
        body.push(0);

        let data = payload(SSH_MSG_GLOBAL_REQUEST, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::GlobalUnknownRequest { want_reply, r#type } = msg else {
            panic!("expected GlobalUnknownRequest, got {msg:?}");
        };
        assert!(!want_reply);
        assert_eq!(r#type, "tcpip-forward");
    }

    #[test]
    fn parse_global_request_host_keys_collects_all_keys() {
        let mut body = Vec::new();
        body.extend_from_slice(&ssh_string(
            openssh::SSH_GLOBAL_REQUEST_TYPE_HOST_KEYS.as_bytes(),
        ));
        body.push(1);
        for key in [b"key-one".as_slice(), b"key-two"] {
            body.extend_from_slice(&(key.len() as u32).to_be_bytes());
            body.extend_from_slice(key);
        }

        let data = payload(SSH_MSG_GLOBAL_REQUEST, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::GlobalRequestHostKeysOpenSSH {
            want_reply,
            host_keys,
        } = msg
        else {
            panic!("expected GlobalRequestHostKeysOpenSSH, got {msg:?}");
        };
        assert!(want_reply);
        assert_eq!(host_keys, vec![b"key-one".as_slice(), b"key-two"]);
    }

    #[test]
    fn parse_ext_info_collects_extensions() {
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&ssh_string(b"server-sig-algs"));
        body.extend_from_slice(&ssh_string(b"ssh-ed25519"));

        let data = payload(SSH_MSG_EXT_INFO, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::ExtInfo { extensions } = msg else {
            panic!("expected ExtInfo, got {msg:?}");
        };
        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions["server-sig-algs"], b"ssh-ed25519");
    }

    #[test]
    fn parse_debug_and_unimplemented() {
        let mut body = Vec::new();
        body.push(0); // always_display = false
        body.extend_from_slice(&ssh_string(b"db"));
        body.extend_from_slice(&ssh_string(b""));

        let data = payload(SSH_MSG_DEBUG, &body);
        let msg = Message::parse(&data).unwrap();
        let Message::Debug {
            always_display,
            message,
            language,
        } = msg
        else {
            panic!("expected Debug, got {msg:?}");
        };
        assert!(!always_display);
        assert_eq!(message, "db");
        assert_eq!(language, "");

        let mut body = Vec::new();
        body.extend_from_slice(&42u32.to_be_bytes());
        let data = payload(SSH_MSG_UNIMPLEMENTED, &body);
        let msg = Message::parse(&data).unwrap();
        assert!(matches!(
            msg,
            Message::Unimplemented {
                sequence_number: 42
            }
        ));
    }

    #[test]
    fn parse_ping_and_pong() {
        let mut body = Vec::new();
        body.extend_from_slice(&ssh_string(b"ping"));

        let data = payload(openssh::SSH_MSG_PING, &body);
        let msg = Message::parse(&data).unwrap();
        assert!(matches!(msg, Message::Ping { data } if data == b"ping"));

        let data = payload(openssh::SSH_MSG_PONG, &body);
        let msg = Message::parse(&data).unwrap();
        assert!(matches!(msg, Message::Pong { data } if data == b"ping"));
    }

    #[test]
    fn parse_unrecognized_code_falls_back() {
        let data = payload(200, b"whatever");
        let msg = Message::parse(&data).unwrap();
        let Message::Unrecognized { code, data: rest } = msg else {
            panic!("expected Unrecognized, got {msg:?}");
        };
        assert_eq!(code, 200);
        assert_eq!(rest, &data[..]);
    }

    #[test]
    fn parse_empty_buffer_fails() {
        assert!(Message::parse(&[]).is_err());
    }

    #[test]
    fn parse_non_utf8_string_fails() {
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&[0xff, 0xfe]);

        assert!(Message::parse(&payload(SSH_MSG_SERVICE_ACCEPT, &body)).is_err());
    }

    #[test]
    fn packet_parse_splits_payload_and_padding() {
        // layout: padding_len, payload, padding
        let mut raw = vec![4u8];
        raw.extend_from_slice(b"abcd");
        raw.extend_from_slice(&[9, 9, 9, 9]);

        let packet = Packet::parse(&raw).unwrap();
        assert_eq!(packet.payload, b"abcd");
        assert_eq!(packet.padding, [9, 9, 9, 9]);
    }

    #[test]
    fn packet_parse_rejects_padding_longer_than_body() {
        // padding_len (200) exceeds the remaining bytes: the subtraction
        // that derives payload_len must not run before the bounds check.
        let raw = [200u8, 1, 2, 3];
        let err = Packet::parse(&raw).expect_err("padding longer than the body");
        assert!(
            err.to_string().contains("Unexpected padding length"),
            "unexpected error: {err}"
        );
    }

    /// The regression test for the underflow: `padding_len` is read straight
    /// off the wire, so every value larger than the body must produce an
    /// error rather than a wrapping subtraction.
    #[test]
    fn packet_parse_never_underflows_on_padding_len() {
        for padding_len in [4u8, 5, 8, 64, 128, 200, 254, 255] {
            // Body shorter than the claimed padding.
            let mut raw = vec![padding_len];
            raw.extend_from_slice(&[0u8; 3]);
            assert!(
                Packet::parse(&raw).is_err(),
                "padding_len {padding_len} with a 3-byte body must be rejected"
            );

            // Body exactly padding_len + 1 long: no room for any payload.
            let mut raw = vec![padding_len];
            raw.extend(std::iter::repeat_n(0u8, padding_len as usize));
            assert!(
                Packet::parse(&raw).is_err(),
                "padding_len {padding_len} filling the whole body must be rejected"
            );
        }
    }

    #[test]
    fn packet_parse_accepts_padding_len_one_below_the_body_end() {
        // Body = 1 (payload) + padding: just enough room, so this parses.
        let mut raw = vec![3u8];
        raw.push(b'x'); // one byte of payload
        raw.extend_from_slice(&[7, 7, 7]); // three bytes of padding

        let packet = Packet::parse(&raw).unwrap();
        assert_eq!(packet.payload, b"x");
        assert_eq!(packet.padding, [7, 7, 7]);
    }

    #[test]
    fn packet_parse_rejects_an_empty_body() {
        // Only the padding-length byte, nothing behind it.
        assert!(Packet::parse(&[0u8]).is_err());
        // No padding length at all.
        assert!(Packet::parse(&[]).is_err());
    }

    #[test]
    fn packet_parse_allows_zero_padding() {
        // padding_len = 0 is accepted by this parser: everything behind the
        // length byte is payload. (RFC 4253 requires at least 4 bytes of
        // padding on the wire, but that is enforced by the packet layer
        // rather than here.)
        let packet = Packet::parse(&[0u8, 1, 2]).unwrap();
        assert_eq!(packet.payload, [1, 2]);
        assert!(packet.padding.is_empty());
    }

    #[test]
    fn packet_parse_rejects_truncated_payload() {
        // padding_len equals the whole body => payload would be empty/invalid
        let raw = [3u8, 1, 2, 3];
        assert!(Packet::parse(&raw).is_err());
    }

    /// A well-formed packet with the maximum padding length a `u8` allows
    /// must still parse when the body is big enough.
    #[test]
    fn packet_parse_handles_max_padding_len() {
        let mut raw = vec![255u8];
        raw.push(b'p'); // one byte of payload
        raw.extend(std::iter::repeat_n(9u8, 255)); // 255 bytes of padding

        let packet = Packet::parse(&raw).unwrap();
        assert_eq!(packet.payload, b"p");
        assert_eq!(packet.padding.len(), 255);
    }
}
