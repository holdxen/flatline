use super::Event;
use super::{UnexpectedBehaviourSnafu, UnexpectedReceivingError, UnexpectedSendingError, channel};
use crate::error;
use crate::session::channel::Channel;
use channel::Message as ChannelMessage;
use snafu::OptionExt;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};

pub const ALL: &str = "";
pub const IPV4_ALL: &str = "0.0.0.0";
pub const IPV6_ALL: &str = "::";
pub const LOCALHOST: &str = "localhost";
pub const IPV4_LOCALHOST: &str = "127.0.0.1";
pub const IPV6_LOCALHOST: &str = "::1";

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct SocketAddr {
    pub host: String,
    pub port: u16,
}

impl SocketAddr {
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }
}

pub enum Message {
    Close,
    Eof,
    Bytes(Vec<u8>),
}

#[derive(derive_more::Debug)]
pub struct Listener<A: 'static, B> {
    #[debug(skip)]
    receiver: mpsc::Receiver<(Stream, B)>,
    #[debug(skip)]
    sender: mpsc::Sender<Event>,
    addr: A,
    cancelled: bool,
}

impl<A: 'static, B> Drop for Listener<A, B> {
    fn drop(&mut self) {
        if self.cancelled {
            return;
        }

        do_drop(&self.addr, &self.sender.clone());
    }
}

impl<A: 'static, B> Listener<A, B> {
    pub(super) fn new(
        receiver: mpsc::Receiver<(Stream, B)>,
        session: mpsc::Sender<Event>,
        addr: A,
    ) -> Self {
        Self {
            receiver,
            sender: session,
            addr,
            cancelled: false,
        }
    }

    pub fn addr(&self) -> &A {
        &self.addr
    }

    // pub async fn cancel(mut self, want_reply: bool) -> error::Result<()> {
    //     let (sender, receiver) = oneshot::channel();
    //     let event = Event::GlobalRequestCancelTcpIpForward {
    //         want_reply,
    //         addr: self.addr.clone(),
    //         back: sender,
    //     };
    //     self.sender.send_next(event).await?;

    //     self.cancelled = true;

    //     receiver.receive_next().await?
    // }
}

fn do_drop<'a, 'b>(value: &'a (dyn std::any::Any + 'b), session: &mpsc::Sender<Event>) {
    if let Some(value) = value.downcast_ref::<SocketAddr>() {
        let (sender, mut receiver) = oneshot::channel();
        let event = Event::GlobalRequestCancelTcpIpForward {
            want_reply: false,
            addr: value.clone(),
            back: sender,
        };

        if let Err(err) = session.try_send(event) {
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
    } else if let Some(value) = value.downcast_ref::<String>() {
        let (sender, mut receiver) = oneshot::channel();
        let event = Event::GlobalRequestCancelStreamLocalForward {
            want_reply: false,
            path: value.clone(),
            back: sender,
        };

        if let Err(err) = session.try_send(event) {
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
    } else {
        tracing::error!("Unknown value: {:?}", value);
    }
}

impl Listener<String, ()> {
    pub async fn cancel(mut self, want_reply: bool) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::GlobalRequestCancelStreamLocalForward {
            want_reply,
            path: self.addr.clone(),
            back: sender,
        };
        self.sender.send_next(event).await?;

        self.cancelled = true;

        receiver.receive_next().await?
    }

    pub async fn accept(&mut self) -> error::Result<Stream> {
        let stream = self
            .receiver
            .recv()
            .await
            .context(UnexpectedBehaviourSnafu {
                detail: "Maybe session is shutdown",
            })?;

        Ok(stream.0)
    }
}

impl Listener<SocketAddr, SocketAddr> {
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

    pub async fn accept(&mut self) -> error::Result<(Stream, SocketAddr)> {
        let stream = self
            .receiver
            .recv()
            .await
            .context(UnexpectedBehaviourSnafu {
                detail: "Maybe session is shutdown",
            })?;

        Ok(stream)
    }
}

#[derive(Debug)]
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
                ChannelMessage::Stdout(data) => break Ok(Message::Bytes(data)),
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
