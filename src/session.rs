use std::cmp::min;
use std::collections::HashMap;

use super::channel::ChannelOpenFailureReson;
use super::handshake;
use super::keys::KeyParser;
use super::ssh::stream::CipherStream;

use crate::channel::ChannelInner;
use crate::channel::Endpoint as ChannelEndpoint;
use crate::channel::TerminalMode;
use crate::cipher::kex::Dependency;

use crate::cipher::sign;

use crate::error::builder;
use crate::error::Error;
use crate::error::Result;
use crate::forward::Listener;
use crate::forward::{SocketAddr, Stream};

use super::channel::Message as ChannelMsg;
use crate::handshake::Behavior;
use crate::sftp::SFtp;
use crate::ssh::{
    buffer::Buffer,
    common::{code::*, PAYLOAD_MAXIMUM_SIZE, SFTP_VERSION},
    stream::BufferStream,
};

use super::channel::{Channel, ExitStatus};
use super::msg::Message;
use super::msg::Request;
use crate::ssh::stream::PlainStream;
use derive_new::new;
use snafu::OptionExt;
use tokio::io::{AsyncRead, AsyncWrite};

use super::{m_channel, o_channel};
use super::{MReceiver, MSender, MWSender};

macro_rules! channel_loop {
    ($sel:ident,$id:expr,$($e:pat $(if $c:expr )? => $h:expr,)*) => {
        loop {
            match $sel.recv_msg().await? {
                Message::ChannelClose(recipient) if recipient == $id => {
                    $sel.handle_channel_close(recipient).await?;
                    return  builder::ChannelClosed.fail();//Err(Error::ChannelClosed);
                }
                $(
                    $e $( if $c )? => $h,
                )*
                msg => {
                    $sel.handle_msg(msg).await?;
                }
            }
        }
    };
}

#[derive(new)]
struct Endpoint {
    banner: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Userauth {
    Success,
    Failure(Vec<String>, bool),
    Expired,
}

#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct DisconnectReson(pub u32);

#[async_trait::async_trait]
pub trait Interactive: Send {
    async fn response(
        &mut self,
        name: &str,
        instruction: &str,
        prompts: &[(&str, bool)],
    ) -> Result<Vec<String>>;
}

// #[async_trait::async_trait]
// impl<T> Interactive for T
// where
//     T: Send + FnMut(&str, &str, &[(&str, bool)]) -> Result<Vec<String>>,
// {
//     async fn response(
//         &mut self,
//         name: &str,
//         instruction: &str,
//         prompts: &[(&str, bool)],
//     ) -> Result<Vec<String>> {
//         let res = self(name, instruction, prompts)?;

//         Ok(res)
//     }
// }

impl DisconnectReson {
    pub const HOST_NOT_ALLOWED_TO_CONNECT: Self = Self(SSH_DISCONNECT_HOST_NOT_ALLOWED_TO_CONNECT);
    pub const PROTOCOL_ERROR: Self = Self(SSH_DISCONNECT_PROTOCOL_ERROR);
    pub const KEY_EXCHANGE_FAILED: Self = Self(SSH_DISCONNECT_KEY_EXCHANGE_FAILED);
    pub const RESERVED: Self = Self(SSH_DISCONNECT_RESERVED);
    pub const MAC_ERROR: Self = Self(SSH_DISCONNECT_MAC_ERROR);
    pub const COMPRESSION_ERROR: Self = Self(SSH_DISCONNECT_COMPRESSION_ERROR);
    pub const SERVICE_NOT_AVAILABLE: Self = Self(SSH_DISCONNECT_SERVICE_NOT_AVAILABLE);
    pub const PROTOCOL_VERSION_NOT_SUPPORTED: Self =
        Self(SSH_DISCONNECT_PROTOCOL_VERSION_NOT_SUPPORTED);
    pub const HOST_KEY_NOT_VERIFIABLE: Self = Self(SSH_DISCONNECT_HOST_KEY_NOT_VERIFIABLE);
    pub const CONNECTION_LOST: Self = Self(SSH_DISCONNECT_CONNECTION_LOST);
    pub const BY_APPLICATION: Self = Self(SSH_DISCONNECT_BY_APPLICATION);
    pub const TOO_MANY_CONNECTIONS: Self = Self(SSH_DISCONNECT_TOO_MANY_CONNECTIONS);
    pub const AUTH_CANCELLED_BY_USER: Self = Self(SSH_DISCONNECT_AUTH_CANCELLED_BY_USER);
    pub const NO_MORE_AUTH_METHODS_AVAILABLE: Self =
        Self(SSH_DISCONNECT_NO_MORE_AUTH_METHODS_AVAILABLE);
    pub const ILLEGAL_USER_NAME: Self = Self(SSH_DISCONNECT_ILLEGAL_USER_NAME);
}

pub struct Session {
    sender: Option<MSender<Request>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").finish()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(ref mut sender) = self.sender {
            let request = Request::SessionDrop {
                reson: DisconnectReson::BY_APPLICATION,
                desc: "exit".to_string(),
                sender: None,
            };

            let _ = sender.send(request);
        }
    }
}

impl Session {
    fn new(sender: MSender<Request>) -> Self {
        Self {
            sender: sender.into(),
        }
    }

    fn send_request(&self, msg: Request) -> Result<()> {
        self.sender
            .as_ref()
            .context(builder::BadOperation {
                detail: "Session has been disconnected",
            })?
            .send(msg)
            .map_err(|_| builder::Disconnected.build())
    }

    // fn manually_drop(&mut self) {
    //     unsafe { ManuallyDrop::drop(&mut self.sender) }
    // }

    pub async fn rexchange(&self) -> Result<()> {
        let (sender, recver) = o_channel();

        let request = Request::KeyReExchange(sender);

        self.send_request(request)?;

        recver.await?
    }

    #[inline]
    pub async fn direct_tcpip_default(
        &self,
        remote: SocketAddr,
        local: SocketAddr,
    ) -> Result<Stream> {
        self.direct_tcpip(2 * 1024 * 1024, 32000, remote, local)
            .await
    }

    pub async fn direct_tcpip(
        &self,
        initial: u32,
        maximum: u32,
        remote: SocketAddr,
        local: SocketAddr,
    ) -> Result<Stream> {
        let (sender, recver) = o_channel();
        let address = remote.clone();
        self.send_request(Request::DirectTcpip {
            initial,
            maximum,
            remote,
            local,
            sender,
        })?;
        let channel = recver.await??;

        Ok(Stream::new(channel, address))
    }

    pub async fn tcpip_forward_default(
        &self,
        address: SocketAddr,
        // port: u32,
    ) -> Result<Listener> {
        self.tcpip_forward(address, 2 * 1024 * 1024, 3200).await
    }

    #[inline]
    pub async fn tcpip_forward(
        &self,
        address: SocketAddr,
        // port: u32,
        initial: u32,
        maximum: u32,
    ) -> Result<Listener> {
        let (sender, recver) = o_channel();
        self.send_request(Request::TcpipForward {
            address,
            // port,
            initial,
            maximum,
            sender,
        })?;

        recver.await?
    }

    #[inline]
    pub async fn disconnect_default(self) -> Result<()> {
        self.disconnect(DisconnectReson::BY_APPLICATION, "exit")
            .await
    }

    pub async fn disconnect(
        mut self,
        reson: DisconnectReson,
        desc: impl Into<String>,
    ) -> Result<()> {
        let (sender, recver) = o_channel();
        let request = Request::SessionDrop {
            reson,
            desc: desc.into(),
            sender: Some(sender),
        };
        let res = async {
            self.send_request(request)?;

            recver.await?
        }
        .await;

        // self.manually_drop();
        // std::mem::forget(self);
        //

        self.sender = None;

        res
    }

    #[inline]
    pub async fn sftp_open_default(&self) -> Result<SFtp> {
        self.sftp_open(2 * 1024 * 1024, 32000).await
    }

    pub async fn sftp_open(&self, initial: u32, maximum: u32) -> Result<SFtp> {
        let (sender, recver) = o_channel();
        self.send_request(Request::SFtpOpen {
            initial,
            maximum,
            sender,
        })?;

        recver.await?
    }

    #[inline]
    pub async fn channel_open_default(&self) -> Result<Channel> {
        self.channel_open(1024 * 1024, 32768).await
    }

    pub async fn channel_open(&self, initial: u32, maximum: u32) -> Result<Channel> {
        let (sender, recver) = o_channel();
        let msg = Request::ChannelOpenSession {
            initial,
            maximum,
            sender,
        };

        self.send_request(msg)?;

        recver.await?
    }

