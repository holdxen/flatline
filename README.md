# flatline

[![Crates.io](https://img.shields.io/crates/v/flatline.svg)](https://crates.io/crates/flatline)
[![License](https://img.shields.io/crates/l/flatline.svg)](LICENSE)

An async SSH-2.0 client library for Rust.

## Features

- Fully async implementation built on Tokio
- Comprehensive algorithm support
- SCP and SFTP support
- TCP/IP forwarding

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
flatline = "0.1"
```

## Quick Start

```rust
use flatline::session::{Session, Config, DefaultNotifier};
use flatline::session::channel::Message;
use tokio::net::TcpStream;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = TcpStream::connect("example.com:22").await?;
    let config = Config::default();
    let notifier = DefaultNotifier::default();

    let session = Session::handshake(socket, config, notifier).await?;

    session.request_authentication().await?;
    let status = session.authenticate_password("user", "password").await?;
    assert!(status.success());

    let mut channel = session.channel_open_default().await?;
    channel.request_exec(true, "echo hello").await?;

    while let Ok(msg) = channel.receive().await {
        match msg {
            Message::Stdout(data) => {
                println!("{}", String::from_utf8_lossy(&data));
            }
            Message::Exit(_) | Message::Close => break,
            _ => {}
        }
    }

    Ok(())
}
```

## Supported Algorithms

| Category | Algorithms |
|----------|------------|
| **Key Exchange** | mlkem768x25519-sha256, curve25519-sha256, ecdh-sha2-nistp256/384/521, diffie-hellman-group14/16/18 |
| **Host Key** | ssh-ed25519, rsa-sha2-256/512, ecdsa-sha2-nistp256/384/521 |
| **Encryption** | chacha20-poly1305, aes256-gcm, aes128-gcm, aes256/192/128-ctr/cbc |
| **MAC** | hmac-sha2-512/256, hmac-sha1, hmac-md5 (with ETM variants) |
| **Compression** | zlib, zlib@openssh.com |

## License

This project is licensed under the [MPL-2.0 License](LICENSE).
