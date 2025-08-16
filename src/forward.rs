use std::fmt::Debug;

use derive_new::new;
use snafu::OptionExt;

use crate::{
    channel::{Channel, Message},
    error::builder,
    error::Result,
    msg::Request,
};

use super::{o_channel, MReceiver, MSender};

#[derive(Debug, Clone, PartialEq, Eq, Hash, new)]
pub struct SocketAddr {
    pub host: String,
    pub port: u32,
}

impl SocketAddr {
    pub const ALL: &'static str = "";
    pub const IPV4_ALL: &'static str = "0.0.0.0";
    pub const IPV6_ALL: &'static str = "::";
    pub const LOCALHOST: &'static str = "localhost";
    pub const IPV4_LOCALHOST: &'static str = "127.0.0.1";
    pub const IPV6_LOCALHOST: &'static str = "::1";
}

pub struct Stream {
    channel: Channel,
    address: SocketAddr,
}

impl Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream")
            .field("channel", &self.channel)
            .field("address", &self.address)
            .finish()
    }
}

impl Stream {
    pub async fn write(&self, data: impl Into<Vec<u8>>) -> Result<usize> {
        self.channel.write(data).await
    }

    pub async fn read(&mut self) -> Result<Vec<u8>> {
        let msg = self.channel.recv().await?;
        match msg {
            crate::channel::Message::Stdout(items) => Ok(items),
            crate::channel::Message::Stderr(_) => builder::BadMessage {
                tip: "unexpected message",
            }
            .fail(),
            _ => builder::ChannelClosed {}.fail(),
        }
    }

    pub async fn message(&mut self) -> Result<Message> {
        self.channel.recv().await
    }

    pub(crate) fn new(channel: Channel, address: SocketAddr) -> Self {
        Self { channel, address }
    }

    pub async fn close(self) -> Result<()> {
        self.channel.close().await
    }

    pub fn address(&self) -> &SocketAddr {
        &self.address
    }
}

pub struct Listener {
    session: Option<MSender<Request>>,
    recver: MReceiver<Stream>,
    address: SocketAddr,
    // port: u32,
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(ref mut session) = self.session {
            let _ = session.send(Request::CancelTcpipForward {
                address: self.address.clone(),
                // port: self.port,
                sender: None,
            });
        }
    }
}

impl Listener {
    pub(crate) fn new(
        session: MSender<Request>,
        recver: MReceiver<Stream>,
        address: SocketAddr,
        // port: u32,
    ) -> Self {
        Self {
            session: session.into(),
            recver,
            address,
            // port,
        }
    }

    pub async fn accpet(&mut self) -> Result<Stream> {
        self.recver.recv().await.context(builder::Disconnected) //.ok_or(Error::Disconnected)
    }

    // fn manually_drop(&mut self) {
    //     unsafe {
    //         ManuallyDrop::drop(&mut self.session);
    //         ManuallyDrop::drop(&mut self.recver);
    //         ManuallyDrop::drop(&mut self.address);
    //     }
    // }

    pub async fn cancel(mut self) -> Result<()> {
        let (sender, recver) = o_channel();
        let request = Request::CancelTcpipForward {
            address: self.address.clone(),
            // port: self.port,
            sender: Some(sender),
        };

        self.session
            .as_ref()
            .context(builder::BadOperation {
                detail: "Listener has been cancelled",
            })?
            .send(request)
            .map_err(|_| builder::Disconnected.build())?;

        recver.await.map_err(|_| builder::Disconnected.build())??;
        self.session = None;
        Ok(())
    }

    // pub fn listen_port(&self) -> u32 {
    //     self.port
    // }

    pub fn listen_address(&self) -> &SocketAddr {
        &self.address
    }
}
