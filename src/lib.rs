mod cipher;
mod error;
#[macro_use]
pub mod ssh;
#[macro_use]
mod log;
mod key;
pub mod session;
mod stream;

const DEFAULT_CHANNEL_CAPACITY: usize = 256;


#[cfg(test)]
mod test {
    use serde::{Serialize, Deserialize};
    use tokio::net::TcpStream;
    use crate::session;

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Config {
        target: Target,
        authentication: Authentication
    }

    impl Config {
        pub async fn load() -> anyhow::Result<Self> {
            let content = tokio::fs::read_to_string("./Test.toml").await?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        }


        pub async fn connect(&self) -> anyhow::Result<TcpStream> {
            let tcp = TcpStream::connect((self.target.host.clone(), self.target.port)).await?;
            Ok(tcp)
        }

        pub async fn open_session(&self) -> anyhow::Result<session::Session> {
            let stream = self.connect().await?;

            let config = session::Config::default();

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

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Target {
        host: String,
        port: u16
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Authentication {
        username: String,
        password: String,
        public_key: String,
        private_key: String,
        certificate: String,
    }
}