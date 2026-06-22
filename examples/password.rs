use flatline::handshake::Config;
use flatline::session::Session;
use flatline::session::Userauth;
use tokio::net::TcpStream;

mod common;
use common::*;

#[tokio::main(flavor = "current_thread")]
async fn main() -> flatline::error::Result<()> {
    let socket = TcpStream::connect(HOST).await?;
    let config = Config::default_with_behavior();
    let session = Session::handshake(config, socket).await?;

    let status = session.userauth_password(USERNAME, PASSWORD).await?;

    assert!(matches!(status, Userauth::Success));

    session.disconnect_default().await?;
    Ok(())
}
