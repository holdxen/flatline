use flatline::async_trait;
use flatline::error::Result;
use flatline::handshake::Config;
use flatline::session::Interactive;
use flatline::session::Session;
use tokio::io::stdin;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

include!("./user.conf");

/// # add to /etc/ssh/sshd_config and restart ssh-server if you were using openssh
/// KbdInteractiveAuthentication yes

pub struct Keyboard {}

#[async_trait]
impl Interactive for Keyboard {
    async fn response(
        &mut self,
        _: &str,
        _: &str,
        prompts: &[(&str, bool)],
    ) -> Result<Vec<String>> {
        let mut ret = vec![];

        for (prompt, _) in prompts {
            println!("{}", prompt);
            let mut input = Vec::with_capacity(1024);
            let len = stdin().read_buf(&mut input).await?;
            ret.push(std::str::from_utf8(&input[..len])?.trim_end().to_string());
        }

        Ok(ret)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let socket = TcpStream::connect(HOST).await.unwrap();
    let config = Config::default_with_behavior();
    let session = Session::handshake(config, socket).await.unwrap();

    let res = session
        .userauth_keyboard_interactive("zhou", [] as [&str; 0], Keyboard {})
        .await
        .unwrap();

    assert!(res);

    session.disconnect_default().await.unwrap();
}
