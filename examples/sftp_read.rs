use flatline::handshake::Config;
use flatline::session::Session;
use flatline::session::Userauth;
use flatline::sftp::OpenFlags;
use tokio::fs;
use tokio::io::AsyncWriteExt;
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
        .open_file("/etc/ssh/sshd_config", OpenFlags::READ, None)
        .await
        .unwrap();

    let mut local_file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open("./sshd_config")
        .await
        .unwrap();

    loop {
        let data = sftp.read_file(&mut remote_file, 1024).await.unwrap();
        if data.is_empty() {
            break;
        }

        local_file.write_all(&data).await.unwrap();
    }

    sftp.close_file(remote_file).await.unwrap();

    sftp.close().await.unwrap();

    session.disconnect_default().await.unwrap();
}
