//! An async SSH-2.0 client library built on Tokio.
//!
//! `flatline` implements the SSH transport, authentication, and connection
//! layers as a client, together with the subsystems that sit on top of a
//! connection channel: shell/exec sessions, [SCP](scp) file transfer,
//! [SFTP](sftp), and TCP/IP (plus Unix-socket) [forwarding](forward).
//!
//! # Overview
//!
//! The usual flow is:
//!
//! 1. [`session::Session::handshake`] performs the version exchange, algorithm
//!    negotiation, and key exchange over any Tokio async stream (typically a
//!    `tokio::net::TcpStream`), producing a [`session::Session`].
//! 2. [`session::Session::request_authentication`] requests the
//!    `ssh-userauth` service, then one of the `authenticate_*` methods
//!    ([`session::Session::authenticate_password`],
//!    [`session::Session::authenticate_public_key`],
//!    [`session::Session::authenticate_none`],
//!    [`session::Session::authenticate_keyboard_interactive`]) completes
//!    authentication.
//! 3. Channels are opened — [`session::Session::channel_open_default`] for an
//!    interactive/exec session, [`session::Session::sftp_open_default`] for
//!    SFTP, or [`session::Session::connect_to_server`] for a forwarded
//!    connection — and are driven by the methods on the returned handle.
//!
//! A [`session::Notifier`] is supplied at handshake time and is consulted for
//! host-key verification and for server-initiated requests (X11 and agent
//! forwarding); it also observes disconnects and session exit.
//!
//! # Example
//!
//! ```rust,no_run
//! use flatline::session::{Config, DefaultNotifier, Session};
//! use flatline::session::channel::Message;
//! use tokio::net::TcpStream;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let socket = TcpStream::connect("example.com:22").await?;
//! let session = Session::handshake(socket, Config::default(), DefaultNotifier).await?;
//!
//! session.request_authentication().await?;
//! let status = session.authenticate_password("user", "password").await?;
//! assert!(status.success());
//!
//! let mut channel = session.channel_open_default().await?;
//! channel.request_exec(true, "echo hello").await?;
//!
//! while let Ok(msg) = channel.receive().await {
//!     match msg {
//!         Message::Stdout(data) => println!("{}", String::from_utf8_lossy(&data)),
//!         Message::Exit(_) | Message::Close => break,
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Modules
//!
//! | Module | Contents |
//! |--------|----------|
//! | [`session`] | Sessions, authentication, channels, and the subsystem handles |
//! | [`session::channel`] | Connection-layer channels, terminal modes, and messages |
//! | [`session::scp`] | The legacy SCP file transfer protocol |
//! | [`session::sftp`] | The SFTP protocol (v3 plus OpenSSH extensions) |
//! | [`session::forward`] | TCP/IP and stream-local (Unix socket) forwarding |
//! | [`key`] | Parsing OpenSSH private keys, public keys, and certificates |
//! | [`cipher`] | Algorithm traits and registries (KEX, cipher, MAC, compression, signatures) |
//! | [`ssh`] | Transport-layer message types and error reporting |
//! | [`error`] | The crate-wide [`error::Error`] type and [`error::Result`] alias |
//!
//! # Feature flags
//!
//! - `umac` — enable the UMAC message authentication codes.
//! - `strict` — stricter protocol behaviour.
//! - `openssl-vendored` — build OpenSSL from source instead of linking the system one.
//! - `openssh` — enable async filesystem helpers used by OpenSSH-compat code paths.

#![warn(missing_docs)]

pub mod cipher;
pub mod error;
#[macro_use]
pub mod ssh;
pub mod key;
pub mod session;
mod stream;
pub use session::channel;
pub use session::forward;
pub use session::scp;
pub use session::sftp;

/// Number of buffered events the session event loop accepts before back-pressure applies.
const DEFAULT_CHANNEL_CAPACITY: usize = 256;

#[cfg(test)]
mod test {
    use crate::session;
    use indexmap::IndexMap;
    use rand::RngExt;
    use serde::{Deserialize, Serialize};
    use tokio::net::TcpStream;

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

    #[easy_ext::ext(ShuffleConfig)]
    pub impl session::Config {
        fn shuffle(&mut self) {
            let mut rng = rand::rng();
            self.kex.shuffle();
            self.host_key.shuffle();
            self.crypt_client_to_server.shuffle();
            self.crypt_server_to_client.shuffle();
            self.mac_server_to_client.shuffle();
            self.mac_client_to_server.shuffle();
            self.compress_client_to_server.shuffle();
            self.compress_server_to_client.shuffle();
            self.signer.shuffle();
            self.disable_compat = rng.random();
            self.ext = rng.random();
            self.key_strict = rng.random();
        }
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Config {
        general: General,
        target: Target,
        pub authentication: Authentication,
    }

    impl Config {
        pub async fn load() -> anyhow::Result<Self> {
            let content = tokio::fs::read_to_string("./Test.toml").await?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        }

        pub async fn open_session_simple(&self) -> anyhow::Result<session::Session> {
            let session = self.open_session().await?;
            session.request_authentication().await?;
            let status = session
                .authenticate_password(
                    self.authentication.username.clone(),
                    self.authentication.password.clone(),
                )
                .await?;
            anyhow::ensure!(status.success(), "Failed to authenticate with password");
            Ok(session)
        }

        pub async fn connect(&self) -> anyhow::Result<TcpStream> {
            let tcp = TcpStream::connect((self.target.host.clone(), self.target.port)).await?;
            Ok(tcp)
        }

        pub async fn open_session(&self) -> anyhow::Result<session::Session> {
            let stream = self.connect().await?;

            let mut config = session::Config::default();

            let notifier = session::DefaultNotifier::default();

            if self.general.shuffle {
                config.shuffle();
            }

            tracing::info!("Using config: {:#?}", config);

            let session = session::Session::handshake(stream, config, notifier).await?;
            Ok(session)
        }

        pub async fn open_session_with_config(
            &self,
            config: session::Config,
        ) -> anyhow::Result<session::Session> {
            let stream = self.connect().await?;

            let notifier = session::DefaultNotifier::default();

            let session = session::Session::handshake(stream, config, notifier).await?;
            Ok(session)
        }

        pub async fn authenticate_password(
            &self,
            session: &session::Session,
        ) -> anyhow::Result<()> {
            let status = session
                .authenticate_password(
                    self.authentication.username.to_string(),
                    self.authentication.password.to_string(),
                )
                .await?;

            assert!(status.success());
            Ok(())
        }
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Target {
        host: String,
        port: u16,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Authentication {
        pub username: String,
        pub password: String,
        pub public_key: String,
        pub private_key: String,
        pub certificate: String,
        pub passphrase: Option<String>,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct General {
        shuffle: bool,
    }
}
