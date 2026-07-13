//! Sessions: handshake, authentication, channels, and subsystems.
//!
//! This module is the main entry point of the crate. A connection is turned
//! into a [`Session`] with [`Session::handshake`], authenticated with one of
//! the `authenticate_*` methods, and then used to open channels and request
//! forwarding.
//!
//! Types re-exported here:
//!
//! - [`Config`] / [`HandshakeError`] — handshake configuration and failures
//! - [`Notifier`] / [`DefaultNotifier`] — callbacks the server can trigger
//!
//! Submodules:
//!
//! - [`channel`] — connection-layer channels and terminal modes
//! - [`scp`] / [`sftp`] — file transfer subsystems
//! - [`forward`] — TCP/IP and stream-local forwarding
//! - [`event`] — the internal event enum that drives the session task

mod backend;
pub mod channel;
pub mod event;
mod handshake;
mod notifier;
pub mod scp;

mod agent;

pub mod sftp;

pub mod forward;

use handshake::CompatOptions;
pub use handshake::Config;
pub use handshake::Error as HandshakeError;
use handshake::Handshaker;
use snafu::OptionExt;
use tokio::sync::oneshot;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
};

pub use notifier::DefaultNotifier;
pub use notifier::Notifier;

use crate::DEFAULT_CHANNEL_CAPACITY;
use crate::error::builder;
use crate::key::{Parser, Public};
use crate::session::channel::Channel;
use crate::ssh::msg;
use crate::ssh::msg::DisconnectReason;
use crate::{error, ssh::stream::CipherStream};
use backend::SessionInner;
use event::Event;

fn create<T: AsyncRead + AsyncWrite + Unpin + Send, N>(
    session_id: Vec<u8>,
    socket: CipherStream<T>,
    notifier: N,
    config: Config,
    client_version: String,
    server_version: String,
    compat_options: CompatOptions,
    // signer: IndexMap<String, Factory<dyn Signature + Send>>,
) -> (Session, SessionInner<T, N>) {
    let (sender, receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
    let inner = SessionInner::new(
        session_id,
        socket,
        notifier,
        client_version,
        server_version,
        compat_options,
        config,
        receiver,
        sender.downgrade(),
    );
    (Session { sender }, inner)
}

/// A single question asked during `keyboard-interactive` authentication.
#[derive(Debug, Clone)]
pub struct Prompt<'a> {
    /// The question to show to the user, e.g. `"Password:"`.
    pub content: &'a str,
    /// Whether the answer should be echoed back (set for password-style prompts).
    pub echo: bool,
}

/// An SSH user-authentication method name (RFC 4252 §7 and its IANA registry).
///
/// Used to report which methods a server still accepts after a failed
/// [`AuthenticateResult::Failure`], and to parse method name lists coming from
/// the wire.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AuthenticationMethod {
    /// `none` — probe the server for a list of acceptable methods.
    None,
    /// `publickey` — public-key or certificate authentication.
    PublicKey,
    /// `password` — plain password authentication.
    Password,
    /// `hostbased` — host-based authentication (RFC 4252 §9).
    HostBased,
    /// `keyboard-interactive` — arbitrary multi-round interactive prompts.
    KeyboardInteractive,
    /// `gssapi-with-mic` — GSS-API authentication with MIC.
    GssapiWithMIC,
    /// `gssapi-keyex` — GSS-API authentication using an already-established key exchange.
    GssapiKeyExchange,
    /// `gssapi` — GSS-API authentication.
    Gssapi,
    /// `external-keyx` — authentication using an externally established key exchange.
    ExternalKeyExchange,
    /// A method name this crate does not know about, kept verbatim.
    Unknown(String),
}

