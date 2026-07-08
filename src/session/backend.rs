use snafu::{OptionExt, ResultExt};
use std::collections::HashMap;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

use crate::DEFAULT_CHANNEL_CAPACITY;
use crate::error::builder;
use crate::session::channel::{IdentityPair, TtyOpcode};
use crate::session::{
    KeyboardInteractive, Notifier, RequestFailureSnafu, UnexpectedBehaviourSnafu, forward,
};
use crate::ssh::buffer::Consumer;
use crate::ssh::msg::{Message, Signal};
use crate::{
    error,
    session::channel,
    session::event::Event,
    ssh::{
        buffer::Producer,
        msg,
        protocol::*,
        stream::{CipherStream, Stream},
    },
};

struct ListenerHandle {
    sender: mpsc::Sender<(forward::Stream, forward::SocketAddr)>,
    initial_window_size: u32,
    maximum_packet_size: u32,
}

impl ListenerHandle {
    fn new(
        sender: mpsc::Sender<(forward::Stream, forward::SocketAddr)>,
        initial_window_size: u32,
        maximum_packet_size: u32,
    ) -> Self {
        Self {
            sender,
            initial_window_size,
            maximum_packet_size,
        }
    }
}

struct ChannelHandle {
    client: ChannelEndpoint,
    server: ChannelEndpoint,
    sender: mpsc::Sender<channel::Message>,
}

#[derive(Clone, Copy, Default, Debug)]
struct ChannelEndpoint {
    id: u32,
    initial_window_size: u32,
    used_window_size: i64,
    maximum_packet_size: u32,
    closed: bool,
    eof: bool,
}

impl ChannelEndpoint {
    fn left(&self) -> u64 {
        assert!(
            self.used_window_size < 0
                || self.initial_window_size as u64 >= self.used_window_size as u64
        );

        (i64::from(self.initial_window_size) - self.used_window_size)
            .try_into()
            .expect("Unreachable")

        // (self.initial_window_size as u64 - self.used_window_size)
    }
}

#[easy_ext::ext]
impl<T> CipherStream<T>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    fn send_channel_window_adjust(
        &mut self,
        server_id: u32,
        count: u32,
    ) -> impl Future<Output = error::Result<()>> {
        async move {
            let buffer = make_buffer_without_header!(
                u8: SSH_MSG_CHANNEL_WINDOW_ADJUST,
                u32: server_id,
                u32: count
            );
            self.send_payload(&buffer[..]).await?;
            Ok(())
        }
    }

    fn send_channel_close(&mut self, server_id: u32) -> impl Future<Output = error::Result<()>> {
        async move {
            let buffer = make_buffer_without_header!(
                u8: SSH_MSG_CHANNEL_CLOSE,
                u32: server_id
            );
            self.send_payload(&buffer[..]).await?;
            Ok(())
        }
    }
}

#[easy_ext::ext]
impl<T> oneshot::Sender<T> {
    #[track_caller]
    fn send_tracing(self, v: T) -> Option<T> {
        if let Err(v) = self.send(v) {
            tracing::warn!("Failed to send result back");
            Some(v)
        } else {
            None
        }
    }
}

pub struct SessionInner<T, N>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    session_id: Vec<u8>,
    socket: CipherStream<T>,
    receiver: mpsc::Receiver<Event>,
    config: super::Config,
    client_version: String,
    server_version: String,
    frontend: mpsc::WeakSender<Event>,
    compat_options: super::CompatOptions,
    notifier: N,
    channels: Vec<ChannelHandle>,
    server_algorithms: Vec<String>,
    server_ping_supported: bool,
    forward: HashMap<forward::SocketAddr, ListenerHandle>,
    renegotiating: bool,
}

