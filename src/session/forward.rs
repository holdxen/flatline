use snafu::OptionExt;
use tokio::sync::{mpsc, oneshot};
use tokio::sync::mpsc::error::TrySendError;
use super::{channel, UnexpectedBehaviourSnafu, UnexpectedReceivingError, UnexpectedSendingError};
use super::Event;
use channel::Message as ChannelMessage;
use crate::error;
use crate::session::channel::Channel;

pub const ALL: &'static str = "";
pub const IPV4_ALL: &'static str = "0.0.0.0";
pub const IPV6_ALL: &'static str = "::";
pub const LOCALHOST: &'static str = "localhost";
pub const IPV4_LOCALHOST: &'static str = "127.0.0.1";
pub const IPV6_LOCALHOST: &'static str = "::1";


#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct SocketAddr {
    pub host: String,
    pub port: u16,
}

impl SocketAddr {
    pub fn new(host: String, port: u16) -> Self {
        Self {
            host,
            port,
        }
    }
}

pub enum Message {
    Close,
    Eof,
    Bytes(Vec<u8>),
}

pub struct Listener {
    receiver: mpsc::Receiver<(Stream, SocketAddr)>,
    sender: mpsc::Sender<Event>,
    addr: SocketAddr,
    cancelled: bool
}

impl Drop for Listener {
    fn drop(&mut self) {
        if self.cancelled {
            return;
        }

        let (sender, mut receiver) = oneshot::channel();
        let event = Event::GlobalRequestCancelTcpIpForward {
            want_reply: false,
            addr: self.addr.clone(),
            back: sender,
        };

        if let Err(err) =  self.sender.try_send(event) {
            match err {
                TrySendError::Full(_) => {
                    tracing::info!("Failed to cancel");
                }
                TrySendError::Closed(_) => {
                    tracing::info!("Maybe session is shutdown");
                }
            }
        }

        if let Err(err) = receiver.try_recv() {
            tracing::error!("Failed to cancel: {:?}", err);
        }
    }
}

impl Listener {

    pub(super) fn new(receiver: mpsc::Receiver<(Stream, SocketAddr)>, session: mpsc::Sender<Event>, addr: SocketAddr) -> Self {
        Self {
            receiver,
            sender: session,
            addr,
            cancelled: false,
        }
    }

    fn addr(&self) -> &SocketAddr {
        &self.addr
    }

    pub async fn accept(&mut self) -> error::Result<(Stream, SocketAddr)> {
         let stream = self.receiver.recv().await.context(UnexpectedBehaviourSnafu {
            detail: "Maybe session is shutdown"
        })?;

        Ok(stream)
    }

    pub async fn cancel(mut self, want_reply: bool) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::GlobalRequestCancelTcpIpForward {
            want_reply,
            addr: self.addr.clone(),
            back: sender,
        };
        self.sender.send_next(event).await?;

        self.cancelled = true;

        receiver.receive_next().await?
    }
}


pub struct Stream {
    channel: Channel,
}

impl Stream {
    pub(super) fn new(channel: Channel) -> Self {
        Self { channel }
    }

    pub async fn close(self) -> error::Result<()> {
        self.channel.close().await
    }

    pub async fn eof(&self) -> error::Result<()> {
        self.channel.eof().await
    }

    pub async fn receive(&mut self) -> error::Result<Message> {
        loop {
            match self.channel.receive().await? {
                ChannelMessage::Close => {
                    break Ok(Message::Close);
                }
                ChannelMessage::Eof => {
                    break Ok(Message::Eof);
                }
                ChannelMessage::Stdout(data) => {
                    break Ok(Message::Bytes(data))
                }
                ChannelMessage::Stderr(_) => {
                    tracing::warn!("Received unexpected stderr message from server");
                }
                ChannelMessage::Exit(status) => {
                    tracing::warn!("Received unexpected channel exit: {:?}", status);
                }
                ChannelMessage::FlowControl { on } => {
                    tracing::warn!("Unexpected flow control message: {:?}", on);
                }
                ChannelMessage::WindowChange { .. } => {}
            }
        }
    }

    pub async fn send(&self, data: Vec<u8>) -> error::Result<usize> {
        self.channel.send(data).await
    }
}