impl From<&str> for AuthenticationMethod {
    /// Parses a wire-level method name, mapping unknown names to [`AuthenticationMethod::Unknown`].
    fn from(value: &str) -> Self {
        match value {
            "none" => AuthenticationMethod::None,
            "publickey" => AuthenticationMethod::PublicKey,
            "password" => AuthenticationMethod::Password,
            "hostbased" => AuthenticationMethod::HostBased,
            "keyboard-interactive" => AuthenticationMethod::KeyboardInteractive,
            "gssapi-with-mic" => AuthenticationMethod::GssapiWithMIC,
            "gssapi-keyex" => AuthenticationMethod::GssapiKeyExchange,
            "gssapi" => AuthenticationMethod::Gssapi,
            "external-keyx" => AuthenticationMethod::ExternalKeyExchange,
            _ => AuthenticationMethod::Unknown(value.to_string()),
        }
    }
}

impl AsRef<str> for AuthenticationMethod {
    /// Returns the wire-level method name, e.g. `"publickey"`.
    fn as_ref(&self) -> &str {
        match self {
            AuthenticationMethod::None => "none",
            AuthenticationMethod::PublicKey => "publickey",
            AuthenticationMethod::Password => "password",
            AuthenticationMethod::HostBased => "hostbased",
            AuthenticationMethod::KeyboardInteractive => "keyboard-interactive",
            AuthenticationMethod::GssapiWithMIC => "gssapi-with-mic",
            AuthenticationMethod::GssapiKeyExchange => "gssapi-keyex",
            AuthenticationMethod::Gssapi => "gssapi",
            AuthenticationMethod::ExternalKeyExchange => "external-keyx",
            AuthenticationMethod::Unknown(method) => method,
        }
    }
}

impl ToString for AuthenticationMethod {
    /// Returns the wire-level method name.
    fn to_string(&self) -> String {
        self.as_ref().to_string()
    }
}

/// The server-side flavour of `keyboard-interactive` authentication, sent as the
/// list of accepted sub-methods.
pub enum InteractiveMethod {
    /// Linux-PAM (`pam`).
    PAM,
    /// BSD `bsdauth` (`bsdauth`).
    BSD,
    /// Any other sub-method name, kept verbatim.
    Other(String),
}

impl AsRef<str> for InteractiveMethod {
    /// Returns the sub-method name sent to the server.
    fn as_ref(&self) -> &str {
        match self {
            InteractiveMethod::PAM => "pam",
            InteractiveMethod::BSD => "bsdauth",
            InteractiveMethod::Other(method) => method.as_str(),
        }
    }
}

/// Supplies the answers for `keyboard-interactive` authentication.
///
/// The server decides how many rounds it needs; each round delivers a display
/// name, an instruction, and a list of [`Prompt`]s, and this trait returns one
/// answer per prompt, in order.
#[async_trait::async_trait]
pub trait KeyboardInteractive: Send + Sync {
    /// Answers one round of prompts.
    ///
    /// `name` is the authentication name (often the server's banner title),
    /// `instruction` is shown to the user before the prompts, and `prompts`
    /// are the questions themselves. The returned vector must contain exactly
    /// one entry per prompt.
    async fn interactive(
        &mut self,
        name: &str,
        instruction: &str,
        prompts: &[Prompt<'_>],
    ) -> error::Result<Vec<String>>;
}

/// Errors raised by session-level operations (authentication, channel requests,
/// forwarding, SCP/SFTP handshakes).
#[derive(Debug, snafu::Snafu)]
pub enum Error {
    /// The peer behaved in a way the protocol does not allow.
    #[snafu(display("Unexpected behaviour: {}", detail))]
    UnexpectedBehaviour {
        /// What the peer did, in human-readable form.
        detail: String,
    },
    /// The server accepted a different service than the one that was requested.
    #[snafu(display("Unexpected service: expected {}, got {}", expect, actual))]
    UnexpectedService {
        /// The service name requested by the client.
        expect: String,
        /// The service name the server actually confirmed.
        actual: String,
    },
    /// The server refused to open a channel (`SSH_MSG_CHANNEL_OPEN_FAILURE`).
    #[snafu(display("Channel open failure (code {}): {}", reason_code, description))]
    ChannelOpenFailure {
        /// The `SSH_OPEN_*` reason code sent by the server.
        reason_code: u32,
        /// The human-readable description sent by the server.
        description: String,
    },
    /// The server answered a channel request with `SSH_MSG_CHANNEL_FAILURE`.
    #[snafu(display("Channel failure"))]
    ChannelFailure,
    /// A channel with the same identity is already open.
    #[snafu(display("Channel already open"))]
    ChannelAlreadyOpen,
    /// A message arrived that is invalid in the current state.
    #[snafu(display("Unexpected message: {}", detail))]
    UnexpectedMessage {
        /// Which message was unexpected, and why.
        detail: String,
    },
    /// The peer disconnected (`SSH_MSG_DISCONNECT`).
    #[snafu(display("Disconnected: {:?} - {}", reason, description))]
    Disconnected {
        /// The protocol-level reason code.
        reason: msg::DisconnectReason,
        /// The description supplied by the peer.
        description: String,
    },
    /// A key type was encountered that this crate cannot handle.
    #[snafu(display("Unsupported key type: {}", r#type))]
    UnsupportedKeyType {
        /// The key type name.
        r#type: String,
        /// Source location of the failure (provided by `snafu`).
        #[snafu(implicit)]
        location: snafu::Location,
    },
    /// The peer answered a request with `SSH_MSG_CHANNEL_FAILURE`/`REQUEST_FAILURE`.
    #[snafu(display("Request failure"))]
    RequestFailure,
    /// A forwarding port number was invalid.
    #[snafu(display("Invalid port"))]
    InvalidPort,
    /// The channel closed while a request was still in flight.
    #[snafu(display("Unexpected channel closed"))]
    UnexpectedChannelClosed,
    /// The peer sent EOF while data was still expected.
    #[snafu(display("Unexpected channel EOF"))]
    UnexpectedChannelEof,

