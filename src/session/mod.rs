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

#[derive(Debug, Clone)]
pub struct Prompt<'a> {
    pub content: &'a str,
    pub echo: bool,
}

pub enum InteractiveMethod {
    PAM,
    BSD,
    Other(String),
}

impl AsRef<str> for InteractiveMethod {
    fn as_ref(&self) -> &str {
        match self {
            InteractiveMethod::PAM => "pam",
            InteractiveMethod::BSD => "bsdauth",
            InteractiveMethod::Other(method) => method.as_str(),
        }
    }
}

#[async_trait::async_trait]
pub trait KeyboardInteractive: Send + Sync {
    async fn interactive(
        &mut self,
        name: &str,
        instruction: &str,
        prompts: &[Prompt<'_>],
    ) -> error::Result<Vec<String>>;
}

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("Unexpected behaviour: {}", detail))]
    UnexpectedBehaviour {
        detail: String,
    },
    UnexpectedService {
        expect: String,
        actual: String,
    },
    ChannelOpenFailure {
        reason_code: u32,
        description: String,
    },
    ChannelFailure,
    ChannelAlreadyOpen,
    UnexpectedMessage {
        detail: String,
    },
    Disconnected {
        reason: msg::DisconnectReason,
        description: String,
    },
    UnsupportedKeyType {
        r#type: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    RequestFailure,
    InvalidPort,
    UnexpectedChannelClosed,
    UnexpectedChannelEof,

    #[snafu(transparent)]
    SecureCopyProtocolError {
        source: scp::Error,
    },
    #[snafu(transparent)]
    SSHFileTransferProtocolError {
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

pub struct Session {
    sender: mpsc::Sender<Event>,
}

#[derive(Debug)]
pub enum AuthenticateResult {
    Success,
    PasswordChangeRequired,
    Failure {
        allow_methods: Vec<String>,
        partial_success: bool,
    },
}

impl AuthenticateResult {
    pub fn success(&self) -> bool {
        matches!(self, AuthenticateResult::Success)
    }
}

impl Session {
    const DEFAULT_INITIAL_WINDOW_SIZE: u32 = 64 * 32 * 1024;
    const DEFAULT_MAXIMUM_PACKET_SIZE: u32 = 32 * 1024;

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

    pub async fn renegotiate(&self) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();
        let event = Event::Renegotiate { back: sender };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }

    pub async fn request_authentication(&self) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();

        let event = Event::RequestAuthentication { back: sender };

        self.sender.send_next(event).await?;

        receiver.receive_next().await??;

        Ok(())
    }

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

    pub async fn listen_on_server(
        &self,
        addr: forward::SocketAddr,
        initial_window_size: u32,
        maximum_packet_size: u32,
    ) -> error::Result<forward::Listener> {
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

    #[inline(always)]
    pub async fn channel_open_default(&self) -> error::Result<Channel> {
        self.channel_open(
            Self::DEFAULT_INITIAL_WINDOW_SIZE,
            Self::DEFAULT_MAXIMUM_PACKET_SIZE,
        )
        .await
    }

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

    #[inline(always)]
    pub async fn sftp_open_default(&self) -> error::Result<sftp::Handle> {
        self.sftp_open(
            Self::DEFAULT_INITIAL_WINDOW_SIZE,
            Self::DEFAULT_MAXIMUM_PACKET_SIZE,
        )
        .await
    }

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

    pub async fn channel_clean(&self) -> error::Result<()> {
        let (sender, receiver) = oneshot::channel();

        let event = Event::ChannelClean { back: sender };

        self.sender.send_next(event).await?;

        receiver.receive_next().await?
    }
}

#[cfg(test)]
mod test {
    pub const HOST: &str = "10.211.55.4";
    pub const PORT: u16 = 22;
    pub const USERNAME: &str = "parallels";
    pub const PASSWORD: &str = "qwer1234";

    pub const PRIVATE_KEY: &str = ".ssh/id_rsa";
    pub const PUBLIC_KEY: &str = ".ssh/id_ed25519.pub";

    pub const CERTIFICATE_FILE: &str = ".ssh/id_rsa-cert.pub";

    use super::*;

    use indexmap::IndexMap;
    use rand::RngExt;

    #[easy_ext::ext]
    impl<K, V> IndexMap<K, V> {
        fn shuffle(&mut self) {
            let mut rng = rand::rng();

            // Fisher-Yates shuffle
            for i in (1..self.len()).rev() {
                let j = rng.random_range(0..=i);
                self.swap_indices(i, j);
            }
        }
    }

    #[tokio::test]
    async fn test_authenticate_keyboard_interactive() {
        tracing_subscriber::fmt().init();
        let tcp = tokio::net::TcpStream::connect((HOST, PORT)).await.unwrap();
        let session = Session::handshake(tcp, Config::default(), DefaultNotifier::default())
            .await
            .unwrap();
        session.request_authentication().await.unwrap();

        #[derive(Default)]
        struct KeyboardInteractiveImpl;

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
                Ok(vec![PASSWORD.to_string(); prompts.len()])
            }
        }

        session
            .authenticate_keyboard_interactive(
                USERNAME,
                Box::new(KeyboardInteractiveImpl),
                Default::default(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_authenticate_public_key() {
        let home = std::env::home_dir().unwrap();
        let tcp = tokio::net::TcpStream::connect((HOST, PORT)).await.unwrap();
        let session = Session::handshake(tcp, Config::default(), DefaultNotifier::default())
            .await
            .unwrap();
        session.request_authentication().await.unwrap();
        let mut result = session.authenticate_none(USERNAME).await.unwrap();

        if !result.success() {
            let private_key_file = tokio::fs::read(home.join(PRIVATE_KEY)).await.unwrap();
            let public_key_file = tokio::fs::read(home.join(PUBLIC_KEY)).await.unwrap();

            result = session
                .authenticate_public_key(USERNAME, &private_key_file, Some(&public_key_file), None)
                .await
                .unwrap();
        }

        assert!(result.success());
    }

    #[tokio::test]
    async fn test_authenticate_certificate() {
        let home = std::env::home_dir().unwrap();
        let tcp = tokio::net::TcpStream::connect((HOST, PORT)).await.unwrap();
        let session = Session::handshake(tcp, Config::default(), DefaultNotifier::default())
            .await
            .unwrap();
        session.request_authentication().await.unwrap();
        let mut result = session.authenticate_none(USERNAME).await.unwrap();

        if !result.success() {
            let private_key_file = tokio::fs::read(home.join(PRIVATE_KEY)).await.unwrap();
            let public_key_file = tokio::fs::read(home.join(CERTIFICATE_FILE)).await.unwrap();

            result = session
                .authenticate_public_key(USERNAME, &private_key_file, Some(&public_key_file), None)
                .await
                .unwrap();
        }

        assert!(result.success());
    }

    #[tokio::test]
    async fn test_renegotiate() -> anyhow::Result<()> {
        tracing_subscriber::fmt::init();
        let config = crate::test::Config::load().await?;
        let session = config.open_session().await?;
        session.request_authentication().await?;
        config.authenticate_password(&session).await?;
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
            config.kex.shuffle();
            config.host_key.shuffle();
            config.crypt_client_to_server.shuffle();
            config.crypt_server_to_client.shuffle();
            config.mac_server_to_client.shuffle();
            config.mac_client_to_server.shuffle();
            config.compress_client_to_server.shuffle();
            config.compress_server_to_client.shuffle();
            tracing::info!("host key: {:?}", config.host_key.keys());
            tracing::info!("kex: {:?}", config.kex.keys());
            tracing::info!("encrypt: {:?}", config.crypt_client_to_server.keys());
            tracing::info!("decrypt: {:?}", config.crypt_server_to_client.keys());
            tracing::info!(
                "mac_client_to_server: {:?}",
                config.mac_client_to_server.keys()
            );
            tracing::info!(
                "mac_server_to_client: {:?}",
                config.mac_server_to_client.keys()
            );
            tracing::info!("encode: {:?}", config.compress_client_to_server.keys());
            tracing::info!("decode: {:?}", config.compress_server_to_client.keys());
            let session = {
                let config = crate::test::Config::load().await?;
                let session = config.open_session().await?;
                session.request_authentication().await?;
                config.authenticate_password(&session).await?;
                session
            };

            session
                .disconnect(DisconnectReason::BY_APPLICATION, "close")
                .await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_authenticate_password() {
        let tcp = tokio::net::TcpStream::connect((HOST, PORT)).await.unwrap();
        let session = Session::handshake(tcp, Config::default(), DefaultNotifier::default())
            .await
            .unwrap();
        session.request_authentication().await.unwrap();
        let result = session.authenticate_none(USERNAME).await.unwrap();
        session
            .authenticate_password(USERNAME, PASSWORD)
            .await
            .unwrap();
        println!("result: {:?}", result)
    }

    #[tokio::test]
    async fn test_open_channel() {
        let session = open_session();
        session
            .await
            .channel_open(1024 * 1024, 1024 * 4)
            .await
            .unwrap();
    }

    async fn open_session() -> Session {
        let tcp = tokio::net::TcpStream::connect((HOST, PORT)).await.unwrap();
        let session = Session::handshake(tcp, Config::default(), DefaultNotifier::default())
            .await
            .unwrap();
        session.request_authentication().await.unwrap();
        let mut result = session.authenticate_none(USERNAME).await.unwrap();
        if !result.success() {
            result = session
                .authenticate_password(USERNAME, PASSWORD)
                .await
                .unwrap();
        }
        assert!(result.success());
        session
    }
}