    pub async fn userauth_none(&self, username: impl Into<String>) -> Result<Userauth> {
        let username = username.into();

        let (sender, recver) = o_channel();

        let requset = Request::UserauthNone { username, sender };

        self.send_request(requset)?;

        recver.await?
    }

    pub async fn userauth_publickey_from_file(
        &self,
        username: impl Into<String>,
        privatekey: impl AsRef<[u8]>,
        publickey: Option<&[u8]>,
        passphrase: Option<&[u8]>,
    ) -> Result<Userauth> {
        let openssh = KeyParser::default();

        let mut private = openssh.parse_privatekey(privatekey.as_ref(), passphrase)?;

        if let Some(pb) = publickey {
            let public = openssh.parse_publickey(pb)?;

            if public.key_type != private.key_type {
                return Err(Error::invalid_format("Cipher doest't match"));
            }

            if public.key != private.public_key {
                return Err(Error::invalid_format("Public key does't match"));
            }
        }
        if private.key_type == "ssh-rsa" {
            private.key_type = "rsa-sha2-256".to_string();
        }
        self.userauth_publickey(
            username,
            private.key_type,
            private.public_key,
            private.private_key,
        )
        .await
    }

    pub async fn userauth_publickey(
        &self,
        username: impl Into<String>,
        method: impl Into<String>,
        publickey: impl Into<Vec<u8>>,
        privatekey: impl Into<Vec<u8>>,
    ) -> Result<Userauth> {
        let (sender, recver) = o_channel();
        let request = Request::UserauthPublickey {
            username: username.into(),
            method: method.into(),
            publickey: publickey.into(),
            privatekey: privatekey.into(),
            sender,
        };
        self.send_request(request)?;

        recver.await?
    }

    pub async fn userauth_password(
        &self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Userauth> {
        let username = username.into();
        let password = password.into();

        let (sender, recver) = o_channel();

        let request = Request::UserAuthPassWord {
            username,
            password,
            sender,
        };

        self.sender
            .as_ref()
            .context(builder::BadOperation {
                detail: "Session has been disconnected",
            })?
            .send(request)
            .map_err(|_| builder::Disconnected.build())?;
        recver.await?
    }

    pub async fn userauth_keyboard_interactive<T: Interactive + 'static>(
        &self,
        username: impl Into<String>,
        prefer: impl IntoIterator<Item = impl Into<String>>,
        cb: T,
    ) -> Result<bool> {
        let (sender, recver) = o_channel();
        let request = Request::UserauthKeyboardInteractive {
            username: username.into(),
            submethods: prefer.into_iter().map(|v| v.into()).collect(),
            cb: Box::new(cb),
            sender,
        };

        self.send_request(request)?;

        recver.await?
    }

    pub async fn handshake<T, B>(mut config: handshake::Config<B>, socket: T) -> Result<Self>
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
        B: Behavior + Send + Sync + 'static,
    {
        let mut buffer_stream = BufferStream::new(socket);

        let (banner, _) =
            handshake::banner_exchange(&mut buffer_stream, config.banner.as_str()).await?;

        let mut plain_stream = PlainStream::new(buffer_stream);
        plain_stream.client.ext = config.ext;
        plain_stream.client.kex_strict = config.key_strict;

        let meex = handshake::method_exchange(&mut plain_stream, &config).await?;

        // socket.kex_strict = config.key_strict && meex.server.1.kex_strict;
        plain_stream.server.kex_strict = meex.server.methods.kex_strict;
        plain_stream.server.ext = meex.server.methods.ext;
        let mut algo =
            handshake::match_method(&meex.client.methods, &meex.server.methods, &config)?;

        let dhconfig = Dependency::new(
            config.banner.clone(),
            meex.client.binary,
            banner.clone(),
            meex.server.binary,
            plain_stream.client.kex_strict && plain_stream.server.kex_strict,
        );

        let mut result = algo.kex.kex(dhconfig, &mut plain_stream).await?;

        algo.hostkey.initialize(&result.server_hostkey)?;

        let res = algo
            .hostkey
            .verify(&result.server_signature, &result.client_hash)?;

        // if !res {
        // return Err(Error::HostKeyVerifyFailed);
        // }

        snafu::ensure!(res, builder::HostKeyVerifyFailed);

        if let Some(ref mut behavior) = config.behavior {
            if !behavior
                .verify_server_hostkey(algo.hostkey.name(), &result.server_hostkey)
                .await?
            {
                // return Err(Error::RejectByUser(
                //     "Server public key rejected by user".to_string(),
                // ));
                return builder::RejectByUser {
                    tip: "Server public key rejected by user",
                }
                .fail();
            }
        }

        handshake::new_keys(&mut plain_stream).await?;

        algo.initialize(&mut result)?;

        let (sender, recver) = m_channel();

        let weak_sender = sender.downgrade();
        let session = Session::new(sender);

        let stream = plain_stream.encrypt(
            (algo.client_crypt, algo.client_mac, algo.client_compress),
            (algo.server_crypt, algo.server_mac, algo.server_compress),
        );
        let mut inner = SessionInner::new(
            result.session_id,
            stream,
            Endpoint::new(config.banner.clone()),
            Endpoint::new(banner),
            recver,
            weak_sender,
            config,
        );
        inner.request_userauth_service().await?;
        inner.run();
        Ok(session)
    }
}

#[derive(new)]
struct ListenerInner {
    sender: MSender<Stream>,
    initial: u32,
    maximum: u32,
}

#[derive(new)]
struct SessionInner<T, B>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
    B: Behavior + Send + 'static,
{
    session_id: Vec<u8>,
    stream: CipherStream<T>,
    client: Endpoint,
    server: Endpoint,
    recver: MReceiver<Request>,

    #[new(default)]
    channels: HashMap<u32, ChannelInner>,

    #[new(default)]
    listeners: HashMap<SocketAddr, ListenerInner>,
    weak_sender: MWSender<Request>,
    // behaivor: Option<B>,
    config: handshake::Config<B>,
}

