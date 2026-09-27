//! TCP/IP and stream-local (Unix socket) forwarding.
//!
//! Two directions are supported:
//!
//! - **Remote forwarding** — ask the server to listen somewhere and hand the
//!   accepted connections back here:
//!   [`Session::listen_on_server`](crate::session::Session::listen_on_server) /
//!   [`Session::listen_on_server_local`](crate::session::Session::listen_on_server_local)
//!   return a [`Listener`].
//! - **Direct forwarding** — open a channel straight to a target:
//!   [`Session::connect_to_server`](crate::session::Session::connect_to_server) /
//!   [`Session::connect_to_server_local`](crate::session::Session::connect_to_server_local)
//!   return a [`Stream`].
//!
//! A [`Listener`] cancels its forward when dropped, and a [`Stream`] closes
//! its channel when dropped.

use super::Event;
use super::{UnexpectedBehaviourSnafu, UnexpectedReceivingError, UnexpectedSendingError, channel};
use crate::error;
use crate::session::channel::Channel;
use channel::Message as ChannelMessage;
use snafu::OptionExt;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};

/// Wildcard address accepted by forwarding requests: the empty string, meaning
/// "every interface" for both IPv4 and IPv6.
pub const ALL: &str = "";
/// The IPv4 wildcard address `0.0.0.0`.
pub const IPV4_ALL: &str = "0.0.0.0";
/// The IPv6 wildcard address `::`.
pub const IPV6_ALL: &str = "::";
/// The `localhost` name, accepted wherever a forwarding address is expected.
pub const LOCALHOST: &str = "localhost";
/// The IPv4 loopback address `127.0.0.1`.
pub const IPV4_LOCALHOST: &str = "127.0.0.1";
/// The IPv6 loopback address `::1`.
pub const IPV6_LOCALHOST: &str = "::1";

/// An address used by forwarding requests, as a name plus a port.
///
/// The host part is a string because forwarding addresses may be hostnames or
/// wildcard names (see [`ALL`], [`LOCALHOST`], ...) as well as IP literals.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct SocketAddr {
    /// Hostname, IP literal, or one of the wildcard constants.
    pub host: String,
    /// Port number; `0` asks the server to choose one when listening.
    pub port: u16,
}

impl SocketAddr {
    /// Creates an address from `host` and `port`.
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }
}

