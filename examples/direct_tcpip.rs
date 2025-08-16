use flatline::forward::SocketAddr;
use flatline::handshake::Config;
use flatline::session::Session;
use flatline::session::Userauth;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::watch;

include!("./user.conf");

async fn copy(first: &mut TcpStream, second: &mut flatline::forward::Stream) {
    let mut buf = Vec::with_capacity(512);
    loop {
        tokio::select! {
            res = first.read_buf(&mut buf) => {
                let size = res.unwrap();
                if size == 0 {
                    break;
                }
                second.write(buf[..size].to_vec()).await.unwrap();
                buf.clear();
            }
            res = second.read() => {
                let data = res.unwrap();

                first.write_all(&data).await.unwrap();
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let socket = TcpStream::connect(HOST).await.unwrap();
    let config = Config::deafult_with_behavior();
    let session = Session::handshake(config, socket).await.unwrap();

    let private = fs::read(PRI_KEY_FILE).await.unwrap();
    let public = fs::read(PUB_KEY_FILE).await.unwrap();
    let status = session
        .userauth_publickey_from_file(USERNAME, private, Some(&public), None)
        .await
        .unwrap();

    assert!(matches!(status, Userauth::Success));

    let listener = TcpListener::bind("127.0.0.1:5000").await.unwrap();

    println!("press enter to exit");
    let mut stdin = tokio::io::stdin();
    let mut handles = vec![];
    let (sender, recver) = watch::channel(());
    loop {
        tokio::select! {
            res = listener.accept() => {
                let mut local = res.unwrap().0;
                let local_addr = local.peer_addr().unwrap();
                let mut remote = session
                    .direct_tcpip_default(
                        SocketAddr::new("127.0.0.1".to_string(), 5000),
                        SocketAddr::new(local_addr.ip().to_string(), local_addr.port() as u32),
                    )
                    .await
                    .unwrap();
                    let mut recver = recver.clone();
                handles.push(tokio::spawn(async move {
                    tokio::select! {
                        _ = recver.changed() => {

                        }
                        _ = copy(&mut local, &mut remote) => {

                        }
                    };

                    remote.close().await.unwrap();

                }));

            }

            _ = stdin.read_u8() => {
                let _ = sender.send(());
                break;
            }
        }
    }
    for i in handles {
        tokio::join!(i).0.unwrap();
    }
    session.disconnect_default().await.unwrap();
}