    /// A window adjustment carried an impossible size.
    #[snafu(display("Unexpected window size"))]
    UnexpectedWindowSize,

    /// More data was offered than the receive window allows.
    #[snafu(display("Channel window overflow"))]
    ChannelWindowOverflow,

    /// The SCP layer reported a failure.
    #[snafu(transparent)]
    SecureCopyProtocolError {
        /// The underlying SCP error.
        source: scp::Error,
    },
    /// The SFTP layer reported a failure.
    #[snafu(transparent)]
    SSHFileTransferProtocolError {
        /// The underlying SFTP error.
        source: sftp::Error,
    },
}

#[easy_ext::ext(UnexpectedSendingError)]
impl<T> mpsc::Sender<T> {
    async fn send_next(&self, v: T) -> error::Result<()> {
        self.send(v).await.ok().context(UnexpectedBehaviourSnafu {
            detail: "Maybe session was shutdown",
        })?;
        Ok(())
    }
}
#[easy_ext::ext(UnexpectedReceivingError)]
impl<T> oneshot::Receiver<T> {
    async fn receive_next(self) -> error::Result<T> {
        let v = self.await.ok().context(UnexpectedBehaviourSnafu {
            detail: "Maybe session was shutdown",
        })?;
        Ok(v)
    }
}

/// A handle to an established SSH connection.
///
/// A `Session` is cheap to clone-free and shared: every method forwards an
/// event to the background task created during
/// [`Session::handshake`] and waits for its reply, so a single session can
/// drive several channels concurrently.
///
/// Dropping the last handle does not close the connection; call
/// [`Session::disconnect`] for an orderly shutdown.
#[derive(derive_more::Debug)]
pub struct Session {
    #[debug(skip)]
    sender: mpsc::Sender<Event>,
}

/// The outcome of an authentication attempt (RFC 4252 §5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthenticateResult {
    /// `SSH_MSG_USERAUTH_SUCCESS` — the user is authenticated.
    Success,
    /// `SSH_MSG_USERAUTH_PK_OK`/password-change request — the server demands a
    /// new password before it will accept this credential.
    PasswordChangeRequired,
    /// `SSH_MSG_USERAUTH_FAILURE` — the attempt was rejected.
    Failure {
        /// Methods the server still accepts for this user.
        allow_methods: Vec<AuthenticationMethod>,
        /// Whether some earlier method already succeeded (multi-factor setups).
        partial_success: bool,
    },
}