impl<T, N> SessionInner<T, N>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    pub fn session_id(&self) -> &[u8] {
        &self.session_id
    }

    pub fn config(&self) -> &super::Config {
        &self.config
    }

    pub fn notifier_mut(&mut self) -> &mut N {
        &mut self.notifier
    }

    pub fn socket_mut(&mut self) -> &mut CipherStream<T> {
        &mut self.socket
    }

    pub fn server_version(&self) -> &str {
        &self.server_version
    }

    pub fn client_version(&self) -> &str {
        &self.client_version
    }

    pub fn compat_options(&self) -> &super::CompatOptions {
        &self.compat_options
    }

    pub(super) fn new(
        session_id: Vec<u8>,
        socket: CipherStream<T>,
        notifier: N,
        client_version: String,
        server_version: String,
        compat_options: super::CompatOptions,
        config: super::Config,
        receiver: mpsc::Receiver<Event>,
        frontend: mpsc::WeakSender<Event>,
    ) -> Self {
        Self {
            session_id,
            socket,
            receiver,
            channels: Default::default(),
            config,
            server_algorithms: Default::default(),
            server_ping_supported: false,
            forward: Default::default(),
            frontend,
            notifier,
            client_version,
            server_version,
            compat_options,
            renegotiating: false,
        }
    }
}
impl<T, N> SessionInner<T, N>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
    N: Notifier + Send,
{
    pub async fn handle_msg(&mut self, msg: Message<'_>) -> error::Result<()> {
        tracing::info!("Handling message: {:?}", msg);
        let mut handled = true;
        match msg {
            Message::Debug {
                always_display,
                message,
                language,
            } => {
                if !language.is_empty() {
                    tracing::debug!("Field lanuage should be empty");
                }
                if always_display {
                    tracing::warn!("Debug message from server: {}", message);
                } else {
                    tracing::info!("Debug message from server: {}", message);
                }
            }
            Message::ExtInfo { extensions } => {
                if let Some(methods) = extensions.get("server-sig-algs") {
                    let method = match std::str::from_utf8(methods) {
                        Ok(method) => method,
                        Err(_) => {
                            tracing::warn!("Invalid server-sig-algs value from server");
                            return Ok(());
                        }
                    };

                    self.server_algorithms = method.split(',').map(|s| s.to_string()).collect();
                }
                if let Some(v) = extensions.get("ping@openssh.com") {
                    if v == b"0" {
                        self.server_ping_supported = true;
                    }
                }
                tracing::info!("Extensions: {:?}", extensions);
            }
            Message::Ignore { data } => {
                tracing::info!("Ignore message: {}", data.len());
            }
            Message::ServiceAccept { .. } => {
                handled = false;
            }
            Message::Disconnect {
                reason,
                description,
                language,
            } => {
                tracing::info!(
                    "Server disconnect: reason: {:?}, description: {}, language: {}",
                    reason,
                    description,
                    language
                );
                self.notifier.disconnected(reason, description).await;
                return Err(super::DisconnectedSnafu {
                    reason,
                    description,
                }
                .build()
                .into());
            }
            Message::Unimplemented { sequence_number } => {
                tracing::warn!("Unimplemented: {}", sequence_number);
            }
            Message::AuthenticationBanner { message, language } => {
                tracing::info!(
                    "UserAuthenticationBanner: message={}, language={}",
                    message,
                    language
                );
            }
            Message::UnrecognizedMessage {
                code: SSH_MSG_KEXINIT,
                data,
            } if !self.renegotiating => {
                self.renegotiating = true;
                self.renegotiate(Some(data.to_vec())).await?;
                self.renegotiating = false;
            }
            Message::UnrecognizedMessage { code, .. } => {
                tracing::warn!("Unrecognized message: {}", code);
                handled = false;
            }
            Message::AuthenticationFailure { .. } => {
                handled = false;
            }
            Message::AuthenticationSuccess => {
                handled = false;
            }
            Message::ChannelOpenConfirmation { .. } => {
                handled = false;
            }
            Message::ChannelOpenFailure { .. } => {
                handled = false;
            }
            Message::ChannelSuccess { .. } => {
                handled = false;
            }
            Message::ChannelFailure { .. } => {
                handled = false;
            }
            Message::ChannelData {
                recipient_channel,
                data,
            } => {
                self.handle_channel_data(recipient_channel, data, false)
                    .await?;
            }
            Message::Ping { data } => {
                // check is supported openssh ping
                self.send_pong(data).await?;
            }
            Message::Pong { data } => {
                // check data is eq and warning
                tracing::debug!("Pong data: len={}", data.len());
            }
            Message::ChannelExtendedData {
                recipient_channel,
                data_type,
                data,
            } => {
                if data_type == SSH_EXTENDED_DATA_STDERR {
                    self.handle_channel_data(recipient_channel, data, true)
                        .await?;
                } else {
                    handled = false;
                }
            }
            Message::ChannelWindowAdjust {
                recipient_channel,
                count,
            } => {
                self.handle_channel_window_adjust(recipient_channel, count)
                    .await?;
            }
            Message::ChannelFlowControl {
                recipient_channel,
                want_reply,
                on,
            } => {
                self.handle_channel_flow_control(recipient_channel, want_reply, on)
                    .await?;
            }
            Message::ChannelExitStatus {
                recipient_channel,
                want_reply,
                exit_status,
            } => {
                self.handle_channel_exit_status(recipient_channel, want_reply, exit_status)
                    .await?;
            }
            Message::ChannelExitSignal {
                recipient_channel,
                want_reply,
                signal,
                core_dumped,
                error_message,
                language,
            } => {
                self.handle_channel_exit_signal(
                    recipient_channel,
                    want_reply,
                    signal,
                    core_dumped,
                    error_message,
                    language,
                )
                .await?;
            }
            Message::ChannelClose { recipient_channel } => {
                self.handle_channel_close(recipient_channel).await?;
            }
            Message::ChannelEof { recipient_channel } => {
                self.handle_channel_eof(recipient_channel).await?;
            }
            Message::ChannelUnknownRequest {
                recipient_channel,
                r#type,
                want_reply,
            } => {
                self.handle_channel_unknown_request(recipient_channel, want_reply, r#type)
                    .await?;
            }
            Message::GlobalRequestKeepAliveOpenSSH { want_reply } => {
                self.handle_global_request_keep_alive_openssh(want_reply)
                    .await?;
            }
            Message::GlobalRequestHostKeysOpenSSH {
                want_reply,
                host_keys,
            } => {
                self.handle_global_request_hosts_key_openssh(want_reply, &host_keys)
                    .await?;
            }
            Message::GlobalUnknownRequest { want_reply, r#type } => {
                self.handle_global_unknown_request(want_reply, r#type)
                    .await?;
            }
            Message::ChannelOpenForwardedTcpIp {
                sender_channel,
                initial_window_size,
                maximum_packet_size,
                connected_address,
                connected_port,
                originator_address,
                originator_port,
            } => {
                self.handle_forwarded_tcp_ip(
                    sender_channel,
                    initial_window_size,
                    maximum_packet_size,
                    connected_address,
                    connected_port,
                    originator_address,
                    originator_port,
                )
                .await?;
            }
            Message::ChannelOpenAgentConnect { .. } => {}
            Message::ChannelOpenUnknown { .. } => {}
            Message::RequestSuccess => {}
            Message::RequestFailure => {}
            Message::ChannelOpenX11 {
                sender_channel,
                initial_window_size,
                maximum_packet_size,
                originator_address,
                originator_port,
            } => {
                self.handle_channel_open_x11(
                    sender_channel,
                    initial_window_size,
                    maximum_packet_size,
                    originator_address,
                    originator_port,
                )
                .await?;
            }
        }
        if !handled {
            self.send_unimplemented().await?;
        }
        Ok(())
    }

    async fn handle_global_request_hosts_key_openssh(
        &mut self,
        want_reply: bool,
        host_keys: &[&[u8]],
    ) -> error::Result<()> {
        let accept = self.notifier.server_host_keys(host_keys).await;

        if want_reply {
            if accept {
                self.socket.send_payload(&[SSH_MSG_REQUEST_SUCCESS]).await?;
            } else {
                self.socket.send_payload(&[SSH_MSG_REQUEST_FAILURE]).await?;
            }
        }

        Ok(())
    }

    async fn disconnect(&mut self, reason: u32, description: &str) -> error::Result<()> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_DISCONNECT,
            u32: reason,
            one: description,
            u32: 0
        };
        self.socket.send_payload(&buffer[..]).await
    }

    #[async_recursion::async_recursion]
    async fn renegotiate(&mut self, msg: Option<Vec<u8>>) -> error::Result<()> {
        let mut exchange = super::handshake::RekeyExchange::new(self);
        exchange.exec(msg).await?;
        Ok(())
    }

    async fn handle_channel_open_x11(
        &mut self,
        sender_channel: u32,
        initial_window_size: u32,
        maximum_packet_size: u32,
        originator_address: &str,
        originator_port: u32,
    ) -> error::Result<()> {
        let mut default_initial_window_size = super::Session::DEFAULT_INITIAL_WINDOW_SIZE;
        let mut default_maximum_packet_size = super::Session::DEFAULT_MAXIMUM_PACKET_SIZE;

        let (forward_sender, forward_receiver) = oneshot::channel();

        let verify = async || {
            let port = u16::try_from(originator_port)
                .ok()
                .context(super::InvalidPortSnafu)?;

            let addr = forward::SocketAddr::new(originator_address.to_string(), port);
            let accept = self
                .notifier
                .x11_forward(
                    addr,
                    forward_receiver,
                    &mut default_initial_window_size,
                    &mut default_maximum_packet_size,
                )
                .await;

            if !accept {
                return Err(super::RequestFailureSnafu.build().into());
            }

            error::ok(())
        };

        if let Err(e) = verify().await {
            let buffer = make_buffer_without_header! {
                u8: SSH_MSG_CHANNEL_OPEN_FAILURE,
                u32: sender_channel,
                u32: msg::ChannelOpenFailureReason::CONNECT_FAILED.0,
                one: e.to_string(),
                one: "" // language
            };

            self.socket.send_payload(&buffer[..]).await?;
        }

        // let originator_port = match u16::try_from(originator_port) {
        //     Ok(v) => v,
        //     Err(e) => {
        //         tracing::info!("Invalid forward message from server");

        //         let buffer = make_buffer_without_header! {
        //             u8: SSH_MSG_CHANNEL_OPEN_FAILURE,
        //             u32: sender_channel,
        //             u32: msg::ChannelOpenFailureReason::CONNECT_FAILED.0,
        //             one: e.to_string(),
        //             one: "" // language
        //         };

        //         self.socket.send_payload(&buffer[..]).await?;

        //         return Ok(());
        //     }
        // };

        let id = IdentityPair::new(self.next_channel_id(), sender_channel);

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_OPEN_CONFIRMATION,
            u32: id.server,
            u32: id.client,
            u32: default_initial_window_size,
            u32: default_maximum_packet_size,
        };

        self.socket.send_payload(&buffer[..]).await?;

        let (sender, receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);

        let handle = ChannelHandle {
            client: ChannelEndpoint {
                id: id.client,
                initial_window_size: default_initial_window_size,
                used_window_size: 0,
                maximum_packet_size: default_maximum_packet_size,
                closed: false,
                eof: false,
            },
            server: ChannelEndpoint {
                id: id.server,
                initial_window_size,
                used_window_size: 0,
                maximum_packet_size,
                closed: false,
                eof: false,
            },
            sender,
        };

        self.channels.push(handle);

        let channel = channel::Channel::new(id, receiver, self.upgrade_frontend()?);

        if let Err(_) = forward_sender.send(forward::Stream::new(channel)) {
            tracing::info!("Failed to send forward stream, try to close stream now");
            self.channel_close(id).await?;
        }
        Ok(())
    }

    async fn authenticate_keyboard_interactive(
        &mut self,
        username: &str,
        mut interactive: Box<dyn KeyboardInteractive>,
        methods: &[super::InteractiveMethod],
    ) -> error::Result<super::AuthenticateResult> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_USERAUTH_REQUEST,
            one: username,
            one: "ssh-connection",
            one: "keyboard-interactive",
            one: "",
            one: methods.iter().map(|v| v.as_ref()).collect::<Vec<&str>>().join(",")
        };

        self.socket.send_payload(&buffer[..]).await?;

        loop {
            let packet = self.socket.recv_packet().await?;
            let msg = Message::parse(&packet.payload)?;
            match msg {
                Message::AuthenticationSuccess => {
                    self.socket.authenticated = true;
                    break Ok(super::AuthenticateResult::Success);
                }
                Message::AuthenticationFailure {
                    partial_success,
                    allow_methods,
                } => {
                    break Ok(super::AuthenticateResult::Failure {
                        partial_success,
                        allow_methods: allow_methods.into_iter().map(|m| m.to_string()).collect(),
                    });
                }
                Message::UnrecognizedMessage {
                    code: SSH_MSG_USERAUTH_INFO_REQUEST,
                    data,
                } => {
                    // let mut response = async || {
                    let mut consumer = Consumer::new(&data[1..]);
                    let name = consumer.consume_one()?;
                    let name = std::str::from_utf8(name).context(msg::ExpectStringSnafu)?;

                    let instruction = consumer.consume_one()?;
                    let instruction =
                        std::str::from_utf8(instruction).context(msg::ExpectStringSnafu)?;

                    let _lang = consumer.consume_one()?;
                    let size = consumer.consume_u32()?;

                    let mut prompts = Vec::with_capacity(size as usize);

                    for _ in 0..size {
                        let content = consumer.consume_one()?;
                        let content =
                            std::str::from_utf8(content).context(msg::ExpectStringSnafu)?;
                        let echo = consumer.consume_u8()? != 0;
                        prompts.push(super::Prompt { content, echo })
                    }

                    let responses = interactive.interactive(name, instruction, &prompts).await?;

                    snafu::ensure!(
                        responses.len() == prompts.len(),
                        builder::InvalidOperation {
                            detail: "Invalid response"
                        }
                    );

                    let mut buffer = make_buffer_without_header! {
                        u8: SSH_MSG_USERAUTH_INFO_RESPONSE,
                        u32: size,
                    };

                    for i in responses {
                        buffer.put_one(i)
                    }

                    self.socket.send_payload(&buffer[..]).await?;
                }
                _ => {
                    self.handle_msg(msg).await?;
                }
            }
        }
    }

    // async fn channel_upgrade_sftp(&mut self, id: IdentityPair) -> error::Result<()> {
    //     self.channel_request_subsystem(id, true, "sftp").await?;
    //
    //
    //     let buffer = make_buffer! {
    //         u8: sftp::SSH_FXP_INIT,
    //         u32: sftp::VERSION,
    //     };
    //
    //     self.socket.send_payload(&buffer[..]).await?;
    //
    //     let mut tmp = vec![];
    //
    //     loop {
    //         let packet = self.socket.recv_packet().await?;
    //         let msg = Message::parse(&packet.payload)?;
    //         match msg {
    //             Message::ChannelData { recipient_channel, data } if recipient_channel == id.client => {
    //                 tmp.extend_from_slice(data);
    //
    //                 let mut consumer = Consumer::new(data);
    //
    //                 let len = consumer.consume_u32()? as usize;
    //                 if len >= consumer.peek().len() {
    //                     let mut consumer = Consumer::new(consumer.consume_bytes(len)?);
    //                     if consumer.consume_u8()? != sftp::SSH_FXP_VERSION {
    //                         return Err(super::UnexpectedMessageSnafu {
    //                             detail: "Expected SSH_FXP_VERSION message"
    //                         }.build().into());
    //                     }
    //                     let version = consumer.consume_u32()?;
    //                     if version != sftp::VERSION {
    //                         tracing::warn!("SFTP version mismatch: {}", version);
    //                     }
    //
    //                     let mut extensions = HashMap::new();
    //                     while !consumer.is_empty() {
    //                         let k = consumer.consume_one()?;
    //                         let k = std::str::from_utf8(k).context(msg::ExpectStringSnafu)?;
    //
    //                         let v = consumer.consume_one()?;
    //                         extensions.insert(k.to_string(), v.to_vec());
    //                     }
    //
    //                 }
    //
    //                 if len > consumer.peek().len() {
    //                     self.handle_channel_data(id.client, &consumer.peek()[len..], false).await?;
    //                 }
    //
    //                 break Ok(());
    //             }
    //             Message::ChannelExtendedData { recipient_channel, data_type, .. } if recipient_channel == id.client => {
    //                 tracing::warn!("SFTP channel data mismatch: {}", data_type);
    //             }
    //             Message::ChannelEof { recipient_channel } if recipient_channel == id.client => {
    //                 return Err(super::UnexpectedChannelEofSnafu.build().into());
    //             }
    //             Message::ChannelClose { recipient_channel } if recipient_channel == id.client => {
    //                 return Err(super::UnexpectedChannelClosedSnafu.build().into());
    //             }
    //             msg => {
    //                 self.handle_msg(msg).await?;
    //             }
    //         }
    //     }
    // }

    pub async fn channel_clean(&mut self) -> error::Result<()> {
        let len = self.channels.len();
        for i in (0..len).rev() {
            let channel = &mut self.channels[i];
            if channel.sender.is_closed() {
                if !channel.client.closed {
                    tracing::info!(
                        "Clean closed channel client={}, server={}",
                        channel.client.id,
                        channel.server.id
                    );
                    self.socket.send_channel_close(channel.server.id).await?;
                    channel.client.closed = true;
                }
                if channel.server.closed {
                    self.channels.remove(i);
                }
            }
        }

        Ok(())
    }

    pub async fn channel_open_sftp(
        &mut self,
        initial_window_size: u32,
        maximum_packet_size: u32,
    ) -> error::Result<channel::Channel> {
        tracing::info!("Waiting -1");

        let channel = self
            .channel_open_session(initial_window_size, maximum_packet_size)
            .await?;

        let id = channel.identity();

        tracing::info!("Waiting");

        self.channel_request_subsystem(id, true, "sftp").await?;

        tracing::info!("Waiting 2");

        Ok(channel)
    }

    pub async fn channel_request_subsystem(
        &mut self,
        id: IdentityPair,
        want_reply: bool,
        subsystem: &str,
    ) -> error::Result<()> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: id.server,
            one: "subsystem",
            u8: want_reply.into(),
            one: subsystem,
        };

        self.socket.send_payload(&buffer[..]).await?;

        self.wait_for_channel_response(want_reply, id).await
    }

    fn upgrade_frontend(&self) -> error::Result<mpsc::Sender<Event>> {
        let sender = self.frontend.upgrade().context(UnexpectedBehaviourSnafu {
            detail: "Maybe session should be shutdown",
        })?;
        Ok(sender)
    }

    async fn global_request_tcp_ip_forward(
        &mut self,
        mut addr: forward::SocketAddr,
        initial_window_size: u32,
        maximum_packet_size: u32,
    ) -> error::Result<forward::Listener> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_GLOBAL_REQUEST,
            one: SSH_GLOBAL_REQUEST_TYPE_TCP_IP_FORWARD,
            u8: 1,
            one: &addr.host,
            u32: addr.port as u32
        };

        self.socket.send_payload(&buffer[..]).await?;

        loop {
            let packet = self.socket.recv_packet().await?;
            if packet.payload[0] == SSH_MSG_REQUEST_SUCCESS {
                let mut consumer = Consumer::new(&packet.payload[1..]);
                let port = consumer.consume_u32()?;
                let port = u16::try_from(port).ok().context(super::InvalidPortSnafu)?;

                let frontend = self.upgrade_frontend()?;

                let (sender, receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);

                addr.port = port;

                let listener = forward::Listener::new(receiver, frontend, addr.clone());

                self.forward.insert(
                    addr,
                    ListenerHandle::new(sender, initial_window_size, maximum_packet_size),
                );

                return Ok(listener);
            } else if packet.payload[0] == SSH_MSG_REQUEST_FAILURE {
                return Err(super::RequestFailureSnafu.build().into());
            } else {
                let msg = Message::parse(&packet.payload)?;
                self.handle_msg(msg).await?;
            }
        }
    }

    async fn handle_global_request_keep_alive_openssh(
        &mut self,
        want_reply: bool,
    ) -> error::Result<()> {
        if want_reply {
            self.socket.send_payload(&[SSH_MSG_REQUEST_SUCCESS]).await?;
        }

        Ok(())
    }

    // async fn handle_global_request_host_keys_openssh(
    //     &mut self,
    //     want_reply: bool,
    //     host_keys: &[&[u8]],
    // ) -> error::Result<()> {
    //     todo!()
    // }

    async fn handle_global_unknown_request(
        &mut self,
        want_reply: bool,
        r#type: &str,
    ) -> error::Result<()> {
        tracing::info!("Global unknown request: {}", r#type);

        if want_reply {
            self.socket.send_payload(&[SSH_MSG_REQUEST_FAILURE]).await?;
        }

        Ok(())
    }

    async fn handle_channel_unknown_request(
        &mut self,
        recipient_channel: u32,
        want_reply: bool,
        r#type: &str,
    ) -> error::Result<()> {
        tracing::warn!("Unknown channel request: {}", r#type);

        let channel = self
            .channels
            .iter()
            .find(|v| v.client.id == recipient_channel);

        let Some(channel) = channel else {
            tracing::warn!("Received unexpected channel: {:?}", recipient_channel);
            return Ok(());
        };

        if want_reply {
            let buffer = make_buffer_without_header! {
                u8: SSH_MSG_CHANNEL_FAILURE,
                u32: channel.server.id
            };

            self.socket.send_payload(&buffer[..]).await?;
        }

        Ok(())
    }

    async fn handle_channel_exit_signal(
        &mut self,
        recipient_channel: u32,
        want_reply: bool,
        signal: &str,
        core_dumped: bool,
        error_message: &str,
        language: &str,
    ) -> error::Result<()> {
        if want_reply {
            tracing::warn!("want_reply should be false");
        }
        if !language.is_empty() {
            tracing::warn!("language should be empty: {}", language);
        }

        let channel = self
            .channels
            .iter()
            .find(|v| v.client.id == recipient_channel);

        let Some(channel) = channel else {
            tracing::warn!("Received unexpected channel: {:?}", recipient_channel);
            return Ok(());
        };

        let status = channel::ExitStatus::Interrupt {
            signal: Signal(signal.to_string()),
            core_dumped,
            error_message: error_message.to_string(),
        };

        if let Err(_) = channel.sender.send(channel::Message::Exit(status)).await {
            tracing::error!("Failed to send exit status");
        }

        Ok(())
    }

    async fn handle_channel_exit_status(
        &mut self,
        recipient_channel: u32,
        want_reply: bool,
        exit_status: u32,
    ) -> error::Result<()> {
        if want_reply {
            tracing::warn!("want_reply should be false");
        }

        let channel = self
            .channels
            .iter()
            .find(|v| v.client.id == recipient_channel);

        let Some(channel) = channel else {
            tracing::warn!("Received unexpected channel: {:?}", recipient_channel);
            return Ok(());
        };

        if let Err(_) = channel
            .sender
            .send(channel::Message::Exit(channel::ExitStatus::Normal(
                exit_status,
            )))
            .await
        {
            tracing::error!("Failed to send exit status");
        }
        Ok(())
    }

    async fn channel_request_window_change(
        &mut self,
        id: IdentityPair,
        columns: u32,
        rows: u32,
        width: u32,
        height: u32,
    ) -> error::Result<()> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: id.server,
            one: "window-change",
            u8: 0,
            u32: columns,
            u32: rows,
            u32: width,
            u32: height
        };

        self.socket.send_payload(&buffer[..]).await
    }

    async fn handle_channel_flow_control(
        &mut self,
        recipient_channel: u32,
        want_reply: bool,
        on: bool,
    ) -> error::Result<()> {
        let channel = self
            .channels
            .iter()
            .find(|v| v.client.id == recipient_channel);
        let Some(channel) = channel else {
            tracing::error!("Channel not found");
            return Ok(());
        };

        if want_reply {
            tracing::warn!("want_reply should be false");
        }

        if let Err(_) = channel
            .sender
            .send(channel::Message::FlowControl { on })
            .await
        {
            tracing::error!("Failed to send message");
        }

        Ok(())
    }

    async fn handle_channel_window_adjust(
        &mut self,
        client_id: u32,
        count: u32,
    ) -> error::Result<()> {
        let Some(channel) = self.channels.iter_mut().find(|c| c.client.id == client_id) else {
            tracing::error!("Channel not found");
            return Ok(());
        };

        channel.server.used_window_size -= count as i64;

        if let Err(e) = channel
            .sender
            .send(channel::Message::WindowChange { size: count })
            .await
        {
            tracing::error!("Failed to send window change: {}", e);
        }

        Ok(())
    }

    async fn channel_send(&mut self, id: IdentityPair, data: &[u8]) -> error::Result<usize> {
        let Some(channel) = self.channels.iter_mut().find(|c| c.server.id == id.server) else {
            return Err(builder::InvalidOperation {
                detail: format!("Channel({}) not found", id.server),
            }
            .build()
            .into());
        };

        let mut sent = 0;

        while sent != data.len() && channel.server.left() != 0 {
            let size = (data.len() - sent)
                .min(MAX_PACKET_PAYLOAD_LENGTH - 4 - 1)
                .min(channel.server.left() as usize);
            let buffer = make_buffer_without_header!(
                u8: SSH_MSG_CHANNEL_DATA,
                u32: id.server,
                one: &data[sent..sent+size],
            );
            self.socket.send_payload(&buffer[..]).await?;

            channel.server.used_window_size += size as i64;
            sent += size;
        }

        // let size = data.len().min(channel.server.left() as usize);
        //
        // let buffer = make_buffer_without_header!(
        //     u8: SSH_MSG_CHANNEL_DATA,
        //     u32: id.server,
        //     one: &data[..size]
        // );
        //
        // self.socket.send_payload(&buffer[..]).await?;
        //
        // channel.server.used_window_size += size as i64;

        Ok(sent)
    }

    async fn handle_channel_data(
        &mut self,
        client_id: u32,
        data: &[u8],
        stderr: bool,
    ) -> error::Result<()> {
        let Some(channel) = self.channels.iter_mut().find(|c| c.client.id == client_id) else {
            tracing::warn!("Channel({}) not found", client_id);
            return Ok(());
        };

        if data.len() > channel.client.maximum_packet_size as usize {
            tracing::warn!(
                "Data too long, ignore: maximum={}, actual={}",
                channel.client.maximum_packet_size,
                data.len()
            );
            return Ok(());
        }

        let min = 1024;

        let left = channel.client.left();

        if data.len() > left as usize {
            tracing::warn!(
                "Data too long, ignore: left={}, actual={}",
                left,
                data.len()
            );
            return Ok(());
        }

        let msg = if stderr {
            channel::Message::Stderr(data.to_vec())
        } else {
            channel::Message::Stdout(data.to_vec())
        };

        if let Err(_) = channel.sender.send(msg).await {
            tracing::warn!("Unable to send data to channel({})", client_id);
            return Ok(());
        }

        channel.client.used_window_size += data.len() as i64;

        if channel.client.left() < min {
            let size = channel
                .client
                .used_window_size
                .unsigned_abs()
                .try_into()
                .expect("Unreachable");
            self.socket
                .send_channel_window_adjust(channel.server.id, size)
                .await?;
            channel.client.used_window_size -= i64::from(size);
        }

        Ok(())
    }

    // async fn send_channel_window_adjust(
    //     &mut self,
    //     server_id: u32,
    //     count: u32,
    // ) -> error::Result<()> {
    //     self.socket
    //         .send_channel_window_adjust(server_id, count)
    //         .await
    // }

    async fn send_pong(&mut self, data: &[u8]) -> error::Result<()> {
        let buffer = make_buffer_without_header!(
            u8: openssh::SSH_MSG_PONG,
            one: data
        );
        self.socket.send_payload(&buffer[..]).await?;
        Ok(())
    }

    async fn handle_event(&mut self, event: Event) -> error::Result<()> {
        match event {
            Event::AuthenticateNone { username, back } => {
                let result = self.authenticate_none(&username).await;
                back.send_tracing(result);
            }
            Event::AuthenticatePassword {
                username,
                password,
                back,
            } => {
                let result = self.authenticate_password(&username, &password).await;
                back.send_tracing(result);
            }
            Event::AuthenticatePublicKey {
                username,
                method,
                is_certificate,
                public_blob,
                private_blob,
                back,
            } => {
                let result = self
                    .authenticate_public_key(
                        &username,
                        &method,
                        is_certificate,
                        &public_blob,
                        &private_blob,
                    )
                    .await;
                back.send_tracing(result);
            }
            Event::RequestAuthentication { back } => {
                let result = self.request_authentication().await;
                back.send_tracing(result);
            }
            Event::SendDebugMessage {
                always_display,
                message,
                back,
            } => {
                let result = self.send_debug_message(always_display, &message).await;
                back.send_tracing(result);
            }
            Event::ChannelRequestPty {
                channel_id,
                terminal,
                want_reply,
                columns,
                rows,
                width,
                height,
                modes,
                back,
            } => {
                let result = self
                    .channel_request_pty(
                        channel_id, want_reply, &terminal, columns, rows, width, height, modes,
                    )
                    .await;

                back.send_tracing(result);
            }
            Event::ChannelOpenSession {
                initial_window_size,
                maximum_packet_size,
                back,
            } => {
                let result = self
                    .channel_open_session(initial_window_size, maximum_packet_size)
                    .await;

                back.send_tracing(result);
            }
            Event::ChannelOpenDirectTcpIp {
                target,
                source,
                initial_window_size,
                maximum_packet_size,
                back,
            } => {
                let result = self
                    .channel_open_direct_tcp_ip(
                        initial_window_size,
                        maximum_packet_size,
                        target,
                        source,
                    )
                    .await;
                back.send_tracing(result);
            }
            Event::ChannelRequestX11 {
                channel_id,
                want_reply,
                single_connection,
                protocol,
                cookie,
                screen,
                back,
            } => {
                let result = self
                    .channel_request_x11(
                        channel_id,
                        want_reply,
                        single_connection,
                        &protocol,
                        &cookie,
                        screen,
                    )
                    .await;
                back.send_tracing(result);
            }
            Event::ChannelSendData {
                channel_id,
                data,
                back,
            } => {
                let result = self.channel_send(channel_id, &data).await;
                back.send_tracing(result);
            }
            Event::ChannelRequestBreak {
                channel_id,
                want_reply,
                milliseconds,
                back,
            } => {
                let result = self
                    .channel_request_break(channel_id, want_reply, milliseconds)
                    .await;
                back.send_tracing(result);
            }
            Event::ChannelRequestShell {
                channel_id,
                want_reply,
                back,
            } => {
                let result = self.channel_request_shell(channel_id, want_reply).await;
                back.send_tracing(result);
            }
            Event::ChannelRequestAgent {
                channel_id,
                want_reply,
                back,
            } => {
                let result = self.channel_request_agent(channel_id, want_reply).await;
                back.send_tracing(result);
            }
            Event::ChannelRequestExec {
                channel_id,
                want_reply,
                command,
                back,
            } => {
                let result = self
                    .channel_request_exec(channel_id, want_reply, &command)
                    .await;
                back.send_tracing(result);
            }
            Event::ChannelRequestWindowChange {
                channel_id,
                columns,
                rows,
                width,
                height,
                back,
            } => {
                let result = self
                    .channel_request_window_change(channel_id, columns, rows, width, height)
                    .await;
                back.send_tracing(result);
            }
            Event::GlobalRequestTcpIPForward {
                addr,
                initial_window_size,
                maximum_packet_size,
                back,
            } => {
                let result = self
                    .global_request_tcp_ip_forward(addr, initial_window_size, maximum_packet_size)
                    .await;
                back.send_tracing(result);
            }
            Event::ChannelClose { channel_id, back } => {
                let result = self.channel_close(channel_id).await;
                back.send_tracing(result);
            }
            Event::ChannelEof { channel_id, back } => {
                let result = self.channel_eof(channel_id).await;
                back.send_tracing(result);
            }
            Event::GlobalRequestCancelTcpIpForward {
                want_reply,
                addr,
                back,
            } => {
                let result = self
                    .global_request_cancel_tcp_ip_forward(want_reply, &addr)
                    .await;
                back.send_tracing(result);
            }
            Event::AuthenticateKeyboardInteractive {
                username,
                interactive,
                methods,
                back,
            } => {
                let result = self
                    .authenticate_keyboard_interactive(&username, interactive, &methods)
                    .await;
                back.send_tracing(result);
            }
            Event::ChannelOpenSFTP {
                initial_window_size,
                maximum_packet_size,
                back,
            } => {
                let result = self
                    .channel_open_sftp(initial_window_size, maximum_packet_size)
                    .await;
                back.send_tracing(result);
            }
            Event::ChannelClean { back } => {
                let result = self.channel_clean().await;
                back.send_tracing(result);
            }
            Event::ChannelRequestSignal {
                channel_id,
                want_reply,
                signal,
                back,
            } => {
                let result = self
                    .channel_request_signal(channel_id, want_reply, &signal)
                    .await;
                back.send_tracing(result);
            }
            Event::ChannelRequestEnv {
                channel_id,
                want_reply,
                name,
                value,
                back,
            } => {
                let result = self
                    .channel_request_env(channel_id, want_reply, &name, &value)
                    .await;
                back.send_tracing(result);
            }
            Event::Renegotiate { back } => {
                let result = self.renegotiate(None).await;
                back.send_tracing(result);
            }
            Event::Disconnect {
                reason,
                description,
                back,
            } => {
                let result = self.disconnect(reason, &description).await;
                back.send_tracing(result);
            }
        }
        Ok(())
    }

    async fn handle_forwarded_tcp_ip(
        &mut self,
        sender_channel: u32,
        initial_window_size: u32,
        maximum_packet_size: u32,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        originator_port: u32,
    ) -> error::Result<()> {
        let verify = || {
            let connected_port = u16::try_from(connected_port)
                .ok()
                .context(super::InvalidPortSnafu)?;
            let originator_port = u16::try_from(originator_port)
                .ok()
                .context(super::InvalidPortSnafu)?;
            let Some(listener) = self.forward.get(&forward::SocketAddr::new(
                connected_address.to_string(),
                connected_port,
            )) else {
                return Err(super::UnexpectedMessageSnafu {
                    detail: "No such forward listener",
                }
                .build()
                .into());
            };

            error::ok((listener, originator_port, connected_port))
        };

        let (listener, originator_port, _) = match verify() {
            Ok(v) => v,
            Err(e) => {
                tracing::info!("Invalid forward message from server");

                let buffer = make_buffer_without_header! {
                    u8: SSH_MSG_CHANNEL_OPEN_FAILURE,
                    u32: sender_channel,
                    u32: msg::ChannelOpenFailureReason::CONNECT_FAILED.0,
                    one: e.to_string(),
                    one: "" // language
                };

                self.socket.send_payload(&buffer[..]).await?;

                return Ok(());
            }
        };

        let id = IdentityPair::new(self.next_channel_id(), sender_channel);

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_OPEN_CONFIRMATION,
            u32: id.server,
            u32: id.client,
            u32: listener.initial_window_size,
            u32: listener.maximum_packet_size,
        };

        self.socket.send_payload(&buffer[..]).await?;

        let (sender, receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);

        let handle = ChannelHandle {
            client: ChannelEndpoint {
                id: id.client,
                initial_window_size: listener.initial_window_size,
                used_window_size: 0,
                maximum_packet_size: listener.maximum_packet_size,
                closed: false,
                eof: false,
            },
            server: ChannelEndpoint {
                id: id.server,
                initial_window_size,
                used_window_size: 0,
                maximum_packet_size,
                closed: false,
                eof: false,
            },
            sender,
        };

        self.channels.push(handle);

        let channel = channel::Channel::new(id, receiver, self.upgrade_frontend()?);

        if let Err(_) = listener
            .sender
            .send((
                forward::Stream::new(channel),
                forward::SocketAddr::new(originator_address.to_string(), originator_port),
            ))
            .await
        {
            tracing::info!("Failed to send forward stream, try to close stream now");
            self.channel_close(id).await?;
        }

        Ok(())
    }

    async fn global_request_cancel_tcp_ip_forward(
        &mut self,
        want_reply: bool,
        addr: &forward::SocketAddr,
    ) -> error::Result<()> {
        if self.forward.remove(addr).is_none() {
            tracing::warn!("Maybe forward listener not exists")
        }

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_GLOBAL_REQUEST,
            one: SSH_GLOBAL_REQUEST_TYPE_TCP_IP_FORWARD,
            one: &addr.host,
            u32: addr.port as u32
        };

        self.socket.send_payload(&buffer[..]).await?;

        if want_reply {
            self.match_msg(|msg| {
                Some(match msg {
                    Message::RequestSuccess => Ok(()),
                    Message::RequestFailure => Err(RequestFailureSnafu.build().into()),
                    _ => return None,
                })
            })
            .await?;
        }
        Ok(())
    }

    async fn channel_eof(&mut self, id: IdentityPair) -> error::Result<()> {
        let index = self
            .channels
            .iter_mut()
            .position(|v| v.client.id == id.client);
        let Some(index) = index else {
            tracing::error!("Channel not found");
            return Err(UnexpectedBehaviourSnafu {
                detail: "No such channel",
            }
            .build()
            .into());
        };

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_EOF,
            u32: id.server
        };

        self.socket.send_payload(&buffer[..]).await?;

        let channel = &mut self.channels[index];

        channel.client.eof = true;

        Ok(())
    }
    async fn channel_close(&mut self, id: IdentityPair) -> error::Result<()> {
        let index = self
            .channels
            .iter_mut()
            .position(|v| v.client.id == id.client);
        let Some(index) = index else {
            tracing::error!("Channel not found");
            return Err(UnexpectedBehaviourSnafu {
                detail: "No such channel",
            }
            .build()
            .into());
        };

        // let buffer = make_buffer_without_header! {
        //     u8: SSH_MSG_CHANNEL_CLOSE,
        //     u32: id.server
        // };

        // self.socket.send_payload(&buffer[..]).await?;

        self.socket.send_channel_close(id.server).await?;

        let channel = &mut self.channels[index];

        channel.client.closed = true;
        if channel.server.closed {
            self.channels.remove(index);
        }

        Ok(())
    }

    async fn handle_channel_eof(&mut self, recipient_channel: u32) -> error::Result<()> {
        let channel = self
            .channels
            .iter_mut()
            .find(|v| v.client.id == recipient_channel);
        let Some(channel) = channel else {
            tracing::error!("Channel not found");
            return Ok(());
        };

        if let Err(_) = channel.sender.send(channel::Message::Eof).await {
            tracing::error!("Failed to send message");
        }

        channel.server.eof = true;

        Ok(())
    }
    async fn handle_channel_close(&mut self, recipient_channel: u32) -> error::Result<()> {
        let index = self
            .channels
            .iter_mut()
            .position(|v| v.client.id == recipient_channel);
        let Some(index) = index else {
            tracing::error!("Channel not found");
            return Ok(());
        };

        let channel = &mut self.channels[index];

        if let Err(_) = channel.sender.send(channel::Message::Close).await {
            tracing::error!("Failed to send message");
        }

        channel.server.closed = true;
        if channel.client.closed {
            self.channels.remove(index);
        }

        Ok(())
    }

    async fn channel_request_agent(
        &mut self,
        channel_id: IdentityPair,
        want_reply: bool,
    ) -> error::Result<()> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: channel_id.server,
            one: "agent-req",
            u8: want_reply.into(),
        };

        self.socket.send_payload(&buffer[..]).await?;

        self.wait_for_channel_response(want_reply, channel_id).await
    }

    async fn channel_request_signal(
        &mut self,
        channel_id: IdentityPair,
        want_reply: bool,
        signal: &str,
    ) -> error::Result<()> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: channel_id.server,
            one: "signal",
            u8: want_reply.into(),
            one: signal,
        };
        self.socket.send_payload(&buffer[..]).await?;

        self.wait_for_channel_response(want_reply, channel_id).await
    }

    async fn channel_request_exec(
        &mut self,
        channel_id: channel::IdentityPair,
        want_reply: bool,
        command: &str,
    ) -> error::Result<()> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: channel_id.server,
            one: "exec",
            u8: want_reply.into(),
            one: command,
        };
        self.socket.send_payload(&buffer[..]).await?;

        self.wait_for_channel_response(want_reply, channel_id).await
    }

    async fn channel_request_x11(
        &mut self,
        channel_id: channel::IdentityPair,
        want_reply: bool,
        single_connection: bool,
        protocol: &str,
        cookie: &str,
        screen: u32,
    ) -> error::Result<()> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: channel_id.server,
            one: "x11-req",
            u8: want_reply.into(),
            u8: single_connection.into(),
            one: protocol,
            one: cookie,
            u32: screen,
        };
        self.socket.send_payload(&buffer[..]).await?;

        self.wait_for_channel_response(want_reply, channel_id).await
    }

    async fn channel_request_env(
        &mut self,
        channel_id: channel::IdentityPair,
        want_reply: bool,
        name: &str,
        value: &str,
    ) -> error::Result<()> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: channel_id.server,
            one: "env",
            u8: want_reply.into(),
            one: name,
            one: value,
        };

        self.socket.send_payload(&buffer[..]).await?;

        self.wait_for_channel_response(want_reply, channel_id).await
    }

    async fn channel_request_shell(
        &mut self,
        channel_id: channel::IdentityPair,
        want_reply: bool,
    ) -> error::Result<()> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: channel_id.server,
            one: "shell".as_bytes(),
            u8: want_reply.into(),
        };

        self.socket.send_payload(&buffer[..]).await?;

        self.wait_for_channel_response(want_reply, channel_id).await
    }

    async fn send_unimplemented(&mut self) -> error::Result<()> {
        let sequence_number = self.socket.server().sequence_number;
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_UNIMPLEMENTED,
            u32: sequence_number,
        };
        self.socket.send_payload(&buffer[..]).await?;
        Ok(())
    }

    async fn request_authentication(&mut self) -> error::Result<()> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_SERVICE_REQUEST,
            one: SSH_SERVICE_NAME_USER_AUTHENTICATION_SERVICE,
        };

        self.socket.send_payload(&buffer[..]).await?;

        self.match_msg(|msg| {
            if let Message::ServiceAccept { service } = msg {
                return if service != SSH_SERVICE_NAME_USER_AUTHENTICATION_SERVICE {
                    Some(Err(super::UnexpectedServiceSnafu {
                        expect: SSH_SERVICE_NAME_USER_AUTHENTICATION_SERVICE,
                        actual: service,
                    }
                    .build()
                    .into()))
                } else {
                    Some(Ok(()))
                };
            }
            None
        })
        .await
    }

    // async fn match_msg_async<R>(
    //     &mut self,
    //     mut f: impl AsyncFnMut(&mut Self, Message<'_>) -> Option<error::Result<R>>,
    // ) -> error::Result<R> {
    //     loop {
    //         let packet = self.socket.recv_packet().await?;
    //         let msg = Message::parse(&packet.payload)?;
    //         if let Some(result) = f(self, msg.clone()).await {
    //             return result;
    //         } else {
    //             self.handle_msg(msg).await?;
    //         }
    //     }
    // }

    async fn match_this_msg<R>(
        &mut self,
        mut f: impl FnMut(&mut Self, Message<'_>) -> Option<error::Result<R>>,
    ) -> error::Result<R> {
        loop {
            let packet = self.socket.recv_packet().await?;
            let msg = Message::parse(&packet.payload)?;
            if let Some(result) = f(self, msg.clone()) {
                return result;
            } else {
                self.handle_msg(msg).await?;
            }
        }
    }

    async fn match_msg<R>(
        &mut self,
        mut f: impl FnMut(Message<'_>) -> Option<error::Result<R>>,
    ) -> error::Result<R> {
        loop {
            let packet = self.socket.recv_packet().await?;
            let msg = Message::parse(&packet.payload)?;
            if let Some(result) = f(msg.clone()) {
                return result;
            } else {
                self.handle_msg(msg).await?;
            }
        }
    }

    fn next_channel_id(&self) -> u32 {
        let mut id = 0;
        for i in self.channels.iter() {
            id = id.max(i.client.id);
        }
        id + 1
    }

    async fn channel_request_break(
        &mut self,
        id: channel::IdentityPair,
        want_reply: bool,
        milliseconds: u32,
    ) -> error::Result<()> {
        let buffer = make_buffer_without_header!(
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: id.server,
            u8: if want_reply { 1 } else { 0 },
            one: "break",
            u32: milliseconds
        );

        self.socket.send_payload(&buffer[..]).await?;

        self.wait_for_channel_response(want_reply, id).await
    }

    async fn wait_for_channel_response(
        &mut self,
        want_reply: bool,
        id: IdentityPair,
    ) -> error::Result<()> {
        if want_reply {
            self.match_msg(|msg| {
                Some(match msg {
                    Message::ChannelClose { recipient_channel }
                        if recipient_channel == id.client =>
                    {
                        Err(super::UnexpectedChannelClosedSnafu.build().into())
                    }
                    Message::ChannelSuccess { recipient_channel } => {
                        if recipient_channel != id.client {
                            Err(super::UnexpectedMessageSnafu {
                                detail: "Unexpected recipient channel",
                            }
                            .build()
                            .into())
                        } else {
                            Ok(())
                        }
                    }
                    Message::ChannelFailure { recipient_channel } => {
                        if recipient_channel != id.client {
                            Err(super::UnexpectedMessageSnafu {
                                detail: "Unexpected recipient channel",
                            }
                            .build()
                            .into())
                        } else {
                            Err(super::ChannelFailureSnafu.build().into())
                        }
                    }
                    _ => return None,
                })
            })
            .await
        } else {
            Ok(())
        }
    }

    async fn channel_request_pty(
        &mut self,
        id: channel::IdentityPair,
        want_reply: bool,
        terminal: &str,
        columns: u32,
        rows: u32,
        width: u32,
        height: u32,
        modes: Vec<(TtyOpcode, u32)>,
    ) -> error::Result<()> {
        let mut producer = Producer::with_capacity(modes.capacity() * 5 + 1);

        for (opcode, value) in modes {
            producer.put_u8(opcode as u8);
            producer.put_u32(value);
        }
        producer.put_u8(TtyOpcode::TtyOpEnd as u8);

        let buffer = make_buffer_without_header!(
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: id.server,
            one: "pty-req"
            u8: want_reply.into(),
            one: terminal,
            u32: columns,
            u32: rows,
            u32: width,
            u32: height,
            bytes: &producer[..],
        );

        self.socket.send_payload(&buffer[..]).await?;

        self.wait_for_channel_response(want_reply, id).await
        // if want_reply {
        //     self.match_msg(|msg| {
        //         Some(match msg {
        //             Message::ChannelSuccess { recipient_channel } => {
        //                 if recipient_channel != id.client {
        //                     Err(super::UnexpectedMessageSnafu {
        //                         detail: "Unexpected recipient channel",
        //                     }
        //                     .build()
        //                     .into())
        //                 } else {
        //                     Ok(())
        //                 }
        //             }
        //             Message::ChannelFailure { recipient_channel } => {
        //                 if recipient_channel != id.client {
        //                     Err(super::UnexpectedMessageSnafu {
        //                         detail: "Unexpected recipient channel",
        //                     }
        //                     .build()
        //                     .into())
        //                 } else {
        //                     Err(super::ChannelFailureSnafu.build().into())
        //                 }
        //             }
        //             _ => return None,
        //         })
        //     })
        //     .await
        // } else {
        //     Ok(())
        // }
    }

    async fn channel_open_direct_tcp_ip(
        &mut self,
        initial_window_size: u32,
        maximum_packet_size: u32,
        target: forward::SocketAddr,
        source: forward::SocketAddr,
    ) -> error::Result<channel::Channel> {
        let client_id = self.next_channel_id();
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_OPEN,
            one: SSH_CHANNEL_TYPE_DIRECT_TCP_IP,
            u32: client_id,
            u32: initial_window_size,
            u32: maximum_packet_size,
            one: target.host,
            u32: target.port as u32,
            one: source.host,
            u32: source.port as u32,
        };

        self.socket.send_payload(&buffer[..]).await?;

        let client = ChannelEndpoint {
            id: client_id,
            initial_window_size,
            used_window_size: 0,
            maximum_packet_size,
            closed: false,
            eof: false,
        };

        self.wait_for_opening_channel(client).await

        // let (sender_channel, initial_window_size, maximum_packet_size) = self
        //     .match_msg(|msg| {
        //         Some(match msg {
        //             Message::ChannelOpenConfirmation {
        //                 recipient_channel,
        //                 sender_channel,
        //                 initial_window_size,
        //                 maximum_packet_size,
        //             } => {
        //                 if recipient_channel != client_id {
        //                     Err(super::UnexpectedMessageSnafu {
        //                         detail: format!(
        //                             "Unexpected recipient channel: expected {}, got {}",
        //                             client_id, recipient_channel
        //                         ),
        //                     }
        //                         .build()
        //                         .into())
        //                 } else {
        //                     Ok((sender_channel, initial_window_size, maximum_packet_size))
        //                 }
        //             }
        //             Message::ChannelOpenFailure {
        //                 recipient_channel,
        //                 reason_code,
        //                 description,
        //                 ..
        //             } => {
        //                 if recipient_channel != client_id {
        //                     tracing::error!(
        //                         "Unexpected channel open failure message: {}",
        //                         recipient_channel
        //                     );
        //                 }
        //                 Err(super::ChannelOpenFailureSnafu {
        //                     reason_code,
        //                     description,
        //                 }
        //                     .build()
        //                     .into())
        //             }
        //             _ => return None,
        //         })
        //     })
        //     .await?;
        //
        // for i in self.channels.iter() {
        //     if i.server.id == sender_channel {
        //         return Err(super::ChannelAlreadyOpenSnafu.build().into());
        //     }
        // }
        //
        // let (sender, receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
        //
        // let channel = ChannelHandle {
        //     client,
        //     server: ChannelEndpoint {
        //         id: sender_channel,
        //         initial_window_size,
        //         used_window_size: 0,
        //         maximum_packet_size,
        //         closed: false,
        //         eof: false,
        //     },
        //     sender,
        // };
        //
        // self.channels.push(channel);
        //
        // let session = self.upgrade_frontend()?;
        //
        // Ok(channel::Channel::new(
        //     IdentityPair::new(client_id, sender_channel),
        //     receiver,
        //     session,
        // ))
    }

    async fn channel_open_session(
        &mut self,
        initial_window_size: u32,
        maximum_packet_size: u32,
    ) -> error::Result<channel::Channel> {
        let client_id = self.next_channel_id();
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_OPEN,
            one: "session",
            u32: client_id,
            u32: initial_window_size,
            u32: maximum_packet_size
        };

        self.socket.send_payload(&buffer[..]).await?;

        let client = ChannelEndpoint {
            id: client_id,
            initial_window_size,
            used_window_size: 0,
            maximum_packet_size,
            closed: false,
            eof: false,
        };

        self.wait_for_opening_channel(client).await

        // let (sender_channel, initial_window_size, maximum_packet_size) = self
        //     .match_msg(|msg| {
        //         Some(match msg {
        //             Message::ChannelOpenConfirmation {
        //                 recipient_channel,
        //                 sender_channel,
        //                 initial_window_size,
        //                 maximum_packet_size,
        //             } => {
        //                 if recipient_channel != client_id {
        //                     Err(super::UnexpectedMessageSnafu {
        //                         detail: format!(
        //                             "Unexpected recipient channel: expected {}, got {}",
        //                             client_id, recipient_channel
        //                         ),
        //                     }
        //                     .build()
        //                     .into())
        //                 } else {
        //                     Ok((sender_channel, initial_window_size, maximum_packet_size))
        //                 }
        //             }
        //             Message::ChannelOpenFailure {
        //                 recipient_channel,
        //                 reason_code,
        //                 description,
        //                 ..
        //             } => {
        //                 if recipient_channel != client_id {
        //                     tracing::error!(
        //                         "Unexpected channel open failure message: {}",
        //                         recipient_channel
        //                     );
        //                 }
        //                 Err(super::ChannelOpenFailureSnafu {
        //                     reason_code,
        //                     description,
        //                 }
        //                 .build()
        //                 .into())
        //             }
        //             _ => return None,
        //         })
        //     })
        //     .await?;
        //
        // for i in self.channels.iter() {
        //     if i.server.id == sender_channel {
        //         return Err(super::ChannelAlreadyOpenSnafu.build().into());
        //     }
        // }
        //
        // let (sender, receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
        //
        // let channel = ChannelHandle {
        //     client,
        //     server: ChannelEndpoint {
        //         id: sender_channel,
        //         initial_window_size,
        //         used_window_size: 0,
        //         maximum_packet_size,
        //         closed: false,
        //         eof: false,
        //     },
        //     sender,
        // };
        //
        // self.channels.push(channel);
        //
        // let session = self.upgrade_frontend()?;
        //
        // Ok(channel::Channel::new(
        //     IdentityPair::new(client_id, sender_channel),
        //     receiver,
        //     session,
        // ))
    }

    async fn wait_for_opening_channel(
        &mut self,
        client: ChannelEndpoint,
    ) -> error::Result<channel::Channel> {
        let (sender_channel, initial_window_size, maximum_packet_size) = self
            .match_msg(|msg| {
                Some(match msg {
                    Message::ChannelOpenConfirmation {
                        recipient_channel,
                        sender_channel,
                        initial_window_size,
                        maximum_packet_size,
                    } => {
                        if recipient_channel != client.id {
                            Err(super::UnexpectedMessageSnafu {
                                detail: format!(
                                    "Unexpected recipient channel: expected {}, got {}",
                                    client.id, recipient_channel
                                ),
                            }
                            .build()
                            .into())
                        } else {
                            Ok((sender_channel, initial_window_size, maximum_packet_size))
                        }
                    }
                    Message::ChannelOpenFailure {
                        recipient_channel,
                        reason_code,
                        description,
                        language,
                    } => {
                        if !language.is_empty() {
                            tracing::debug!("Field language should be empty");
                        }
                        if recipient_channel != client.id {
                            tracing::error!(
                                "Unexpected channel open failure message: {}",
                                recipient_channel
                            );
                        }
                        Err(super::ChannelOpenFailureSnafu {
                            reason_code,
                            description,
                        }
                        .build()
                        .into())
                    }
                    _ => return None,
                })
            })
            .await?;

        for i in self.channels.iter() {
            if i.server.id == sender_channel {
                return Err(super::ChannelAlreadyOpenSnafu.build().into());
            }
        }

        let (sender, receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);

        let channel = ChannelHandle {
            client,
            server: ChannelEndpoint {
                id: sender_channel,
                initial_window_size,
                used_window_size: 0,
                maximum_packet_size,
                closed: false,
                eof: false,
            },
            sender,
        };

        self.channels.push(channel);

        let session = self.upgrade_frontend()?;

        Ok(channel::Channel::new(
            IdentityPair::new(client.id, sender_channel),
            receiver,
            session,
        ))
    }

    pub async fn authenticate_password(
        &mut self,
        username: &str,
        password: &str,
    ) -> error::Result<super::AuthenticateResult> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_USERAUTH_REQUEST,
            one: username,
            one: "ssh-connection",
            one: "password",
            u8: 0,
            one: password,
        };

        let s = self.socket.server().sequence_number;
        let c = self.socket.client().sequence_number;
        tracing::info!("Password sending: s={s}, c={c}");

        self.socket.send_payload(&buffer[..]).await?;
        self.match_this_msg(|this, msg| {
            //
            Some(match msg {
                Message::AuthenticationSuccess => {
                    this.socket.authenticated = true;
                    Ok(super::AuthenticateResult::Success)
                }
                Message::AuthenticationFailure {
                    allow_methods,
                    partial_success,
                } => Ok(super::AuthenticateResult::Failure {
                    allow_methods: allow_methods.into_iter().map(|v| v.to_string()).collect(),
                    partial_success,
                }),
                Message::UnrecognizedMessage {
                    code: SSH_MSG_USERAUTH_PASSWD_CHANGEREQ,
                    ..
                } => Ok(super::AuthenticateResult::PasswordChangeRequired),
                _ => return None,
            })
        })
        .await
    }

    pub async fn authenticate_none(
        &mut self,
        username: &str,
    ) -> error::Result<super::AuthenticateResult> {
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_USERAUTH_REQUEST,
            one: username,
            one: "ssh-connection",
            one: "none",
        };

        self.socket.send_payload(&buffer[..]).await?;

        self.match_this_msg(|this, msg| {
            //
            Some(match msg {
                Message::AuthenticationSuccess => {
                    this.socket.authenticated = true;
                    Ok(super::AuthenticateResult::Success)
                }
                Message::AuthenticationFailure {
                    allow_methods,
                    partial_success,
                } => Ok(super::AuthenticateResult::Failure {
                    allow_methods: allow_methods.into_iter().map(|v| v.to_string()).collect(),
                    partial_success,
                }),
                _ => return None,
            })
        })
        .await
    }

    pub async fn authenticate_public_key(
        &mut self,
        username: &str,
        r#type: &str,
        is_certificate: bool,
        public_blob: &[u8],
        private_blob: &[u8],
    ) -> error::Result<super::AuthenticateResult> {
        let rsa = ["rsa-sha2-512", "rsa-sha2-256", "ssh-rsa"];

        let mut signer = None;

        let compat_server_algorithms = ["rsa-sha2-256".to_string(), "rsa-sha2-512".to_string()];

        let server_algorithms =
            if !self.config.disable_compat && self.compat_options.specify_server_sign_algorithm {
                tracing::info!("Using compat algorithms: {:?}", compat_server_algorithms);
                &compat_server_algorithms[..]
            } else {
                &self.server_algorithms[..]
            };

        if r#type == "ssh-rsa" {
            for (k, v) in self.config.signer.iter() {
                if rsa.contains(&k.as_str()) {
                    if server_algorithms.contains(k) {
                        signer = Some(v());
                        break;
                    }
                }
            }
            if signer.is_none() {
                if let Some(create) = self.config.signer.get(rsa[0]) {
                    signer = Some(create())
                }
            }
        } else {
            if let Some(create) = self.config.signer.get(r#type) {
                signer = Some(create())
            }
        }

        let Some(mut signer) = signer else {
            return Err(super::UnsupportedKeyTypeSnafu { r#type }.build().into());
        };

        tracing::info!("Using signer: {}", signer.name());

        if self
            .server_algorithms
            .iter()
            .position(|v| v == r#type)
            .is_none()
        {
            tracing::warn!(
                "We don't whether server supports this algorithm: {}, try anyway",
                signer.name()
            );
        }

        let r#type = signer.name().to_string();

        let method = if is_certificate {
            format!("{}{}", r#type, crate::key::CERT_SUFFIX)
        } else {
            r#type.clone()
        };

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_USERAUTH_REQUEST,
            one: username,
            one: b"ssh-connection"
            one: b"publickey",
            u8: 0,
            one: &method,
            one: public_blob,
        };

        self.socket.send_payload(&buffer[..]).await?;

        let msg = self
            .match_this_msg(|this, msg| {
                Some(match msg {
                    Message::AuthenticationSuccess => {
                        this.socket.authenticated = true;
                        Ok(Some(super::AuthenticateResult::Success))
                    }
                    Message::AuthenticationFailure {
                        partial_success,
                        allow_methods,
                    } => Ok(Some(super::AuthenticateResult::Failure {
                        partial_success,
                        allow_methods: allow_methods.into_iter().map(|v| v.to_string()).collect(),
                    })),
                    Message::UnrecognizedMessage {
                        code: SSH_MSG_USERAUTH_PK_OK,
                        ..
                    } => Ok(None),
                    _ => return None,
                })
            })
            .await?;

        if let Some(msg) = msg {
            return Ok(msg);
        }

        let buffer = make_buffer_without_header! {
            one: &self.session_id,
            u8: SSH_MSG_USERAUTH_REQUEST,
            one: username,
            one: "ssh-connection",
            one: b"publickey",
            u8: 1,
            one: &method,
            one: public_blob,
        };

        signer.initialize(private_blob)?;

        let signature = signer.signature(&buffer[..])?;

        let len = 4 + r#type.len() + 4 + signature.len();
        let len = len as u32;
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_USERAUTH_REQUEST,
            one: username,
            one: b"ssh-connection",
            one: b"publickey",
            u8: 1,
            one: method,
            one: public_blob,
            u32: len,
            one: r#type,
            one: &signature,
        };

        self.socket.send_payload(&buffer[..]).await?;

        // let mut buffer = make_buffer_without_header! {
        // };

        self.match_this_msg(|this, msg| {
            Some(match msg {
                Message::AuthenticationSuccess => {
                    this.socket.authenticated = true;
                    Ok(super::AuthenticateResult::Success)
                }
                Message::AuthenticationFailure {
                    partial_success,
                    allow_methods,
                } => Ok(super::AuthenticateResult::Failure {
                    partial_success,
                    allow_methods: allow_methods.into_iter().map(|v| v.to_string()).collect(),
                }),
                _ => return None,
            })
        })
        .await
    }

    pub async fn send_debug_message(
        &mut self,
        always_display: bool,
        message: &str,
    ) -> error::Result<()> {
        let buf = make_buffer_without_header! {
            u8: SSH_MSG_DEBUG,
            u8: always_display.into(),
            one: message,
            one: ""
        };

        self.socket.send_payload(&buf[..]).await?;

        Ok(())
    }

    pub async fn notify_exited(&mut self, result: error::Result<()>) {
        self.notifier.exited(result).await;
    }

    pub async fn exec(&mut self) -> error::Result<()> {
        loop {
            tokio::select! {
                packet = self.socket.recv_packet() => {
                    let packet = packet?;
                    let msg = Message::parse(&packet.payload)?;

                    self.handle_msg(msg).await?;

                },
                event = self.receiver.recv() => {
                    if let Some(event) = event {
                        self.handle_event(event).await?;
                    } else {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}
