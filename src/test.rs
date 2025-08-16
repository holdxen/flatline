use indexmap::IndexMap;
use rand::thread_rng;
use rand::Rng;
use tokio::net::TcpStream;

use crate::handshake::Behavior;
use crate::handshake::Config;
use crate::handshake::DefaultBehavior;
use crate::keys;
use crate::scp;
use crate::session::Session;
use crate::session::Userauth;
use crate::sftp::Permissions;

// const IP: &str = "127.0.0.1:22";
const IP: &str = "192.168.8.116:22";
const USER: &str = "zhou";
const PASS: &str = "123456";

async fn open_session<B: Behavior + Send + Sync + 'static>(config: Config<B>) -> Session {
    let socket = TcpStream::connect(IP).await.unwrap();
    let session = Session::handshake(config, socket).await.unwrap();

    let status = session.userauth_password(USER, PASS).await.unwrap();

    assert!(matches!(status, Userauth::Success));

    session
}

#[tokio::test]
async fn userauth_publickey() {
    let openssh = keys::KeyParser::default();

    let private_key_file = tokio::fs::read("/home/zhou/.ssh/id_ecdsa256")
        .await
        .unwrap();
    let public_key_file = tokio::fs::read("/home/zhou/.ssh/id_ecdsa256.pub")
        .await
        .unwrap();

    let key = openssh
        .parse_privatekey(&private_key_file, Some(b"123456"))
        .unwrap();
    let _ = openssh.parse_publickey(&public_key_file).unwrap();

    let config: Config<DefaultBehavior> = Config::default();

    let session = Session::handshake(config, TcpStream::connect(IP).await.unwrap())
        .await
        .unwrap();

    let status = session
        .userauth_publickey(USER, key.key_type, key.public_key, key.private_key)
        .await
        .unwrap();

    println!("{:?}", status);
}

#[tokio::test]
async fn random() {
    for _ in 0..100 {
        echo_hello(rand_config::<DefaultBehavior>(), 10).await
    }
}

fn rand_config<B>() -> Config<B> {
    let mut config = Config::default();

    rand_map(&mut config.compress_client_to_server);
    rand_map(&mut config.compress_server_to_client);
    rand_map(&mut config.crypt_client_to_server);
    rand_map(&mut config.crypt_server_to_client);
    rand_map(&mut config.hostkey);
    rand_map(&mut config.key_exchange);
    rand_map(&mut config.mac_client_to_server);
    rand_map(&mut config.mac_server_to_client);

    config.key_strict = thread_rng().gen();
    config
}

fn rand_map<K, V>(map: &mut IndexMap<K, V>) {
    map.sort_by_cached_key(|_, _| thread_rng().gen::<usize>());
}

// #[tokio::test]
// async fn open_sftp() {
//     let session = open_session::<DefaultBehavior>(Default::default()).await;

//     let mut sftp = session.sftp_open_default().await.unwrap();

//     let dir = sftp.open_dir("/usr/lib/aarch64-linux-gnu").await.unwrap();

//     let path = sftp.realpath("./Documents").await.unwrap();

//     println!("real path: {}", path);

//     loop {
//         let infos = sftp.read_dir(&dir).await.unwrap();

//         if infos.is_empty() {
//             break;
//         }
//     }

//     let channel = session.channel_open_default().await.unwrap();

//     let content = "123456789\n";
//     let mut sender = scp::Sender::from_channel(
//         channel,
//         "./Documents/test1.scp.txt",
//         content.len() as u64,
//         Permissions::p0755(),
//         None,
//         false,
//     )
//     .await
//     .unwrap();

//     sender.send(content).await.unwrap();

//     sender.finish().await.unwrap();
// }

async fn echo_hello<B: Behavior + Send + Sync + 'static>(config: Config<B>, times: usize) {
    let socket = TcpStream::connect(IP).await.unwrap();
    let session = Session::handshake(config, socket).await.unwrap();

    let status = session.userauth_password(USER, PASS).await.unwrap();

    assert!(matches!(status, Userauth::Success));

    for _ in 0..times {
        let mut channel = session.channel_open_default().await.unwrap();
        channel.exec("echo \"hello\"").await.unwrap();

        loop {
            let msg = channel.recv().await.unwrap();
            match msg {
                crate::channel::Message::Close => break,
                crate::channel::Message::Eof => break,
                crate::channel::Message::Stdout(data) => {
                    println!("{}", String::from_utf8(data).unwrap());
                }
                crate::channel::Message::Stderr(data) => {
                    println!("{}", String::from_utf8(data).unwrap());
                }
                crate::channel::Message::Exit(e) => {
                    println!("exit: {e:?}",)
                }
            }
        }
        // let status = channel.exec_and_wait("echo \"hello\"").await.unwrap();
        // assert!(matches!(status, crate::msg::ExitStatus::Normal(0)));
        // let buf: Vec<u8> = channel.read().await.unwrap();
        // channel.close().await.unwrap();
        // assert_eq!(buf, b"hello\n");
    }

    session.disconnect_default().await.unwrap();
    // channel.close().await.unwrap();
}
