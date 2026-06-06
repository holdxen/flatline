use flatline::handshake::Config;
use flatline::session::Session;
use flatline::session::Userauth;
use flatline::sftp::OpenFlags;
use flatline::sftp::Permissions;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

include!("./user.conf");

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let socket = TcpStream::connect(HOST).await.unwrap();
    let config = Config::default_with_behavior();
    let session = Session::handshake(config, socket).await.unwrap();

    let status = session.userauth_password(USERNAME, PASSWORD).await.unwrap();

    assert!(matches!(status, Userauth::Success));

    let mut sftp = session.sftp_open_default().await.unwrap();

    let mut remote_file = sftp
        .open_file(
            "/tmp/README.md",
            OpenFlags::WRITE | OpenFlags::TRUNC | OpenFlags::CREAT,
            Some(Permissions::p0755()),
        )
        .await
        .unwrap();

    let mut local_file = fs::File::open("./README.md").await.unwrap();

    let mut buf = Vec::with_capacity(1024);
    loop {
        let size = local_file.read_buf(&mut buf).await.unwrap();
        if size == 0 {
            break;
        }
        sftp.write_file(&mut remote_file, &buf[..size])
            .await
            .unwrap();
        buf.clear();
    }

    sftp.close_file(remote_file).await.unwrap();

    sftp.close().await.unwrap();

    session.disconnect_default().await.unwrap();
}
