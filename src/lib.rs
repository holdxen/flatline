pub mod cipher;
pub mod error;
#[macro_use]
pub mod ssh;
pub mod key;
pub mod session;
mod stream;

const DEFAULT_CHANNEL_CAPACITY: usize = 256;


#[cfg(test)]
mod test {
    use rand::RngExt;
use serde::{Serialize, Deserialize};
    use tokio::net::TcpStream;
    use crate::session;
    use indexmap::IndexMap;

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
        pub authentication: Authentication
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
            let status = session.authenticate_password(self.authentication.username.clone(), self.authentication.password.clone()).await?;
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

        pub async fn open_session_with_config(&self, config: session::Config) -> anyhow::Result<session::Session> {
            let stream = self.connect().await?;

            let notifier = session::DefaultNotifier::default();

            let session = session::Session::handshake(stream, config, notifier).await?;
            Ok(session)
        }


        pub async fn authenticate_password(&self, session: &session::Session) -> anyhow::Result<()> {
            let status = session.authenticate_password(self.authentication.username.to_string(),
                                                       self.authentication.password.to_string()).await?;

            assert!(status.success());
            Ok(())
        }
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Target {
        host: String,
        port: u16
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