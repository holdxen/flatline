use tokio::sync::oneshot;

use crate::{
    error,
    session::channel::{Channel, IdentityPair, TtyOpcode},
};
use crate::session::{forward, InteractiveMethod, KeyboardInteractive};

pub(super) enum Event {
    AuthenticateNone {
        username: String,
        back: oneshot::Sender<error::Result<super::AuthenticateResult>>,
    },
    AuthenticatePassword {
        username: String,
        password: String,
        back: oneshot::Sender<error::Result<super::AuthenticateResult>>,
    },
    AuthenticatePublicKey {
        username: String,
        method: String,
        is_certificate: bool,
        public_blob: Vec<u8>,
        private_blob: Vec<u8>,
        back: oneshot::Sender<error::Result<super::AuthenticateResult>>,
    },
    AuthenticateKeyboardInteractive {
        username: String,
        interactive: Box<dyn KeyboardInteractive>,
        methods: Vec<InteractiveMethod>,
        back: oneshot::Sender<error::Result<super::AuthenticateResult>>,
    },
    RequestAuthentication {
        back: oneshot::Sender<error::Result<()>>,
    },
    SendDebugMessage {
        always_display: bool,
        message: String,
        back: oneshot::Sender<error::Result<()>>,
    },
    ChannelRequestPty {
        channel_id: IdentityPair,
        terminal: String,
        want_reply: bool,
        columns: u32,
        rows: u32,
        width: u32,
        height: u32,
        modes: Vec<(TtyOpcode, u32)>,
        back: oneshot::Sender<error::Result<()>>,
    },
    ChannelRequestBreak {
        channel_id: IdentityPair,
        want_reply: bool,
        milliseconds: u32,
        back: oneshot::Sender<error::Result<()>>,
    },
    ChannelRequestShell {
        channel_id: IdentityPair,
        want_reply: bool,
        back: oneshot::Sender<error::Result<()>>,
    },
    ChannelRequestExec {
        channel_id: IdentityPair,
        want_reply: bool,
        command: String,
        back: oneshot::Sender<error::Result<()>>,
    },
    ChannelRequestAgent {
        channel_id: IdentityPair,
        want_reply: bool,
        back: oneshot::Sender<error::Result<()>>,
    },
    ChannelRequestWindowChange {
        channel_id: IdentityPair,
        columns: u32,
        rows: u32,
        width: u32,
        height: u32,
        back: oneshot::Sender<error::Result<()>>,
    },
    ChannelRequestSignal {
        channel_id: IdentityPair,
        want_reply: bool,
        signal: String,
        back: oneshot::Sender<error::Result<()>>
    },
    ChannelRequestEnv {
        channel_id: IdentityPair,
        want_reply: bool,
        name: String,
        value: String,
        back: oneshot::Sender<error::Result<()>>
    },
    ChannelOpenSFTP {
        initial_window_size: u32,
        maximum_packet_size: u32,
        back: oneshot::Sender<error::Result<Channel>>,
    },
    ChannelOpenSession {
        initial_window_size: u32,
        maximum_packet_size: u32,
        back: oneshot::Sender<error::Result<Channel>>,
    },
    ChannelOpenDirectTcpIp {
        target: forward::SocketAddr,
        source: forward::SocketAddr,
        initial_window_size: u32,
        maximum_packet_size: u32,
        back: oneshot::Sender<error::Result<Channel>>,
    },
    ChannelRequestX11 {
        channel_id: IdentityPair,
        want_reply: bool,
        single_connection: bool,
        protocol: String,
        cookie: String,
        screen: u32,
        back: oneshot::Sender<error::Result<()>>
    },
    ChannelSendData {
        channel_id: IdentityPair,
        data: Vec<u8>,
        back: oneshot::Sender<error::Result<usize>>,
    },
    ChannelClose {
        channel_id: IdentityPair,
        back: oneshot::Sender<error::Result<()>>,
    },
    ChannelEof {
        channel_id: IdentityPair,
        back: oneshot::Sender<error::Result<()>>,
    },
    ChannelClean {
        back: oneshot::Sender<error::Result<()>>,
    },
    GlobalRequestTcpIPForward {
        addr: forward::SocketAddr,
        initial_window_size: u32,
        maximum_packet_size: u32,
        back: oneshot::Sender<error::Result<forward::Listener>>
    },
    GlobalRequestCancelTcpIpForward {
        want_reply: bool,
        addr: forward::SocketAddr,
        back: oneshot::Sender<error::Result<()>>
    }
}
