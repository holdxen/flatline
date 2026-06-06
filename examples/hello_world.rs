use flatline::channel::ExitStatus;
use flatline::handshake::Config;
use flatline::session::Session;
use flatline::session::Userauth;
use tokio::net::TcpStream;

include!("./user.conf");

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let socket = TcpStream::connect(HOST).await.unwrap();
    let config = Config::default_with_behavior();
    let session = Session::handshake(config, socket).await.unwrap();

    let status = session.userauth_password(USERNAME, PASSWORD).await.unwrap();

    assert!(matches!(status, Userauth::Success));

    let mut channel = session.channel_open_default().await.unwrap();
    channel.exec("echo \"hello\"").await.unwrap();
    loop {
        let msg = channel.recv().await.unwrap();
        match msg {
            flatline::channel::Message::Close => break,
            flatline::channel::Message::Eof => break,
            flatline::channel::Message::Stdout(data) => assert_eq!(data, b"hello\n"),
            flatline::channel::Message::Stderr(_) => unreachable!(),
            flatline::channel::Message::Exit(status) => {
                assert!(matches!(status, ExitStatus::Normal(0)))
            }
        }
    }
}
