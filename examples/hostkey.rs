use flatline::handshake::Config;
use flatline::session::Session;
use flatline::session::Userauth;
use tokio::fs;
use tokio::net::TcpStream;

include!("./user.conf");

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let socket = TcpStream::connect(HOST).await.unwrap();
    let config = Config::default_with_behavior();
    let session = Session::handshake(config, socket).await.unwrap();

    let private = fs::read(PRI_KEY_FILE).await.unwrap();
    let public = fs::read(PUB_KEY_FILE).await.unwrap();
    let status = session
        .userauth_publickey_from_file(USERNAME, private, Some(&public), None)
        .await
        .unwrap();

    assert!(matches!(status, Userauth::Success));

    session.disconnect_default().await.unwrap();
}