impl<T, B> SessionInner<T, B>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    B: Behavior + Send + Sync,
{
    fn run(mut self) {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    request = self.recver.recv() => {
                        let Some(request) = request else {
                            self.session_disconnect(DisconnectReson::BY_APPLICATION, "exit").await?;
                            break;
                        };
                        if self.handle_request(request).await {
                            break;
                        }
                    }
                    packet = self.stream.recv_packet() => {
                        let msg = Message::parse(&packet?.payload).map_err(Error::invalid_format)?;
                        self.handle_msg(msg).await?;
                    }
                }
            }
            Result::Ok(())
        });
    }

    async fn session_disconnect(&mut self, reson: DisconnectReson, desc: &str) -> Result<()> {
        // let mut buffer = Buffer::new();
        // buffer.put_u8(SSH_MSG_DISCONNECT);
        // buffer.put_u32(reson.0);
        // buffer.put_one(desc);
        // buffer.put_u32(0);
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_DISCONNECT,
            u32: reson.0,
            one: desc,
            u32: 0
        };
        self.stream.send_payload(buffer).await
    }

    fn genarate_channel_id(&self) -> u32 {
        let mut next = 0;
        for (_, channel) in self.channels.iter() {
            if (!channel.server.closed || !channel.client.closed) && next <= channel.client.id {
                next = channel.client.id + 1;
            }
        }

        next
    }

    async fn request_userauth_service(&mut self) -> Result<()> {
        // let mut buffer = Buffer::new();
        // buffer.put_u8(SSH_MSG_SERVICE_REQUEST);
        // buffer.put_one(b"ssh-userauth");

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_SERVICE_REQUEST,
            one: "ssh-userauth",
        };

        self.stream.send_payload(buffer).await?;

        loop {
            let msg = self.recv_msg().await?;
            if let Message::UserauthServiceAccept = msg {
                return Ok(());
            }

            self.handle_msg(msg).await?;
        }
    }

    async fn recv_msg(&mut self) -> Result<Message> {
        let packet = self.stream.recv_packet().await?;
        Message::parse(&packet.payload).map_err(Error::invalid_format)
    }

    async fn handle_msg(&mut self, msg: Message) -> Result<()> {
        match msg {
            Message::ChannelStdoutData { recipient, data } => {
                self.append_channel_stdout(recipient, data).await?;
            }
            Message::ChannelWindowAdjust { recipient, count } => {
                self.add_channel_bytes_count(recipient, count)?;
                // self.channel_flush_stdout(recipient).await?;
            }
            Message::ChannelStderrData { recipient, data } => {
                self.append_channel_stderr(recipient, data).await?;
            }
            Message::ChannelEof(recipient) => {
                self.handle_channel_eof(recipient);
            }
            Message::ChannelClose(recipient) => {
                self.handle_channel_close(recipient).await?;
            }
            Message::ChannelExitSignal {
                recipient,
                signal,
                core_dumped,
                error_msg,
                tag,
            } => {
                let channel = self.get_server_channel(recipient)?;

                let _ = channel
                    .sender
                    .send(ChannelMsg::Exit(ExitStatus::new_interrupt(
                        signal,
                        core_dumped,
                        error_msg,
                        tag,
                    )));

                // channel.exit_status = Some(ExitStatus::new_interrupt(
                //     signal,
                //     core_dumped,
                //     error_msg,
                //     tag,
                // ));
            }
            Message::ChannelExitStatus { recipient, status } => {
                let channel = self.get_server_channel(recipient)?;
                let _ = channel
                    .sender
                    .send(ChannelMsg::Exit(ExitStatus::new_normal(status)));
                // channel.exit_status = Some(ExitStatus::new_normal(status));
            }
            Message::Disconnect {
                reason,
                description,
                tag,
            } => {
                if let Some(behavior) = self.behaviour() {
                    behavior.disconnect(reason, &description, &tag).await?;
                }
                return builder::Disconnected.fail(); //Err(Error::Disconnected);
            }
            Message::Ping(data) => {
                self.session_pong(data).await?;
            }
            Message::HostKeysOpenSSH {
                want_reply,
                hostkeys,
            } => {
                if let Some(behavior) = self.behaviour() {
                    let hostkeys: Vec<&[u8]> = hostkeys.iter().map(|v| v.as_slice()).collect();
                    behavior.openssh_hostkeys(want_reply, &hostkeys).await?;
                }
            }
            Message::UserauthBanner { msg, tag } => {
                if let Some(behavior) = self.behaviour() {
                    behavior.useauth_banner(&msg, &tag).await?;
                }
            }
            Message::Debug {
                always_display,
                msg,
                tag,
            } => {
                if let Some(behavior) = self.behaviour() {
                    behavior.debug(always_display, &msg, &tag).await?;
                }
            }
            Message::Ignore(data) => {
                if let Some(behavior) = self.behaviour() {
                    behavior.ignore(&data).await?;
                }
            }
            Message::Unimplemented(seqno) => {
                return builder::Unimplemented {
                    sequence_number: seqno,
                }
                .fail();
            }
            Message::ForwardTcpIp {
                sender,
                initial,
                maximum,
                listen_address,
                listen_port,
                originator_address,
                originator_port,
            } => {
                self.accpet_tcpip_forward(
                    sender,
                    initial,
                    maximum,
                    listen_address,
                    listen_port,
                    originator_address,
                    originator_port,
                )
                .await?;
            }
            Message::GlobalKeepAliveOpenSSH { want_reply } => {
                self.handle_global_keep_alive(want_reply).await?;
            }
            Message::ExtInfo(ext) => {
                if let Some(behavior) = self.behaviour() {
                    for (name, value) in ext {
                        if name == "server-sig-algs" {
                            let value = std::str::from_utf8(&value)?;
                            let algos = value.split(',').collect::<Vec<_>>();
                            behavior.server_signature_algorithms(&algos).await?;
                        }
                    }
                }
            }
            Message::X11Forward {
                sender,
                initial,
                maximum,
                address,
                port,
            } => {
                self.handle_x11_forward(sender, initial, maximum, address, port)
                    .await?;
            }
            Message::ChannelKeepAliveOpenSSH {
                recipient,
                want_reply,
            } => {
                self.handle_channel_keep_alive(want_reply, recipient)
                    .await?;
            }
            Message::KexIntial(payload) => {
                self.handle_rexchange(&payload).await?;
            }
            msg => {
                println!("msg :{:?} from server is ignore", msg);
            }
        }

        Ok(())
    }

    async fn handle_rexchange(&mut self, payload: &[u8]) -> Result<()> {
        let meex = handshake::method_exchange_with_payload(&mut self.stream, payload, &self.config)
            .await?;

        self.stream.server.kex_strict = meex.server.methods.kex_strict;
        self.stream.server.ext = meex.server.methods.ext;

        let mut algo =
            handshake::match_method(&meex.client.methods, &meex.server.methods, &self.config)?;

        let dhconfig = Dependency::new(
            self.client.banner.clone(),
            meex.client.binary,
            self.server.banner.clone(),
            meex.server.binary,
            self.stream.client.kex_strict && self.stream.server.kex_strict,
        );
        let mut result = algo.kex.kex(dhconfig, &mut self.stream).await?;
        // result.session_id = self.session_id.clone();
        // result.client_hash = self.session_id.clone();

        algo.hostkey.initialize(&result.server_hostkey)?;

        let res = algo
            .hostkey
            .verify(&result.server_signature, &result.client_hash)?;

        // if !res {
        //     return Err(Error::HostKeyVerifyFailed);
        // }
        snafu::ensure!(res, builder::HostKeyVerifyFailed);

        if let Some(behavior) = self.behaviour() {
            if !behavior
                .verify_server_hostkey(algo.hostkey.name(), &result.server_hostkey)
                .await?
            {
                // return Err(Error::RejectByUser(
                //     "Server public key rejected by user".to_string(),
                // ));
                return builder::RejectByUser {
                    tip: "Server public key rejected by user",
                }
                .fail();
            }
        }

        handshake::new_keys(&mut self.stream).await?;

        result.session_id.clone_from(&self.session_id);

        algo.initialize(&mut result)?;

        self.stream.decrypt = algo.server_crypt;
        self.stream.encrypt = algo.client_crypt;
        self.stream.server.mac = algo.server_mac;
        self.stream.client.mac = algo.client_mac;
        self.stream.decode = algo.server_compress;
        self.stream.encode = algo.client_compress;

        Ok(())
    }

    fn behaviour(&mut self) -> Option<&mut B> {
        // if let Some(ref mut config) = self.config {
        //     if let Some(ref mut b) = config.behavior {
        //         return  Some(b);
        //     }
        // }
        // None
        self.config.behavior.as_mut()
    }

    fn upgrade_sender(&self) -> Result<MSender<Request>> {
        self.weak_sender
            .upgrade()
            .ok_or(Error::ub("Session has been dropped"))
    }

    async fn handle_channel_keep_alive(&mut self, want_reply: bool, recipient: u32) -> Result<()> {
        if want_reply {
            if let Ok(id) = self.get_server_channel_id(recipient) {
                let buffer = make_buffer_without_header! {
                    u8: SSH_MSG_CHANNEL_SUCCESS,
                    u32: id,
                };
                self.stream.send_payload(buffer).await?;
            }
        }

        Ok(())
    }

    async fn handle_global_keep_alive(&mut self, want_reply: bool) -> Result<()> {
        if want_reply {
            self.stream.send_payload(&[SSH_MSG_REQUEST_SUCCESS]).await?;
        }
        Ok(())
    }

    async fn handle_x11_forward(
        &mut self,
        sender: u32,
        initial: u32,
        maximum: u32,
        originator_address: String,
        originator_port: u32,
    ) -> Result<()> {
        let session = self.upgrade_sender()?;
        let client_id = self.genarate_channel_id();
        if let Some(behavior) = self.config.behavior.as_mut() {
            let buffer = make_buffer_without_header! {
                u8: SSH_MSG_CHANNEL_OPEN_CONFIRMATION,
                u32: sender,
                u32: client_id,
                u32: 1024 * 1024,
                u32: 32768
            };

            let (tx, rx) = m_channel();

            self.stream.send_payload(buffer).await?;

            let channel = Channel::new(client_id, rx, session);
            use super::channel::Endpoint as ChannelEp;

            let inner = ChannelInner::new(
                ChannelEp::new(client_id, 1024 * 1024, 32768),
                ChannelEp::new(sender, initial, maximum),
                tx,
            );

            self.channels.insert(client_id, inner);

            let socket = Stream::new(
                channel,
                SocketAddr::new(originator_address, originator_port),
            );

            behavior.x11_forward(socket).await?;
        } else {
            let buffer = make_buffer_without_header! {
                u8: SSH_MSG_CHANNEL_OPEN_FAILURE,
                u32: sender,
                u32: ChannelOpenFailureReson::UNKNOWN_CHANNELTYPE.0,
                one: "ignore x11 forward",
                u32: 0
            };

            self.stream.send_payload(buffer).await?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn accpet_tcpip_forward(
        &mut self,
        sender: u32,
        initial: u32,
        maximum: u32,
        listen_address: String,
        listen_port: u32,
        originator_address: String,
        originator_port: u32,
    ) -> Result<()> {
        let listen = SocketAddr::new(listen_address, listen_port);

        let mut buffer = Buffer::with_capacity(128);
        match self.listeners.get(&listen) {
            Some(listener) => {
                let session = self.upgrade_sender()?;
                buffer.put_u8(SSH_MSG_CHANNEL_OPEN_CONFIRMATION);
                buffer.put_u32(sender);

                let client_id = self.genarate_channel_id();

                buffer.put_u32(client_id);
                buffer.put_u32(listener.initial);
                buffer.put_u32(listener.maximum);

                self.stream.send_payload(buffer).await?;

                let (tx, rx) = m_channel();

                let channel = Channel::new(client_id, rx, session);
                use super::channel::Endpoint as ChannelEp;

                let inner = ChannelInner::new(
                    ChannelEp::new(client_id, listener.initial, listener.maximum),
                    ChannelEp::new(sender, initial, maximum),
                    tx,
                );

                self.channels.insert(client_id, inner);

                let socket = Stream::new(
                    channel,
                    SocketAddr::new(originator_address, originator_port),
                );
                let _ = listener.sender.send(socket);
            }
            None => {
                buffer.put_u8(SSH_MSG_CHANNEL_OPEN_FAILURE);
                buffer.put_u32(sender);
                buffer.put_u32(ChannelOpenFailureReson::UNKNOWN_CHANNELTYPE.0);
                buffer.put_one("Listener not found");
                buffer.put_u32(0); // tag
                self.stream.send_payload(buffer).await?;
            }
        }

        Ok(())
    }

    async fn session_pong(&mut self, data: Vec<u8>) -> Result<()> {
        // let mut buffer = Buffer::new();
        // buffer.put_u8(SSH2_MSG_PONG);
        // buffer.put_one(data);

        let buffer = make_buffer_without_header! {
            u8: SSH2_MSG_PONG,
            one: data
        };

        self.stream.send_payload(buffer).await
    }

    fn handle_channel_eof(&mut self, id: u32) -> bool {
        let Some(channel) = self.channels.get_mut(&id) else {
            return false;
        };

        channel.server_eof();

        true
    }

    fn add_channel_bytes_count(&mut self, recipeint: u32, count: u32) -> Result<()> {
        self.get_server_channel(recipeint)?.server.size += count;
        Ok(())
    }

    // async fn channel_window_adjust(
    //     // stream: &mut CipherStream<T>,
    //     &mut self,
    //     channel: &mut ChannelInner,
    //     count: u32,
    // ) -> Result<()> {
    //     let mut buffer = Buffer::new();
    //     buffer.put_u8(SSH_MSG_CHANNEL_WINDOW_ADJUST);
    //     buffer.put_u32(channel.server.id);
    //     buffer.put_u32(count);
    //     self.stream.send_payload(buffer.as_ref()).await?;
    //     channel.client.size += count;
    //     Ok(())
    // }

    async fn append_channel_stderr(&mut self, recipient: u32, data: Vec<u8>) -> Result<bool> {
        if let Some(channel) = self.channels.get_mut(&recipient) {
            channel.client.size -= data.len() as u32;

            let value = channel.sender.send(ChannelMsg::Stderr(data)).is_ok();
            // let value = channel.stderr.write(data).await.is_ok();
            if channel.client.size < channel.client.maximum {
                let count = channel.client.initial - channel.client.size;
                // let mut buffer = Buffer::new();
                // buffer.put_u8(SSH_MSG_CHANNEL_WINDOW_ADJUST);
                // buffer.put_u32(channel.server.id);
                // buffer.put_u32(count);
                let buffer = make_buffer_without_header! {
                    u8: SSH_MSG_CHANNEL_WINDOW_ADJUST,
                    u32: channel.server.id,
                    u32: count,
                };
                self.stream.send_payload(buffer).await?;
                channel.client.size += count;
            }
            Ok(value)
        } else {
            Ok(false)
        }
    }

    async fn append_channel_stdout(&mut self, recipient: u32, data: Vec<u8>) -> Result<bool> {
        if let Some(channel) = self.channels.get_mut(&recipient) {
            channel.client.size -= data.len() as u32;
            let value = channel.sender.send(ChannelMsg::Stdout(data)).is_ok();
            // let value = channel.stdout.write(data).await.is_ok();
            if channel.client.size < channel.client.maximum {
                let count = channel.client.initial - channel.client.size;
                // let mut buffer = Buffer::new();
                // buffer.put_u8(SSH_MSG_CHANNEL_WINDOW_ADJUST);
                // buffer.put_u32(channel.server.id);
                // buffer.put_u32(count);

                let buffer = make_buffer_without_header! {
                    u8: SSH_MSG_CHANNEL_WINDOW_ADJUST,
                    u32: channel.server.id,
                    u32: count,
                };

                self.stream.send_payload(buffer).await?;
                channel.client.size += count;
            }
            Ok(value)
        } else {
            Ok(false)
        }
    }

    async fn handle_request(&mut self, request: Request) -> bool {
        match request {
            Request::UserAuthPassWord {
                username,
                password,
                sender,
            } => {
                let res = self.userauth_password(&username, &password).await;
                let _ = sender.send(res);
            }
            Request::ChannelOpenSession {
                initial,
                maximum,
                sender,
            } => {
                let res = self.channel_open_session(initial, maximum).await;

                let _ = sender.send(res);
            }
            Request::ChannelExec { id, cmd, sender } => {
                let res = self.channel_exec(id, &cmd).await;
                let _ = sender.send(res);
            }
            Request::ChannelDrop { id, sender } => {
                let res = self.channel_drop(id).await;
                if let Some(sender) = sender {
                    let _ = sender.send(res);
                }
            }
            Request::ChannelWriteStdout { id, data, sender } => {
                let res = self.channel_write_stdout(id, &data).await;

                let _ = sender.send(res);
            }
            Request::SFtpOpen {
                initial,
                maximum,
                sender,
            } => {
                let res = self.sftp_open(initial, maximum).await;

                let _ = sender.send(res);
            }
            Request::UserauthNone { username, sender } => {
                let res = self.userauth_none(&username).await;
                let _ = sender.send(res);
            }
            Request::UserauthPublickey {
                username,
                method,
                publickey,
                privatekey,
                sender,
            } => {
                let res = self
                    .userauth_publickey(&username, &method, &publickey, &privatekey)
                    .await;
                let _ = sender.send(res);
            }
            // Request::ChannelExecWait { id, cmd, sender } => {
            //     let res = self.channel_exec_wait(id, &cmd).await;

            //     let _ = sender.send(res);
            // }
            // Request::ChannelGetExitStatus { id, sender } => {
            //     let res = self.wait_for_finish(id).await;

            //     let _ = sender.send(res);
            // }
            Request::ChannelEof { id, sender } => {
                let res = self.channel_eof(id).await;

                let _ = sender.send(res);
            }
            Request::ChannelSetEnv {
                id,
                name,
                value,
                sender,
            } => {
                let res = self.channel_set_env(id, &name, &value).await;
                let _ = sender.send(res);
            }
            Request::ChannelSendSignal { id, signal, sender } => {
                let res = self.channel_send_signal(id, &signal.0).await;
                let _ = sender.send(res);
            }
            Request::SessionDrop {
                reson,
                desc,
                sender,
            } => {
                let res = self.session_disconnect(reson, &desc).await;
                if let Some(sender) = sender {
                    let _ = sender.send(res);
                }
                return true;
            }
            // Request::ChannelFlushStdout { id, sender } => {
            //     // let res = self.channel_flush_stdout_blocking(id).await;

            //     // let _ = sender.send(res);
            // }
            Request::ChannelReuqestShell { id, sender } => {
                let _ = sender.send(self.channel_request_shell(id).await);
            }
            Request::SFtpFromChannel { channel, sender } => {
                let func = async {
                    let inner = self
                        .channels
                        .remove(&channel.id)
                        .ok_or(Error::ub("Channel not found"))?;

                    self.sftp_from_channel(channel, inner).await
                };

                let _ = sender.send(func.await);
            }
            Request::TcpipForward {
                address,
                // port,
                initial,
                maximum,
                sender,
            } => {
                let _ = sender.send(self.tcpip_forward(address, initial, maximum).await);
            }
            Request::CancelTcpipForward {
                address,
                // port,
                sender,
            } => {
                let res = self.cancel_tcpip_forward(&address).await;

                if let Some(sender) = sender {
                    let _ = sender.send(res);
                }
            }
            Request::DirectTcpip {
                initial,
                maximum,
                remote,
                local,
                sender,
            } => {
                let _ = sender.send(self.direct_tcpip(initial, maximum, remote, local).await);
            }
            Request::ChannelRequestPty {
                id,
                term,
                columns,
                rows,
                width,
                height,
                terimal_modes,
                sender,
            } => {
                let res = self
                    .channel_request_pty(id, &term, columns, rows, width, height, &terimal_modes)
                    .await;
                let _ = sender.send(res);
            }
            Request::ChannelPtyChangeSize {
                id,
                columns,
                rows,
                width,
                height,
                sender,
            } => {
                let res = self
                    .channel_pty_change_size(id, columns, rows, width, height)
                    .await;

                let _ = sender.send(res);
            }
            Request::UserauthKeyboardInteractive {
                username,
                submethods,
                mut cb,
                sender,
            } => {
                let res = self
                    .userauth_keyboard_interactive(&username, &submethods, &mut cb)
                    .await;
                let _ = sender.send(res);
            }
            Request::X11Forward {
                id,
                single_connection,
                protocol,
                cookie,
                screen_number,
                sender,
            } => {
                let res = self
                    .channel_x11_forward(id, single_connection, &protocol, &cookie, screen_number)
                    .await;
                let _ = sender.send(res);
            }
            Request::XonXoff { id, allow, sender } => {
                let res = self.channel_xon_xoff(id, allow).await;

                let _ = sender.send(res);
            }
            Request::KeyReExchange(sender) => {
                let res = self.rexchange().await;

                let _ = sender.send(res);
            }
        }
        false
    }

    async fn rexchange(&mut self) -> Result<()> {
        let meex = handshake::method_exchange(&mut self.stream, &self.config).await?;

        self.stream.server.kex_strict = meex.server.methods.kex_strict;
        self.stream.server.ext = meex.server.methods.ext;

        let mut algo =
            handshake::match_method(&meex.client.methods, &meex.server.methods, &self.config)?;

        let dhconfig = Dependency::new(
            self.client.banner.clone(),
            meex.client.binary,
            self.server.banner.clone(),
            meex.server.binary,
            self.stream.client.kex_strict && self.stream.server.kex_strict,
        );
        let mut result = algo.kex.kex(dhconfig, &mut self.stream).await?;
        // result.session_id = self.session_id.clone();
        // result.client_hash = self.session_id.clone();

        algo.hostkey.initialize(&result.server_hostkey)?;

        let res = algo
            .hostkey
            .verify(&result.server_signature, &result.client_hash)?;

        // if !res {
        //     return Err(Error::HostKeyVerifyFailed);
        // }
        snafu::ensure!(res, builder::HostKeyVerifyFailed);

        if let Some(behavior) = self.behaviour() {
            if !behavior
                .verify_server_hostkey(algo.hostkey.name(), &result.server_hostkey)
                .await?
            {
                // return Err(Error::RejectByUser(
                //     "Server public key rejected by user".to_string(),
                // ));
                return builder::RejectByUser {
                    tip: "Server public key rejected by user",
                }
                .fail();
            }
        }

        handshake::new_keys(&mut self.stream).await?;

        result.session_id.clone_from(&self.session_id);

        algo.initialize(&mut result)?;

        self.stream.decrypt = algo.server_crypt;
        self.stream.encrypt = algo.client_crypt;
        self.stream.server.mac = algo.server_mac;
        self.stream.client.mac = algo.client_mac;
        self.stream.decode = algo.server_compress;
        self.stream.encode = algo.client_compress;

        Ok(())
    }

    async fn channel_xon_xoff(&mut self, id: u32, allow: bool) -> Result<()> {
        let recipient = self.get_server_channel_id(id)?;

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: recipient,
            one: "xon-xoff",
            u8: 0,
            u8: allow as u8,
        };

        self.stream.send_payload(buffer).await
    }

    async fn channel_x11_forward(
        &mut self,
        id: u32,
        single_connection: bool,
        protocol: &str,
        cookie: &str,
        screen_number: u32,
    ) -> Result<()> {
        let recipient = self.get_server_channel_id(id)?;

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: recipient,
            one: "x11-req",
            u8: 1,
            u8: single_connection as u8,
            one: protocol,
            one: cookie,
            u32: screen_number
        };

        self.stream.send_payload(buffer).await?;

        channel_loop!(
            self,
            id,
            Message::ChannelSuccess(recipient) if recipient == id => return Ok(()),
            Message::ChannelFailure(recipient) if recipient == id => {
                // return Err(Error::ChannelFailure);
                return builder::ChannelFailure.fail();
            },
        );
    }

    async fn userauth_keyboard_interactive(
        &mut self,
        username: &str,
        submethods: &[String],
        cb: &mut Box<dyn Interactive>,
    ) -> Result<bool> {
        let submethods = submethods.join(",");

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_USERAUTH_REQUEST,
            one: username,
            one: "ssh-connection"
            one: "keyboard-interactive",
            u32: 0,
            one: submethods
        };

        self.stream.send_payload(buffer).await?;

        #[allow(clippy::type_complexity)]
        fn parse(payload: &[u8]) -> Option<(&[u8], &[u8], Vec<(&[u8], bool)>)> {
            let payload = Buffer::from_slice(payload);

            let name = payload.take_one()?.1;
            let instruction = payload.take_one()?.1;

            let _ = payload.take_one()?.1; // tag

            let num_prompts = payload.take_u32()?;

            let mut prompts = vec![];

            for _ in 0..num_prompts {
                let prompt = payload.take_one()?.1;
                let echo = payload.take_u8()? != 0;

                prompts.push((prompt, echo));
            }

            Some((name, instruction, prompts))
        }

        let mut buffer = Buffer::with_capacity(128);
        loop {
            let packet = self.stream.recv_packet().await?;
            if packet.payload.is_empty() {
                return Err(Error::invalid_format("Invalid: empty payload"));
            }
            match packet.payload[0] {
                SSH_MSG_USERAUTH_SUCCESS => {
                    return Ok(true);
                }

                SSH_MSG_USERAUTH_FAILURE => {
                    return Ok(false);
                }
                SSH_MSG_USERAUTH_INFO_REQUEST => {
                    let Some((name, instruction, prompts)) = parse(&packet.payload[1..]) else {
                        return Err(Error::invalid_format(
                            "Invalid keyboard interactive request from server",
                        ));
                    };
                    if prompts.is_empty() {
                        buffer.put_u8(SSH_MSG_USERAUTH_INFO_RESPONSE);
                        buffer.put_u32(0);
                        self.stream.send_payload(&buffer).await?;
                        buffer.clear();
                    } else {
                        let mut utf8_prompts = vec![];

                        for (prompts, echo) in prompts {
                            utf8_prompts.push((std::str::from_utf8(prompts)?, echo));
                        }

                        let response = cb
                            .response(
                                std::str::from_utf8(name)?,
                                std::str::from_utf8(instruction)?,
                                &utf8_prompts,
                            )
                            .await?;

                        let num_res = response.len() as u32;

                        buffer.put_u8(SSH_MSG_USERAUTH_INFO_RESPONSE);
                        buffer.put_u32(num_res);

                        for i in response {
                            buffer.put_one(i);
                        }

                        self.stream.send_payload(&buffer).await?;
                        buffer.clear();
                    }
                }
                _ => {
                    let msg = Message::parse(&packet.payload).map_err(Error::invalid_format)?;
                    self.handle_msg(msg).await?;
                }
            }
        }
    }

    async fn channel_pty_change_size(
        &mut self,
        id: u32,
        columns: u32,
        rows: u32,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let server_id = self.get_server_channel_id(id)?;

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: server_id,
            one: "window-change",
            u8: 0,
            u32: columns,
            u32: rows,
            u32: width,
            u32: height
        };

        self.stream.send_payload(buffer).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn channel_request_pty(
        &mut self,
        id: u32,
        term: &str,
        columns: u32,
        rows: u32,
        width: u32,
        height: u32,
        terminal_modes: &[(TerminalMode, u32)],
    ) -> Result<()> {
        let server_id = self.get_server_channel_id(id)?;

        let mut modes = Buffer::with_capacity(terminal_modes.len() * 5 + 1 + 4);

        modes.put_u32((terminal_modes.len() * 5 + 1) as u32);

        for (md, arg) in terminal_modes {
            modes.put_u8(*md as u8);
            modes.put_u32(*arg);
        }

        modes.put_u8(0);

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: server_id,
            one: "pty-req",
            u8: 1,
            one: term,
            u32: columns,
            u32: rows,
            u32: width,
            u32: height,
            bytes: modes
        };

        self.stream.send_payload(buffer).await?;

        channel_loop!(
            self,
            id,
            Message::ChannelSuccess(recipient) if recipient == id => return Ok(()),
            Message::ChannelFailure(recipient) if recipient == id => {
                // return Err(Error::ChannelFailure);
                return builder::ChannelFailure.fail();
            },
        );
    }

    async fn direct_tcpip(
        &mut self,
        initial: u32,
        maximum: u32,
        remote: SocketAddr,
        local: SocketAddr,
    ) -> Result<Channel> {
        let session = self.upgrade_sender()?;
        // let mut buffer = Buffer::new();

        let client_id = self.genarate_channel_id();
        // buffer.put_u8(SSH_MSG_CHANNEL_OPEN);
        // buffer.put_one("direct-tcpip");
        // buffer.put_u32(client_id);
        // buffer.put_u32(initial);
        // buffer.put_u32(maximum);
        // buffer.put_one(remote.0);
        // buffer.put_u32(remote.1);
        // buffer.put_one(local.0);
        // buffer.put_u32(local.1);

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_OPEN,
            one: "direct-tcpip",
            u32: client_id,
            u32: initial,
            u32: maximum,
            one: remote.host,
            u32: remote.port,
            one: local.host,
            u32: local.port
        };

        self.stream.send_payload(buffer).await?;

        loop {
            let msg = self.recv_msg().await?;
            return match msg {
                Message::ChannelOpenFailure {
                    recipient,
                    reson,
                    desc,
                    ..
                } if recipient == client_id => builder::ChannelOpenFail { reson, desc }.fail(),
                Message::ChannelOpenConfirmation {
                    recipient,
                    sender,
                    initial: server_initial,
                    maximum: server_maximum,
                } if recipient == client_id => {
                    let client = ChannelEndpoint::new(client_id, initial, maximum);
                    let server = ChannelEndpoint::new(sender, server_initial, server_maximum);
                    let (sender, recver) = m_channel();

                    let channel = Channel::new(client.id, recver, session);

                    let inner = ChannelInner::new(
                        client, server, // Box::new(NormalChannel::new(stdout.0, stderr.0)),
                        sender,
                    );

                    self.channels.insert(client_id, inner);
                    return Ok(channel);
                }

                msg => {
                    self.handle_msg(msg).await?;
                    continue;
                }
            };
        }
    }

    async fn cancel_tcpip_forward(&mut self, address: &SocketAddr) -> Result<()> {
        // let mut buffer = Buffer::new();
        // buffer.put_u8(SSH_MSG_GLOBAL_REQUEST);
        // buffer.put_one("cancel-tcpip-forward");
        // buffer.put_u8(1);
        // buffer.put_one(address);
        // buffer.put_u32(port);

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_GLOBAL_REQUEST,
            one: "cancel-tcpip-forward",
            u8: 1,
            one: &address.host,
            u32: address.port,
        };

        self.stream.send_payload(buffer).await?;

        loop {
            let packet = self.stream.recv_packet().await?;
            let payload = Buffer::from_slice(&packet.payload);

            match payload.take_u8() {
                Some(SSH_MSG_REQUEST_SUCCESS) => {
                    self.listeners.remove(address);
                    return Ok(());
                }
                Some(SSH_MSG_REQUEST_FAILURE) => {
                    // return Err(Error::RequestFailure(
                    //     "Failed to cancel tcpip forward".to_string(),
                    // ));
                    return builder::RequestFailure {
                        tip: "Failed to cancel tcpip forward",
                    }
                    .fail();
                }
                None => return Err(Error::invalid_format("Invalid code")),
                _ => {
                    let msg = Message::parse(&packet.payload).map_err(Error::invalid_format)?;

                    self.handle_msg(msg).await?;
                }
            }
        }
    }

    async fn tcpip_forward(
        &mut self,
        mut address: SocketAddr,
        // mut port: u32,
        initial: u32,
        maximum: u32,
    ) -> Result<Listener> {
        let session = self.upgrade_sender()?;
        // let mut buffer = Buffer::new();
        // buffer.put_u8(SSH_MSG_GLOBAL_REQUEST);
        // buffer.put_one("tcpip-forward");
        // buffer.put_u8(1);
        // buffer.put_one(address);
        // buffer.put_u32(port);

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_GLOBAL_REQUEST,
            one: "tcpip-forward",
            u8: 1,
            one: &address.host,
            u32: address.port,
        };

        self.stream.send_payload(buffer).await?;

        loop {
            let packet = self.stream.recv_packet().await?;
            let payload = Buffer::from_slice(&packet.payload);

            match payload.take_u8() {
                Some(SSH_MSG_REQUEST_SUCCESS) => {
                    if address.port == 0 {
                        address.port = payload
                            .take_u32()
                            .ok_or(Error::invalid_format("Invalid port"))?;
                    }
                    let (sender, recver) = m_channel();

                    self.listeners.insert(
                        address.clone(),
                        ListenerInner::new(sender, initial, maximum),
                    );

                    return Ok(Listener::new(session, recver, address));
                }
                Some(SSH_MSG_REQUEST_FAILURE) => {
                    // return Err(Error::RequestFailure("Failed to tcpip forward".to_string()));
                    return builder::RequestFailure {
                        tip: "Failed to tcpip forward",
                    }
                    .fail();
                }
                None => return Err(Error::invalid_format("Invalid code")),
                _ => {
                    let msg = Message::parse(&packet.payload).map_err(Error::invalid_format)?;

                    self.handle_msg(msg).await?;
                }
            }
        }
    }

    async fn channel_request_shell(&mut self, id: u32) -> Result<()> {
        // let mut buffer = Buffer::new();

        // buffer.put_u8(SSH_MSG_CHANNEL_REQUEST);
        // buffer.put_u32(id);
        // buffer.put_one("shell");
        // buffer.put_u8(1);

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: id,
            one: "shell",
            u8: 1,
        };

        self.stream.send_payload(buffer).await?;

        channel_loop!(
            self,
            id,
            Message::ChannelSuccess(recipient) if recipient == id => return Ok(()),
            Message::ChannelFailure(recipient) if recipient == id => {
                // return Err(Error::ChannelFailure);
                return builder::ChannelFailure.fail();
            },
        );
    }

    async fn channel_send_signal(&mut self, id: u32, signal: &str) -> Result<()> {
        // let mut buffer = Buffer::new();

        let server_id = self.get_server_channel_id(id)?;
        // buffer.put_u32(server_id);
        // buffer.put_one("signal");
        // buffer.put_u8(0);
        // buffer.put_one(signal);

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: server_id,
            one: "signal",
            u8: 0,
            one: signal
        };

        self.stream.send_payload(buffer).await
    }

    async fn channel_set_env(&mut self, id: u32, name: &str, value: &[u8]) -> Result<()> {
        let server_id = self.get_server_channel_id(id)?;
        // let mut buffer = Buffer::new();

        // buffer.put_u32(server_id);

        // buffer.put_one("env");
        // buffer.put_u8(1);
        // buffer.put_one(name);
        // buffer.put_one(value);

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: server_id,
            one: "env",
            u8: 1,
            one: name,
            one: value,
        };

        self.stream.send_payload(buffer).await?;

        channel_loop!(
            self,
            id,
            Message::ChannelSuccess(recipient) if recipient == id => return Ok(()),
            Message::ChannelFailure(recipient) if recipient == id => {
                // return Err(Error::ChannelFailure);
                return builder::ChannelFailure.fail();
            },
        );
    }

    async fn channel_write_stdout(&mut self, id: u32, data: &[u8]) -> Result<usize> {
        // let (channel, stream) = func(self)?;
        let channel = self
            .channels
            .get_mut(&id)
            .ok_or(Error::ub("Failed to find channel"))?;

        if channel.client.closed || channel.server.closed {
            // return Err(Error::ChannelClosed);
            return builder::ChannelClosed.fail();
        }

        if channel.client.eof {
            // return Err(Error::ChannelEof);
            return builder::ChannelEof.fail();
        }

        // channel.stdout_buf.extend(data);

        // if data.is_empty() {
        //     return Ok(0);
        // } else if channel.server.size == 0 {
        //     return Ok(0);
        // }
        let total = data.len();
        let mut pos = 0;

        let mut buffer = Buffer::with_capacity(1024);
        while channel.server.size > 0 && pos < total {
            buffer.put_u8(SSH_MSG_CHANNEL_DATA);

            buffer.put_u32(channel.server.id);

            let len = min(channel.server.maximum, channel.server.size) as usize;
            let len = min(len, PAYLOAD_MAXIMUM_SIZE - 100);
            let len = min(total - pos, len);

            buffer.put_one(&data[pos..pos + len]);
            self.stream.send_payload(buffer.as_ref()).await?;
            channel.server.size -= len as u32;
            pos += len;
            buffer.clear();
        }

        Ok(pos)
    }

    async fn channel_drop(&mut self, id: u32) -> Result<()> {
        let mut channel = self.remove_channel(id)?;
        // let channel = self.channels.get_mut(&id).ok_or(Error::ChannelNotFound)?;

        if !channel.client.closed {
            if !channel.client.eof {
                // if !channel.server.closed {
                //     self.channel_flush_stdout_blocking(id).await?;
                // }
                // let mut buffer = Buffer::new();
                // buffer.put_u8(SSH_MSG_CHANNEL_EOF);
                // buffer.put_u32(channel.server.id);

                let buffer = make_buffer_without_header! {
                    u8: SSH_MSG_CHANNEL_EOF,
                    u32: channel.server.id
                };

                self.stream.send_payload(buffer).await?;
                channel.client.eof = true;
            }

            // let mut buffer = Buffer::new();
            // buffer.put_u8(SSH_MSG_CHANNEL_CLOSE);
            // buffer.put_u32(channel.server.id);

            let buffer = make_buffer_without_header! {
                u8: SSH_MSG_CHANNEL_CLOSE,
                u32: channel.server.id
            };

            self.stream.send_payload(buffer).await?;
            channel.client.closed = true;
        }

        if !channel.server.closed {
            // do not remove, because we need to wait for channel close msg from server
            self.channels.insert(id, channel);
        }

        Ok(())
    }

    async fn channel_eof(&mut self, id: u32) -> Result<()> {
        let channel = self
            .channels
            .get_mut(&id)
            .ok_or(Error::ub("Failed to find channel"))?;

        if channel.client.closed {
            return Ok(());
        }
        if channel.client.eof {
            return Ok(());
        }

        // let mut buffer = Buffer::new();
        // buffer.put_u8(SSH_MSG_CHANNEL_EOF);
        // buffer.put_u32(channel.server.id);

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_EOF,
            u32: channel.server.id,
        };

        // if !channel.server.closed {
        //     self.channel_flush_stdout_blocking(id).await?;
        // }

        self.stream.send_payload(buffer).await?;

        channel.client.eof = true;
        Ok(())
    }

    async fn handle_channel_close(&mut self, id: u32) -> Result<()> {
        let mut channel = self.remove_channel(id)?;
        if channel.server.closed {
            // return Err(Error::ProtocolError(
            //     "The channel is already closed".to_string(),
            // ));
            return builder::Protocol {
                tip: "The channel is already closed",
            }
            .fail();
        }

        channel.server_close();
        // std::mem::take(&mut channel.stdout_buf);

        if !channel.client.closed {
            if !channel.client.eof {
                // let mut buffer = Buffer::new();
                // buffer.put_u8(SSH_MSG_CHANNEL_EOF);
                // buffer.put_u32(channel.server.id);
                let buffer = make_buffer_without_header! {
                    u8: SSH_MSG_CHANNEL_EOF,
                    u32: channel.server.id,
                };
                self.stream.send_payload(buffer).await?;
                channel.client.eof = true;
            }
            // let mut buffer = Buffer::new();
            // buffer.put_u8(SSH_MSG_CHANNEL_CLOSE);
            // buffer.put_u32(channel.server.id);
            let buffer = make_buffer_without_header! {
                u8: SSH_MSG_CHANNEL_CLOSE,
                u32: channel.server.id
            };

            self.stream.send_payload(buffer).await?;

            channel.client.closed = true;
            // do not remove, because we need to wait for the channel to be dropped or called close() by user
            self.channels.insert(id, channel);
        }
        Ok(())
    }

    fn remove_channel(&mut self, id: u32) -> Result<ChannelInner> {
        self.channels
            .remove(&id)
            .ok_or(Error::ub("Failed to find channel"))
    }

    fn get_server_channel_id(&mut self, id: u32) -> Result<u32> {
        self.channels
            .get(&id)
            .map(|v| v.server.id)
            .ok_or(Error::ub("Failed to find channel"))
    }

    fn get_server_channel(&mut self, id: u32) -> Result<&mut ChannelInner> {
        self.channels
            .get_mut(&id)
            .ok_or(Error::ub("Failed to find channel"))
    }

    async fn channel_exec(&mut self, id: u32, cmd: &str) -> Result<()> {
        let server_id = self.get_server_channel_id(id)?;
        // let mut buffer = Buffer::new();
        // buffer.put_u8(SSH_MSG_CHANNEL_REQUEST);
        // buffer.put_u32(self.get_server_channel_id(id)?);
        // buffer.put_one(b"exec");
        // buffer.put_u8(1);
        // buffer.put_one(cmd.as_bytes());
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_REQUEST,
            u32: server_id,
            one: "exec",
            u8: 1,
            one: cmd,
        };

        self.stream.send_payload(buffer).await?;

        channel_loop!(
            self,
            id,
            Message::ChannelSuccess(recipient) if recipient == id => return Ok(()),
            Message::ChannelFailure(recipient) if recipient == id => {
                // return Err(Error::ChannelFailure);
                return builder::ChannelFailure.fail();
            },
        );
    }

    async fn channel_open_raw(
        &mut self,
        initial: u32,
        maximum: u32,
    ) -> Result<(ChannelEndpoint, ChannelEndpoint)> {
        let client_id = self.genarate_channel_id();
        // let mut buffer = Buffer::new();
        // buffer.put_u8(SSH_MSG_CHANNEL_OPEN);
        // buffer.put_one(b"session");
        // buffer.put_u32(client_id);
        // buffer.put_u32(initial);
        // buffer.put_u32(maximum);
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_CHANNEL_OPEN,
            one: "session",
            u32: client_id,
            u32: initial,
            u32: maximum
        };

        self.stream.send_payload(buffer).await?;

        loop {
            let msg = self.recv_msg().await?;
            return match msg {
                Message::ChannelOpenFailure {
                    recipient,
                    reson,
                    desc,
                    ..
                } if recipient == client_id => builder::ChannelOpenFail { reson, desc }.fail(),
                Message::ChannelOpenConfirmation {
                    recipient,
                    sender,
                    initial: server_initial,
                    maximum: server_maximum,
                } if recipient == client_id => Ok((
                    ChannelEndpoint::new(client_id, initial, maximum),
                    ChannelEndpoint::new(sender, server_initial, server_maximum),
                )),

                _ => {
                    self.handle_msg(msg).await?;
                    continue;
                }
            };
        }
    }

    // async fn channel_open<B: SubSystem + Send + 'static>(
    //     &mut self,
    //     initial: u32,
    //     maximum: u32,
    //     subsystem: B,
    // ) -> Result<()> {
    //     let (client, server) = self.channel_open_raw(initial, maximum).await?;

    //     let inner = ChannelInner::new(client, server, Box::new(subsystem), None);

    //     self.channels.insert(inner.client.id, inner);

    //     Ok(())
    // }

    async fn channel_open_normal(
        &mut self,
        initial: u32,
        maximum: u32,
    ) -> Result<(Channel, ChannelInner)> {
        let session = self.upgrade_sender()?;
        let (client, server) = self.channel_open_raw(initial, maximum).await?;

        // let stdout = io_channel();
        // let stderr = io_channel();

        let (sender, recver) = m_channel();

        let channel = Channel::new(client.id, recver, session);

        let inner = ChannelInner::new(
            client, server, // Box::new(NormalChannel::new(stdout.0, stderr.0)),
            sender,
        );

        Ok((channel, inner))
    }

    async fn channel_open_session(&mut self, initial: u32, maximum: u32) -> Result<Channel> {
        let (channel, inner) = self.channel_open_normal(initial, maximum).await?;

        self.channels.insert(inner.client.id, inner);

        Ok(channel)
    }

    async fn userauth_none(&mut self, username: &str) -> Result<Userauth> {
        // let mut buffer = Buffer::new();
        // buffer.put_u8(SSH_MSG_USERAUTH_REQUEST);
        // buffer.put_one(username);
        // buffer.put_one(b"ssh-connection");
        // buffer.put_one(b"none");

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_USERAUTH_REQUEST,
            one: username,
            one: "ssh-connection",
            one: "none",
        };

        self.stream.send_payload(buffer.as_ref()).await?;

        loop {
            return match self.recv_msg().await? {
                Message::UserauthSuccess => {
                    self.stream.authed = true;
                    Ok(Userauth::Success)
                }
                Message::UserauthFailure { methods, partial } => {
                    Ok(Userauth::Failure(methods, partial))
                }
                Message::UserauthChangeReq => Ok(Userauth::Failure(vec![], false)),

                msg => {
                    self.handle_msg(msg).await?;
                    continue;
                }
            };
        }
    }

    async fn userauth_password(&mut self, username: &str, password: &str) -> Result<Userauth> {
        // let mut buffer = Buffer::new();
        // buffer.put_u8(SSH_MSG_USERAUTH_REQUEST);
        // buffer.put_one(username);
        // buffer.put_one(b"ssh-connection");
        // buffer.put_one(b"password");
        // buffer.put_u8(0);
        // buffer.put_one(password);

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_USERAUTH_REQUEST,
            one: username,
            one: "ssh-connection",
            one: "password",
            u8: 0,
            one: password,
        };

        self.stream.send_payload(buffer).await?;

        loop {
            return match self.recv_msg().await? {
                Message::UserauthSuccess => {
                    self.stream.authed = true;
                    Ok(Userauth::Success)
                }
                Message::UserauthFailure { methods, partial } => {
                    Ok(Userauth::Failure(methods, partial))
                }
                Message::UserauthChangeReq => Ok(Userauth::Expired),

                msg => {
                    self.handle_msg(msg).await?;
                    continue;
                }
            };
        }
    }

    async fn userauth_publickey(
        &mut self,
        username: &str,
        method: &str,
        publickey: &[u8],
        privatekey: &[u8],
    ) -> Result<Userauth> {
        // let mut buffer = Buffer::new();
        // buffer.put_u8(SSH_MSG_USERAUTH_REQUEST);
        // buffer.put_one(username);
        // buffer.put_one(b"ssh-connection");
        // buffer.put_one("publickey");
        // buffer.put_u8(0);
        // buffer.put_one(method);
        // buffer.put_one(publickey);

        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_USERAUTH_REQUEST,
            one: username,
            one: b"ssh-connection",
            one: b"publickey",
            u8: 0,
            one: method,
            one: publickey,
        };

        self.stream.send_payload(buffer).await?;

        loop {
            let packet = self.stream.recv_packet().await?;
            let payload = Buffer::from_slice(&packet.payload);
            let code = payload
                .take_u8()
                .ok_or(Error::invalid_format("Invalid ssh packet"))?;
            match code {
                SSH_MSG_USERAUTH_FAILURE => {
                    let (_, methods) = payload
                        .take_one()
                        .ok_or(Error::invalid_format("Invalid ssh packet"))?;
                    return Ok(Userauth::Failure(
                        std::str::from_utf8(methods)?
                            .split(',')
                            .map(|v| v.to_string())
                            .collect(),
                        payload
                            .take_u8()
                            .ok_or(Error::invalid_format("Invalid ssh packet"))?
                            != 0,
                    ));
                }
                SSH_MSG_USERAUTH_SUCCESS => return Ok(Userauth::Success),
                SSH_MSG_USERAUTH_PK_OK => break,
                _ => {
                    self.handle_msg(
                        Message::parse(&packet.payload).map_err(Error::invalid_format)?,
                    )
                    .await?;
                }
            }
        }

        // let mut buffer = Buffer::new();
        // buffer.put_one(&self.session_id);
        // buffer.put_u8(SSH_MSG_USERAUTH_REQUEST);
        // buffer.put_one(username);
        // buffer.put_one(b"ssh-connection");
        // buffer.put_one("publickey");
        // buffer.put_u8(1);
        // buffer.put_one(method);
        // buffer.put_one(publickey);

        let buffer = make_buffer_without_header! {
            one: &self.session_id,
            u8: SSH_MSG_USERAUTH_REQUEST,
            one: username,
            one: b"ssh-connection",
            one: b"publickey",
            u8: 1,
            one: method,
            one: publickey,
        };

        let mut algo = sign::new_signature_by_name(method)
            .ok_or(Error::ub("Unable to create cipher"))?
            .create();

        algo.initialize(privatekey.as_ref())?;
        let sign = algo.signature(buffer.as_ref())?;

        // let mut buffer = Buffer::new();
        // buffer.put_u8(SSH_MSG_USERAUTH_REQUEST);
        // buffer.put_one(username);
        // buffer.put_one(b"ssh-connection");
        // buffer.put_one("publickey");
        // buffer.put_u8(1);
        // buffer.put_one(method);
        // // buffer.put_u32((4 + method.len() + 4 + publickey.len()) as u32);
        // // buffer.put_one(method);
        // buffer.put_one(publickey);

        // buffer.put_u32((4 + method.len() + 4 + sign.len()) as _);

        // buffer.put_one(method);
        // buffer.put_one(sign);

        let len = 4 + method.len() + 4 + sign.len();
        let len = len as u32;
        let buffer = make_buffer_without_header! {
            u8: SSH_MSG_USERAUTH_REQUEST,
            one: username,
            one: b"ssh-connection",
            one: b"publickey",
            u8: 1,
            one: method,
            one: publickey,
            u32: len,
            one: method,
            one: &sign,
        };
        self.stream.send_payload(buffer).await?;

        loop {
            match self.recv_msg().await? {
                Message::UserauthSuccess => {
                    self.stream.authed = true;
                    break Ok(Userauth::Success);
                }
                Message::UserauthFailure { methods, partial } => {
                    break Ok(Userauth::Failure(methods, partial));
                }
                msg => {
                    self.handle_msg(msg).await?;
                }
            }
        }
    }

    async fn sftp_from_channel(&mut self, channel: Channel, inner: ChannelInner) -> Result<SFtp> {
        let server_id: u32 = inner.server.id;
        let client_id = inner.client.id;

        // self.channels.insert(channel.id, inner);

        // let (sender, recver) = async_channel::unbounded();

        // let sub = SFtpSystem::new(sender);

        // let inner = ChannelInner::new(client, server, Box::new(sub), None);

        self.channels.insert(client_id, inner);

        let mut buffer = Buffer::new();

        buffer.put_u8(SSH_MSG_CHANNEL_REQUEST);
        buffer.put_u32(server_id);
        buffer.put_one(b"subsystem");
        buffer.put_u8(1);
        buffer.put_one(b"sftp");

        self.stream.send_payload(buffer.as_ref()).await?;

        // loop {
        //     match self.recv_msg().await? {
        //         ServerMessage::ChannelSuccess(_) => break,
        //         ServerMessage::ChannelFailure(_) => return Err(Error::SubsystemFailed),
        //         msg => {
        //             self.handle_msg(msg).await?;
        //         }
        //     }
        // }

        channel_loop!(
            self,
            client_id,
            Message::ChannelSuccess(recipient) if recipient == client_id => break,
            Message::ChannelFailure(recipient) if recipient == client_id => return builder::SubsystemFailed.fail(), //Err(Error::SubsystemFailed),
        );

        let mut buffer = Buffer::new();
        // buffer.put_u8(SSH_MSG_CHANNEL_DATA);
        // buffer.put_u32(server_id);
        // buffer.put_u32(4 + 1 + 4);
        buffer.put_u32(5);
        buffer.put_u8(SSH_FXP_INIT);
        buffer.put_u32(SFTP_VERSION);

        // self.stream.send_payload(buffer.as_ref()).await?;

        self.channel_write_stdout(client_id, buffer.as_ref())
            .await?;

        loop {
            let msg = self.recv_msg().await?;
            match msg {
                Message::ChannelClose(recipient) if recipient == client_id => {
                    // break Err(Error::ChannelClosed);
                    break builder::ChannelClosed.fail();
                }
                Message::ChannelEof(recipient) if recipient == client_id => {
                    // break Err(Error::ChannelEof);
                    break builder::ChannelEof.fail();
                }
                Message::ChannelStdoutData { recipient, data } if recipient == client_id => {
                    let data = Buffer::from_slice(&data);
                    let (_, data) = data
                        .take_one()
                        .ok_or(Error::invalid_format("Invalid ssh packet"))?;

                    let data = Buffer::from_slice(data);

                    let value = data
                        .take_u8()
                        .ok_or(Error::invalid_format("Invalid ssh packet"))?;

                    if value != SSH_FXP_VERSION {
                        // return Err(Error::ProtocolError(
                        //     "Unable to receive SFtp version".to_string(),
                        // ));
                        return builder::Protocol {
                            tip: "Unable to receive SFtp version",
                        }
                        .fail();
                    }

                    let version = data
                        .take_u32()
                        .ok_or(Error::invalid_format("Invalid ssh packet"))?;

                    let ext = || {
                        let Some((_, key)) = data.take_one() else {
                            return Ok(None);
                        };

                        let key = std::str::from_utf8(key)?;

                        let (_, value) = data
                            .take_one()
                            .ok_or(Error::invalid_format("Failed to parse value"))?;

                        Result::Ok(Some((key.to_string(), value.to_vec())))
                    };

                    let mut extension = HashMap::new();
                    while let Some((k, v)) = ext()? {
                        extension.insert(k, v);
                    }

                    break Ok(SFtp::new(channel, version, extension));
                }
                msg => {
                    self.handle_msg(msg).await?;
                    continue;
                }
            }
        }
    }

    async fn sftp_open(&mut self, initial: u32, maximum: u32) -> Result<SFtp> {
        let (channel, inner) = self.channel_open_normal(initial, maximum).await?;

        self.sftp_from_channel(channel, inner).await
    }
}