impl ToString for SocketAddr {
    /// Formats the address as `host:port`.
    fn to_string(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// A chunk of traffic read from a forwarded connection.
pub enum Message {
    /// The peer closed the connection and no more data will arrive.
    Close,
    /// The peer sent EOF: no more data will be sent, but the channel is still open.
    Eof,
    /// Application data received from the peer.
    Bytes(Vec<u8>),
}

/// A server-side listener created by a remote-forwarding request.
///
/// `A` is the address type reported by [`Listener::addr`] (the bound
/// [`SocketAddr`] for TCP forwards, the socket `path` for stream-local
/// forwards) and `B` is the extra information delivered alongside each
/// accepted connection (the originator [`SocketAddr`] for TCP forwards, `()`
/// for stream-local forwards).
///
/// Dropping the listener asks the server to stop forwarding.
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

    /// Returns the address this listener is bound to.
    ///
    /// For TCP forwards this reflects the port the server actually assigned
    /// when the requested port was `0`.
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
    /// Cancels a stream-local forward and waits for the server's reply.
    ///
    /// `want_reply` asks the server to answer the cancel request. The
    /// listener is consumed; dropping it later would not send a second
    /// cancellation.
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

    /// Waits for the next connection forwarded to the socket path this
    /// listener was created for.
    ///
    /// Returns an error if the session shut down while waiting.
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
    /// Cancels a TCP/IP forward and waits for the server's reply.
    ///
    /// `want_reply` asks the server to answer the cancel request. The
    /// listener is consumed; dropping it later would not send a second
    /// cancellation.
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

    /// Waits for the next connection accepted by the forwarded port.
    ///
    /// Returns the data [`Stream`] together with the originator's
    /// [`SocketAddr`] — the address of the peer that connected to the
    /// forwarded port.
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

/// A byte stream carried over a connection channel.
///
/// This is what a forwarded connection looks like to the application: read
/// with [`Stream::receive`], write with [`Stream::send`]. Dropping the stream
/// closes the underlying channel.
#[derive(Debug)]
pub struct Stream {
    channel: Channel,
}

impl Stream {
    pub(super) fn new(channel: Channel) -> Self {
        Self { channel }
    }

    /// Closes the channel behind this stream, consuming the stream.
    pub async fn close(self) -> error::Result<()> {
        self.channel.close().await
    }

    /// Sends EOF to the peer while leaving the channel open.
    pub async fn eof(&self) -> error::Result<()> {
        self.channel.eof().await
    }

    /// Reads the next chunk of traffic, waiting for one if necessary.
    ///
    /// Returns [`Message::Bytes`] for data, [`Message::Eof`] when the peer is
    /// done sending, and [`Message::Close`] when the channel is gone. Any
    /// message that cannot occur on a forwarding channel (standard error,
    /// exit status, ...) is logged and skipped.
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

    /// Writes `data` to the peer, returning how many bytes were accepted.
    ///
    /// The transport may accept fewer bytes than supplied; callers should
    /// keep sending until everything has been written.
    pub async fn send(&self, data: Vec<u8>) -> error::Result<usize> {
        self.channel.send(data).await
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn socket_addr_formats_as_host_colon_port() {
        assert_eq!(
            SocketAddr::new("example.com".into(), 22).to_string(),
            "example.com:22"
        );
        assert_eq!(
            SocketAddr::new("127.0.0.1".into(), 8080).to_string(),
            "127.0.0.1:8080"
        );
        // IPv6 literals are not bracketed: the SSH forwarding protocol
        // carries host and port as separate fields, so this is only for
        // display.
        assert_eq!(SocketAddr::new("::1".into(), 22).to_string(), "::1:22");
    }

    #[test]
    fn socket_addr_port_zero_is_allowed() {
        // Port 0 asks the server to pick a free port when listening.
        let addr = SocketAddr::new("localhost".into(), 0);
        assert_eq!(addr.port, 0);
        assert_eq!(addr.to_string(), "localhost:0");
    }

    #[test]
    fn socket_addr_default_is_the_empty_wildcard() {
        // The default (empty host, port 0) is what "every interface, any
        // port" looks like on the wire.
        let addr = SocketAddr::default();
        assert_eq!(addr.host, ALL);
        assert_eq!(addr.port, 0);
        assert_eq!(addr.to_string(), ":0");
    }

    #[test]
    fn socket_addr_equality_and_hashing_use_both_fields() {
        use std::collections::HashSet;

        let a = SocketAddr::new("localhost".into(), 22);
        let b = SocketAddr::new("localhost".into(), 22);
        let c = SocketAddr::new("localhost".into(), 2222);
        let d = SocketAddr::new("127.0.0.1".into(), 22);

        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);

        // Hashing must agree with equality so these can key a set.
        let mut set = HashSet::new();
        set.insert(a);
        set.insert(b);
        set.insert(c);
        set.insert(d);
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn socket_addr_is_usable_as_a_map_key() {
        use std::collections::HashMap;

        let mut map = HashMap::new();
        map.insert(SocketAddr::new("localhost".into(), 8080), "forward-1");
        assert_eq!(
            map.get(&SocketAddr::new("localhost".into(), 8080)),
            Some(&"forward-1")
        );
        assert_eq!(map.get(&SocketAddr::new("localhost".into(), 8081)), None);
    }

    #[test]
    fn address_constants_hold_the_expected_values() {
        assert_eq!(ALL, "");
        assert_eq!(IPV4_ALL, "0.0.0.0");
        assert_eq!(IPV6_ALL, "::");
        assert_eq!(LOCALHOST, "localhost");
        assert_eq!(IPV4_LOCALHOST, "127.0.0.1");
        assert_eq!(IPV6_LOCALHOST, "::1");
    }

    #[test]
    fn wildcard_and_loopback_constants_are_distinct() {
        // The three wildcard spellings must not collide, nor the loopbacks.
        assert_ne!(ALL, IPV4_ALL);
        assert_ne!(ALL, IPV6_ALL);
        assert_ne!(IPV4_ALL, IPV6_ALL);
        assert_ne!(LOCALHOST, IPV4_LOCALHOST);
        assert_ne!(LOCALHOST, IPV6_LOCALHOST);
        assert_ne!(IPV4_LOCALHOST, IPV6_LOCALHOST);
    }
}