impl AuthenticateResult {
    /// Returns `true` only for [`AuthenticateResult::Success`].
    pub fn success(&self) -> bool {
        matches!(self, AuthenticateResult::Success)
    }
}

impl Session {
    /// The receive window (in bytes) opened by [`Session::channel_open_default`]
    /// and [`Session::sftp_open_default`]: 2 MiB.
    pub const DEFAULT_INITIAL_WINDOW_SIZE: u32 = 64 * 32 * 1024;
    /// The maximum channel data packet size (in bytes) used by
    /// [`Session::channel_open_default`] and [`Session::sftp_open_default`]: 32 KiB.
    pub const DEFAULT_MAXIMUM_PACKET_SIZE: u32 = 32 * 1024;

    /// Sends `SSH_MSG_DISCONNECT` and tears the connection down.
    ///
    /// `description` is the human-readable text shown by the peer; `reason` is
    /// the protocol reason code (e.g. [`DisconnectReason::BY_APPLICATION`]).
    /// The call resolves once the message has been handed to the transport.
    pub async fn disconnect(
        &self,
        reason: DisconnectReason,
        description: impl Into<String>,
    ) -> error::Result<()> {
        let description = description.into();
        let (sender, receiver) = oneshot::channel();

        let event = Event::Disconnect {
            reason: reason.0,
            description,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await??;

        Ok(())
    }

    /// Sends `SSH_MSG_IGNORE` carrying arbitrary `data`.
    ///
    /// Useful for testing or for keeping a connection busy; the peer must
    /// discard the payload.
    pub async fn send_ignore_message(&self, data: impl Into<Vec<u8>>) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();

        let event = Event::SendIgnoreMessage {
            data: data.into(),
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await??;

        Ok(())
    }

    /// Sends `SSH_MSG_DEBUG` with the given `message`.
    ///
    /// `always_display` maps to the protocol's `always_display` flag: when
    /// `true` the peer should show the message even if debugging is off.
    pub async fn send_debug_message(
        &self,
        always_display: bool,
        message: impl Into<String>,
    ) -> error::Result<()> {
        let message = message.into();

        let (sender, receiver) = oneshot::channel();

        let event = Event::SendDebugMessage {
            always_display,
            message,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await??;

        Ok(())
    }

    /// Starts a key re-exchange (rekey) and waits for it to complete.
    ///
    /// OpenSSH-compatible clients rekey after a set amount of data or time;
    /// this forces one immediately. All channels stay open across the rekey.
    pub async fn renegotiate(&self) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::Renegotiate { back: sender };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Requests the `ssh-userauth` service (`SSH_MSG_SERVICE_REQUEST`).
    ///
    /// This must be awaited before any `authenticate_*` call; it is a separate
    /// step because the transport layer authenticates the connection, not the
    /// user, first.
    pub async fn request_authentication(&self) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();

        let event = Event::RequestAuthentication { back: sender };

        self.sender.send_next(event).await?;

        receiver.receive_next().await??;

        Ok(())
    }

    /// Attempts authentication with the `none` method.
    ///
    /// Servers use this to report the list of methods they accept (see
    /// [`AuthenticateResult::Failure`]), which is how a client decides which
    /// credential to offer next.
    pub async fn authenticate_none(
        &self,
        username: impl Into<String>,
    ) -> error::Result<AuthenticateResult> {
        let username = username.into();

        let (sender, receiver) = oneshot::channel();

        let event = Event::AuthenticateNone {
            username,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Attempts `password` authentication (RFC 4252 §8).
    ///
    /// The result reports success, a password-change demand, or a rejection
    /// with the methods the server still accepts.
    pub async fn authenticate_password(
        &self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> error::Result<AuthenticateResult> {
        let username = username.into();
        let password = password.into();

        let (sender, receiver) = oneshot::channel();

        let event = Event::AuthenticatePassword {
            username,
            password,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Attempts `publickey` (or certificate) authentication (RFC 4252 §7).
    ///
    /// - `username` is the account to log in as.
    /// - `private_key_file` is the raw contents of a private key file (OpenSSH
    ///   `openssh-key-v1` or PEM). `passphrase` is the key's passphrase, if
    ///   any.
    /// - `public_key_file` is the optional contents of the matching `.pub`
    ///   file. When given it is cross-checked against the private key (type
    ///   and blob must match) and, if it is an OpenSSH certificate, the
    ///   certificate is offered instead of the bare key.
    ///
    /// Returns [`error::Error::InvalidArgument`] if the public and private key
    /// files do not belong together.
    pub async fn authenticate_public_key(
        &self,
        username: impl Into<String>,
        private_key_file: impl AsRef<[u8]>,
        public_key_file: Option<impl AsRef<[u8]>>,
        passphrase: Option<&[u8]>,
    ) -> error::Result<AuthenticateResult> {
        let username = username.into();
        let parser = Parser::default();
        let private = parser.parse_private_key_file(private_key_file.as_ref(), passphrase)?;

        let (sender, receiver) = oneshot::channel();

        let event = if let Some(public_key_file) = public_key_file {
            let public = parser.parse_public_key_file(public_key_file.as_ref())?;
            match public {
                Public::Normal {
                    r#type,
                    content,
                    comment,
                } => {
                    if r#type != private.r#type {
                        return builder::InvalidArgument {
                            detail: "Public key file and private key file mismatch",
                        }
                        .fail();
                    }
                    if content != private.public {
                        return builder::InvalidArgument {
                            detail: "Public key file and private key file mismatch",
                        }
                        .fail();
                    }
                    if comment.unwrap_or_default() != private.comment {
                        tracing::warn!("Public key file and private key file comment mismatch");
                    }
                    Event::AuthenticatePublicKey {
                        username,
                        method: r#type,
                        is_certificate: false,
                        public_blob: private.public,
                        private_blob: private.private,
                        back: sender,
                    }
                }
                Public::Certificate {
                    r#type,
                    content,
                    comment,
                    principals,
                    ..
                } => {
                    let cert_type = format!("{}{}", private.r#type, crate::key::CERT_SUFFIX);
                    if r#type != cert_type {
                        return builder::InvalidArgument {
                            detail: "Public key file and private key file mismatch",
                        }
                        .fail();
                    }
                    if comment.unwrap_or_default() != private.comment {
                        tracing::warn!("Public key file and private key file comment mismatch");
                    }
                    if !principals.contains(&username) {
                        tracing::warn!(
                            "Maybe {} is not allowed to use this certificate to authenticate",
                            username
                        );
                    }

                    Event::AuthenticatePublicKey {
                        username,
                        method: private.r#type,
                        is_certificate: true,
                        public_blob: content,
                        private_blob: private.private,
                        back: sender,
                    }
                }
            }
        } else {
            Event::AuthenticatePublicKey {
                username,
                method: private.r#type,
                is_certificate: false,
                public_blob: private.public,
                private_blob: private.private,
                back: sender,
            }
        };
        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Attempts `keyboard-interactive` authentication (RFC 4252 §9).
    ///
    /// `interactive` is called once per round with the server's prompts and
    /// returns the answers; `methods` lists the sub-methods (e.g. PAM,
    /// bsdauth) the client offers — an empty list leaves the choice to the
    /// server.
    pub async fn authenticate_keyboard_interactive(
        &self,
        username: impl Into<String>,
        interactive: Box<dyn KeyboardInteractive>,
        methods: Vec<InteractiveMethod>,
    ) -> error::Result<AuthenticateResult> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::AuthenticateKeyboardInteractive {
            username: username.into(),
            interactive,
            methods,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Runs the SSH handshake over `socket` and returns a [`Session`].
    ///
    /// This performs, in order: the identification string exchange, algorithm
    /// negotiation (`SSH_MSG_KEXINIT`), key exchange and host-key
    /// verification, and `SSH_MSG_NEWKEYS`. On success a background task is
    /// spawned that owns the encrypted socket and dispatches every later
    /// request; the returned handle is how you talk to it.
    ///
    /// `config` selects the offered algorithms (see [`Config::default`]), and
    /// `notifier` receives host-key and forwarding callbacks.
    pub async fn handshake<T, N>(socket: T, config: Config, notifier: N) -> error::Result<Self>
    where
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
        N: Notifier + Send + 'static,
    {
        let mut shaker = Handshaker::new(socket, notifier, config);
        shaker.banner_version_exchange().await?;
        shaker.negotiate_methods().await?;
        shaker.key_exchange().await?;
        let (session, mut inner) = create(
            shaker.session_id.unwrap(),
            shaker.cipher_stream.take().unwrap(),
            shaker.notifier,
            shaker.config,
            shaker.client_version,
            shaker.server_version.take().unwrap(),
            shaker.compat_options,
        );

        tokio::spawn(async move {
            let result = inner.exec().await;
            tracing::info!("Session exited with {:#?}", result);
            inner.notify_exited(result).await;
        });

        Ok(session)
    }

    /// Asks the server to forward connections to the Unix socket `path`
    /// (stream-local forwarding).
    ///
    /// Returns a [`forward::Listener`] whose [`accept`](forward::Listener::accept)
    /// yields one [`forward::Stream`] per incoming connection. Dropping the
    /// listener asks the server to stop forwarding.
    ///
    /// `initial_window_size` and `maximum_packet_size` size the receive window
    /// of every accepted channel; see [`Session::DEFAULT_INITIAL_WINDOW_SIZE`]
    /// and [`Session::DEFAULT_MAXIMUM_PACKET_SIZE`] for sensible values.
    pub async fn listen_on_server_local(
        &self,
        path: impl Into<String>,
        initial_window_size: u32,
        maximum_packet_size: u32,
    ) -> error::Result<forward::Listener<String, ()>> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::GlobalRequestStreamLocalForward {
            path: path.into(),
            initial_window_size,
            maximum_packet_size,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Asks the server to listen on `addr` and forward accepted TCP
    /// connections back to this client (`tcpip-forward`).
    ///
    /// If `addr.port` is `0` the server picks a port; the bound address —
    /// including the assigned port — is available from
    /// [`Listener::addr`](forward::Listener::addr). Each accepted connection
    /// arrives as `(Stream, originator)` from
    /// [`Listener::accept`](forward::Listener::accept), where the second
    /// element is the address of the peer that connected to the forwarded
    /// port.
    ///
    /// Dropping the listener cancels the forward.
    pub async fn listen_on_server(
        &self,
        addr: forward::SocketAddr,
        initial_window_size: u32,
        maximum_packet_size: u32,
    ) -> error::Result<forward::Listener<forward::SocketAddr, forward::SocketAddr>> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::GlobalRequestTcpIPForward {
            addr,
            initial_window_size,
            maximum_packet_size,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Opens a channel directly to the Unix socket `path` on the server
    /// (direct stream-local forwarding).
    ///
    /// Unlike [`Session::listen_on_server_local`], this initiates the
    /// connection: the returned [`forward::Stream`] is already connected.
    pub async fn connect_to_server_local(
        &self,
        path: impl Into<String>,
        initial_window_size: u32,
        maximum_packet_size: u32,
    ) -> error::Result<forward::Stream> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelOpenDirectStreamLocal {
            initial_window_size,
            maximum_packet_size,
            path: path.into(),
            back: sender,
        };

        self.sender.send_next(event).await?;

        let channel = receiver.receive_next().await??;

        Ok(forward::Stream::new(channel))
    }

    /// Opens a channel to `target` as seen from the server (`direct-tcpip`).
    ///
    /// `source` describes the client endpoint the connection appears to
    /// originate from (address and port reported to the server); it is
    /// informational, so [`forward::LOCALHOST`] with an arbitrary port is
    /// fine when you have no real client endpoint. The returned
    /// [`forward::Stream`] behaves like a socket connected to `target`.
    pub async fn connect_to_server(
        &self,
        target: forward::SocketAddr,
        source: forward::SocketAddr,
        initial_window_size: u32,
        maximum_packet_size: u32,
    ) -> error::Result<forward::Stream> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::ChannelOpenDirectTcpIp {
            target,
            source,
            initial_window_size,
            maximum_packet_size,
            back: sender,
        };

        self.sender.send_next(event).await?;

        let channel = receiver.receive_next().await??;

        Ok(forward::Stream::new(channel))
    }

    /// Opens a `session` channel using [`Session::DEFAULT_INITIAL_WINDOW_SIZE`]
    /// and [`Session::DEFAULT_MAXIMUM_PACKET_SIZE`].
    #[inline(always)]
    pub async fn channel_open_default(&self) -> error::Result<Channel> {
        self.channel_open(
            Self::DEFAULT_INITIAL_WINDOW_SIZE,
            Self::DEFAULT_MAXIMUM_PACKET_SIZE,
        )
        .await
    }

    /// Opens a `session` channel (`SSH_MSG_CHANNEL_OPEN`) with an explicit
    /// receive window and maximum packet size.
    ///
    /// `initial_window_size` is how many bytes this side is willing to
    /// receive before the peer must send a window adjustment;
    /// `maximum_packet_size` caps a single channel data packet.
    pub async fn channel_open(
        &self,
        initial_window_size: u32,
        maximum_packet_size: u32,
    ) -> error::Result<Channel> {
        let (sender, receiver) = oneshot::channel();

        let event = Event::ChannelOpenSession {
            initial_window_size,
            maximum_packet_size,
            back: sender,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    /// Opens a `session` channel and starts the `sftp` subsystem using the
    /// default window sizes (see [`Session::sftp_open`]).
    #[inline(always)]
    pub async fn sftp_open_default(&self) -> error::Result<sftp::Handle> {
        self.sftp_open(
            Self::DEFAULT_INITIAL_WINDOW_SIZE,
            Self::DEFAULT_MAXIMUM_PACKET_SIZE,
        )
        .await
    }

    /// Opens a `session` channel, requests the `sftp` subsystem, and performs
    /// the SFTP version exchange, returning a ready [`sftp::Handle`].
    ///
    /// See [`Session::channel_open`] for the meaning of the window parameters.
    pub async fn sftp_open(
        &self,
        initial_window_size: u32,
        maximum_packet_size: u32,
    ) -> error::Result<sftp::Handle> {
        let (sender, receiver) = oneshot::channel();

        let event = Event::ChannelOpenSFTP {
            initial_window_size,
            maximum_packet_size,
            back: sender,
        };

        self.sender.send_next(event).await?;

        let channel = receiver.receive_next().await??;

        sftp::Handle::handshake(channel).await
    }

    /// Tears down state the session is holding on behalf of this client.
    ///
    /// Each flag selects what to clean up: `channel` closes every open
    /// channel, `forward_tcp` cancels TCP/IP forwards, and `forward_local`
    /// cancels stream-local (Unix socket) forwards. A flag set to `false`
    /// leaves that category untouched.
    pub async fn clean(
        &self,
        channel: bool,
        forward_tcp: bool,
        forward_local: bool,
    ) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();

        let event = Event::Clean {
            back: sender,
            channel,
            forward_tcp,
            forward_local,
        };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test::{Config as TestHandle, ShuffleConfig};

    #[tokio::test]
    async fn test_authenticate_keyboard_interactive() -> anyhow::Result<()> {
        tracing_subscriber::fmt().init();

        let handle = TestHandle::load().await?;

        let session = handle.open_session().await?;
        session.request_authentication().await?;

        struct KeyboardInteractiveImpl {
            handle: TestHandle,
        }

        #[async_trait::async_trait]
        impl KeyboardInteractive for KeyboardInteractiveImpl {
            async fn interactive(
                &mut self,
                name: &str,
                instruction: &str,
                prompts: &[Prompt<'_>],
            ) -> error::Result<Vec<String>> {
                tracing::info!(
                    "Interactive: name={}, instruction={}, prompts={:?}",
                    name,
                    instruction,
                    prompts
                );
                Ok(vec![
                    self.handle.authentication.password.clone();
                    prompts.len()
                ])
            }
        }

        session
            .authenticate_keyboard_interactive(
                handle.authentication.username.clone(),
                Box::new(KeyboardInteractiveImpl { handle }),
                Default::default(),
            )
            .await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_authenticate_public_key() -> anyhow::Result<()> {
        let home = std::env::home_dir().unwrap();

        let handle = TestHandle::load().await?;

        let session = handle.open_session().await?;
        session.request_authentication().await?;
        let private_key_file =
            tokio::fs::read(home.join(handle.authentication.private_key.as_str())).await?;
        let public_key_file =
            tokio::fs::read(home.join(handle.authentication.public_key.as_str())).await?;

        let passphrase = handle
            .authentication
            .passphrase
            .as_ref()
            .map(|v| v.as_str());

        let status = session
            .authenticate_public_key(
                handle.authentication.username.clone(),
                private_key_file,
                Some(public_key_file),
                passphrase.map(|v| v.as_bytes()),
            )
            .await?;

        anyhow::ensure!(status.success(), "Failed to authenticate with public key");

        Ok(())
    }

    #[tokio::test]
    async fn test_authenticate_certificate() -> anyhow::Result<()> {
        let home = std::env::home_dir().unwrap();

        let handle = TestHandle::load().await?;

        let session = handle.open_session().await?;
        session.request_authentication().await?;
        let private_key_file =
            tokio::fs::read(home.join(handle.authentication.private_key.as_str())).await?;
        let public_key_file =
            tokio::fs::read(home.join(handle.authentication.certificate.as_str())).await?;

        let passphrase = handle
            .authentication
            .passphrase
            .as_ref()
            .map(|v| v.as_str());

        let status = session
            .authenticate_public_key(
                handle.authentication.username.clone(),
                private_key_file,
                Some(public_key_file),
                passphrase.map(|v| v.as_bytes()),
            )
            .await?;

        anyhow::ensure!(status.success(), "Failed to authenticate with public key");

        Ok(())
    }

    #[tokio::test]
    async fn test_renegotiate() -> anyhow::Result<()> {
        tracing_subscriber::fmt::init();
        let handle = TestHandle::load().await?;
        let session = handle.open_session_simple().await?;
        session
            .send_debug_message(true, "Start renegotiating")
            .await?;
        session.renegotiate().await?;
        session
            .send_debug_message(true, "Finished renegotiating")
            .await?;
        session
            .send_debug_message(true, "About to disconnect")
            .await?;
        session
            .disconnect(DisconnectReason::BY_APPLICATION, "Disconnecting")
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_handshake() -> anyhow::Result<()> {
        tracing_subscriber::fmt::init();

        for _ in 0..999 {
            let mut config = Config::default();
            config.shuffle();
            tracing::info!("Using config: {:?}", config);
            let session = {
                let handle = TestHandle::load().await?;
                let session = handle.open_session_with_config(config).await?;
                session.request_authentication().await?;
                handle.authenticate_password(&session).await?;
                session
            };

            session.send_debug_message(true, "DEBUG handshake").await?;

            session
                .disconnect(DisconnectReason::BY_APPLICATION, "close")
                .await?;
        }
        Ok(())
    }

    async fn open_session_simple() -> anyhow::Result<Session> {
        let config = crate::test::Config::load().await?;
        let session = config.open_session_simple().await?;
        Ok(session)
    }

    #[tokio::test]
    async fn test_authenticate_password() -> anyhow::Result<()> {
        let session = open_session_simple().await?;
        session
            .disconnect(DisconnectReason::BY_APPLICATION, "Close")
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_open_channel() -> anyhow::Result<()> {
        let session = open_session_simple().await?;

        let channel = session.channel_open_default().await?;

        channel.close().await?;

        session
            .disconnect(DisconnectReason::BY_APPLICATION, "Close")
            .await?;

        Ok(())
    }
}
