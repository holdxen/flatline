use std::fmt::Debug;

use super::cipher::hash::Hash;
use super::cipher::kex::Summary as DHSumary;
use super::cipher::Boxtory;
use super::error::{Error, Result};
use super::forward::Stream as ForwardStream;
use super::session::DisconnectReson;
use super::ssh::common::*;
use super::ssh::stream::{BufferStream, Stream};
use crate::cipher::compress::{self, Decode, Encode};
use crate::cipher::crypt::{self, Decrypt, Encrypt};
use crate::cipher::kex::{self, KeyExChange};
use crate::cipher::mac::{self, Mac};
use crate::cipher::sign::{self, Verify};
use crate::error::builder;
use crate::handshake::code::*;
use crate::project;
use crate::ssh::buffer::Buffer;
use derive_new::new;
use indexmap::IndexMap;
use openssl::rand::rand_bytes;
use snafu::ResultExt;
use tokio::io::{AsyncRead, AsyncWrite};

pub struct Config<B> {
    pub(crate) banner: String, //
    pub key_exchange: IndexMap<String, Boxtory<dyn KeyExChange + Send>>,
    pub hostkey: IndexMap<String, Boxtory<dyn Verify + Send>>,
    pub crypt_client_to_server: IndexMap<String, Boxtory<dyn Encrypt + Send>>,
    pub crypt_server_to_client: IndexMap<String, Boxtory<dyn Decrypt + Send>>,
    pub mac_client_to_server: IndexMap<String, Boxtory<dyn Mac + Send>>,
    pub mac_server_to_client: IndexMap<String, Boxtory<dyn Mac + Send>>,
    pub compress_client_to_server: IndexMap<String, Boxtory<dyn Encode + Send>>,
    pub compress_server_to_client: IndexMap<String, Boxtory<dyn Decode + Send>>,
    pub key_strict: bool,
    pub behavior: Option<B>,
    pub(crate) ext: bool,
}

impl<B> Debug for Config<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("banner", &self.banner)
            .finish()
    }
}

// #[cfg(not(feature = "async-compatible"))]
// pub trait Behavior {
//     fn openssh_hostkeys(
//         &mut self,
//         want_reply: bool,
//         hostkeys: &[&[u8]],
//     ) -> impl Future<Output = Result<()>> + Send;
//     fn debug(
//         &mut self,
//         always_display: bool,
//         msg: &str,
//         tag: &str,
//     ) -> impl Future<Output = Result<()>> + Send;
//     fn ignore(&mut self, data: &[u8]) -> impl Future<Output = Result<()>> + Send;
//     fn useauth_banner(&mut self, msg: &str, tag: &str) -> impl Future<Output = Result<()>> + Send;
//     fn disconnect(
//         &mut self,
//         reson: DisconnectReson,
//         dest: &str,
//         tag: &str,
//     ) -> impl Future<Output = Result<()>> + Send;
//     fn verify_server_hostkey(
//         &mut self,
//         keytype: &str,
//         hostkeys: &[u8],
//     ) -> impl Future<Output = Result<bool>> + Send;
// }

#[async_trait::async_trait]
pub trait Behavior {
    async fn openssh_hostkeys(&mut self, want_reply: bool, hostkeys: &[&[u8]]) -> Result<()>;
    async fn debug(&mut self, always_display: bool, msg: &str, tag: &str) -> Result<()>;
    async fn ignore(&mut self, data: &[u8]) -> Result<()>;
    async fn useauth_banner(&mut self, msg: &str, tag: &str) -> Result<()>;
    async fn disconnect(&mut self, reson: DisconnectReson, dest: &str, tag: &str) -> Result<()>;
    async fn verify_server_hostkey(&mut self, keytype: &str, hostkeys: &[u8]) -> Result<bool>;
    async fn server_signature_algorithms(&mut self, algorithms: &[&str]) -> Result<()>;
    async fn x11_forward(&mut self, stream: ForwardStream) -> Result<()>;
}

impl Config<DefaultBehavior> {
    pub fn deafult_with_behavior() -> Self {
        Self::default()
    }
}

#[derive(Default)]
pub struct DefaultBehavior;

// #[cfg(not(feature = "async-compatible"))]
// impl Behavior for DefaultBehavior {
//     async fn openssh_hostkeys(&mut self, _: bool, _: &[&[u8]]) -> Result<()> {
//         Ok(())
//     }

//     async fn debug(&mut self, _: bool, _: &str, _: &str) -> Result<()> {
//         Ok(())
//     }

//     async fn ignore(&mut self, _: &[u8]) -> Result<()> {
//         Ok(())
//     }

//     async fn disconnect(&mut self, _: DisconnectReson, _: &str, _: &str) -> Result<()> {
//         Ok(())
//     }

//     async fn verify_server_hostkey(&mut self, _: &str, _: &[u8]) -> Result<bool> {
//         Ok(true)
//     }

//     async fn useauth_banner(&mut self, _: &str, _: &str) -> Result<()> {
//         Ok(())
//     }
// }

#[async_trait::async_trait]
impl Behavior for DefaultBehavior {
    async fn openssh_hostkeys(&mut self, _: bool, _: &[&[u8]]) -> Result<()> {
        Ok(())
    }

    async fn debug(&mut self, _: bool, _: &str, _: &str) -> Result<()> {
        Ok(())
    }

    async fn ignore(&mut self, _: &[u8]) -> Result<()> {
        Ok(())
    }

    async fn useauth_banner(&mut self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }

    async fn disconnect(&mut self, _: DisconnectReson, _: &str, _: &str) -> Result<()> {
        Ok(())
    }

    async fn verify_server_hostkey(&mut self, _: &str, _: &[u8]) -> Result<bool> {
        Ok(true)
    }

    async fn server_signature_algorithms(&mut self, _: &[&str]) -> Result<()> {
        Ok(())
    }

    async fn x11_forward(&mut self, _: ForwardStream) -> Result<()> {
        Ok(())
    }
}

impl<B> Default for Config<B> {
    fn default() -> Self {
        fn convert<K: ToString, V>(value: IndexMap<K, V>) -> IndexMap<String, V> {
            value.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
        }

        let banner = format!(
            "SSH-2.0-{}_{}\r\n",
            project::PROJECT_NAME,
            project::PROJECT_VERSION
        );
        Self {
            banner,
            key_exchange: convert(kex::new_all()),
            hostkey: convert(sign::new_verify_all()),
            crypt_server_to_client: convert(crypt::new_decrypt_all()),
            crypt_client_to_server: convert(crypt::new_encrypt_all()),
            mac_client_to_server: convert(mac::new_all()),
            mac_server_to_client: convert(mac::new_all()),
            compress_client_to_server: convert(compress::new_encode_all()),
            compress_server_to_client: convert(compress::new_decode_all()),
            key_strict: true,
            behavior: None,
            ext: false,
        }
    }
}

impl<B> Config<B> {
    pub fn disable_compress(&mut self) {
        self.compress_client_to_server.clear();
        self.compress_server_to_client.clear();
        self.compress_client_to_server
            .insert("none".to_string(), compress::none_encode());
        self.compress_server_to_client
            .insert("none".to_string(), compress::none_decode());
    }

    pub fn new(behaviour: B) -> Self {
        fn convert<K: ToString, V>(value: IndexMap<K, V>) -> IndexMap<String, V> {
            value.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
        }

        let banner = format!(
            "SSH-2.0-{}_{}\r\n",
            project::PROJECT_NAME,
            project::PROJECT_VERSION
        );

        Self {
            banner,
            key_exchange: convert(kex::new_all()),
            hostkey: convert(sign::new_verify_all()),
            crypt_server_to_client: convert(crypt::new_decrypt_all()),
            crypt_client_to_server: convert(crypt::new_encrypt_all()),
            mac_client_to_server: convert(mac::new_all()),
            mac_server_to_client: convert(mac::new_all()),
            compress_client_to_server: convert(compress::new_encode_all()),
            compress_server_to_client: convert(compress::new_decode_all()),
            key_strict: true,
            behavior: Some(behaviour),
            ext: false,
        }
    }
}

pub(crate) async fn banner_exchange<T: AsyncWrite + AsyncRead + Unpin>(
    stream: &mut BufferStream<T>,
    banner: &str,
) -> Result<(String, Vec<String>)> {
    stream.write(banner.as_bytes()).await?;
    stream.flush().await?;

    let mut count = 0;
    let mut lines = vec![];
    const MAX: usize = 255;
    loop {
        let line = stream.read_line_crlf().await?;
        snafu::ensure!(
            count <= MAX,
            builder::BannerTooLong {
                tip: "server banner too long"
            }
        );
        // if count > MAX {
        //     return Err(Error::BannerExchange("server banner too long".to_string()));
        // }
        count += line.len();
        if line.starts_with(b"SSH-2.0") || line.starts_with(b"SSH-1.99") {
            // self.server_info.banner = Some(line);
            return Ok((String::from_utf8(line).map_err(|e| e.utf8_error())?, lines));
        } else if line.starts_with(b"SSH-") {
            // anyhow::bail!("server doesn't support ssh2");
            // return Err(Error::Ssh2Unsupport);
            return builder::Ssh2Unsupport.fail();
        }
        lines.push(String::from_utf8(line).map_err(|e| e.utf8_error())?);
    }
}

#[derive(new)]
pub(crate) struct Summary {
    pub binary: Vec<u8>,
    pub methods: Methods,
}

#[allow(clippy::too_many_arguments)]
#[derive(new, Debug)]
pub(crate) struct Methods {
    pub kex: Vec<String>,
    pub host_key: Vec<String>,
    pub en_client_to_server: Vec<String>,
    pub en_server_to_client: Vec<String>,
    pub mac_client_to_server: Vec<String>,
    pub mac_server_to_client: Vec<String>,
    pub com_client_to_server: Vec<String>,
    pub com_server_to_client: Vec<String>,
    pub lang_client_to_server: Vec<String>,
    pub lang_server_to_client: Vec<String>,
    pub kex_strict: bool,
    pub ext: bool,
}

impl Methods {
    fn from_config<B: Behavior>(config: &Config<B>) -> Self {
        fn convert(methods: impl IntoIterator<Item = impl ToString>) -> Vec<String> {
            methods.into_iter().map(|v| v.to_string()).collect()
        }
        let lang: [&str; 0] = [];
        Self::new(
            convert(config.key_exchange.keys()),
            convert(config.hostkey.keys()),
            convert(config.crypt_client_to_server.keys()),
            convert(config.crypt_server_to_client.keys()),
            convert(config.mac_client_to_server.keys()),
            convert(config.mac_server_to_client.keys()),
            convert(config.compress_client_to_server.keys()),
            convert(config.compress_server_to_client.keys()),
            convert(lang),
            convert(lang),
            config.key_strict,
            config.ext,
        )
    }
}

#[derive(new)]
pub(crate) struct MethodExchange {
    pub client: Summary,
    pub server: Summary,
    // algo: Algorithm,
}

pub(crate) async fn method_exchange<B: Behavior>(
    stream: &mut dyn Stream,
    config: &Config<B>,
) -> Result<MethodExchange> {
    let invalid_arg = |str: &str| builder::InvalidArgument { tip: str }.fail();
    if config.compress_client_to_server.is_empty() {
        return invalid_arg(
            "Compress client to server is empty, 'none' should be provided at least",
        );
    }

    if config.compress_server_to_client.is_empty() {
        return invalid_arg(
            "Compress_server_to_client is empty, 'none' should be provided at least",
        );
    }

    if config.crypt_client_to_server.is_empty() {
        return invalid_arg("Crypt client to server is empty");
    }

    if config.crypt_server_to_client.is_empty() {
        return invalid_arg("Crypt server to client is empty");
    }

    if config.mac_client_to_server.is_empty() {
        return invalid_arg("Mac client to server is empty");
    }

    if config.mac_server_to_client.is_empty() {
        return invalid_arg("Mac server to client is empty");
    }

    if config.hostkey.is_empty() {
        return invalid_arg("Hostkey is empty");
    }

    if config.key_exchange.is_empty() {
        return invalid_arg("Key exhange is empty");
    }

    let client_methods = Methods::from_config(config);

    let mut kex = client_methods.kex.clone();

    if client_methods.kex_strict {
        kex.push(KEX_STRICT_CLIENT.to_string());
    }

    if client_methods.ext {
        kex.push(EXT_INFO_CLIENT.to_string());
    }

    let mut randbytes = [0; 16];

    rand_bytes(&mut randbytes).context(builder::Openssl)?;

    let mut buffer = Buffer::new();
    buffer.put_u8(SSH_MSG_KEXINIT);
    buffer.put_bytes(randbytes);
    buffer.put_one(kex.join(","));
    buffer.put_one(client_methods.host_key.join(","));
    buffer.put_one(client_methods.en_client_to_server.join(","));
    buffer.put_one(client_methods.en_server_to_client.join(","));
    buffer.put_one(client_methods.mac_client_to_server.join(","));
    buffer.put_one(client_methods.mac_server_to_client.join(","));
    buffer.put_one(client_methods.com_client_to_server.join(","));
    buffer.put_one(client_methods.com_server_to_client.join(","));
    buffer.put_one(client_methods.lang_client_to_server.join(","));
    buffer.put_one(client_methods.lang_server_to_client.join(","));

    buffer.put_u8(0); // ssh.first_kex_packet_follows
    buffer.put_bytes([0; 4]); // ssh.kex.reserved

    stream.send_payload(buffer.as_ref()).await?;

    let reply = stream.recv_packet().await?;

    if reply.payload.is_empty() || reply.payload[0] != SSH_MSG_KEXINIT {
        // return Err(Error::ProtocolError(
        //     "Failed to receive kex msg".to_string(),
        // ));
        return builder::Protocol {
            tip: "Failed to receive kex msg",
        }
        .fail();
    }

    let parser = || {
        let reply = Buffer::from_slice(&reply.payload);

        reply.take_u8()?;

        reply.take_bytes(16)?;

        let get = || {
            let (_, methods) = reply.take_one()?;
            let methods = std::str::from_utf8(methods).ok()?;

            Some(
                methods
                    .split(',')
                    .map(|v| v.to_string())
                    .collect::<Vec<String>>(),
            )
        };
        let mut kex = get()?;

        let mut kex_strict = false;
        let mut ext = false;
        if let Some(index) = kex.iter().position(|v| v == KEX_STRICT_SERVER) {
            kex.remove(index);
            kex_strict = true;
        };

        if let Some(index) = kex.iter().position(|v| v == EXT_INFO_SERVER) {
            kex.remove(index);
            ext = true;
        };

        let methods = Methods::new(
            kex,
            get()?,
            get()?,
            get()?,
            get()?,
            get()?,
            get()?,
            get()?,
            get()?,
            get()?,
            kex_strict,
            ext,
        );

        let _ = reply.take_u8()?;
        let _ = reply.take_bytes(4)?;

        Some(methods)
    };

    let server_methods = parser().ok_or(Error::invalid_format("Invalid packet"))?;

    let client = Summary::new(buffer.into_vec(), client_methods);
    let server = Summary::new(reply.payload, server_methods);

    Ok(MethodExchange::new(client, server))
}

pub(crate) async fn method_exchange_with_payload<B: Behavior>(
    stream: &mut dyn Stream,
    bytes: &[u8],
    config: &Config<B>,
) -> Result<MethodExchange> {
    let invalid_arg = |str: &str| builder::InvalidArgument { tip: str }.fail();
    if config.compress_client_to_server.is_empty() {
        return invalid_arg(
            "Compress client to server is empty, 'none' should be provided at least",
        );
    }

    if config.compress_server_to_client.is_empty() {
        return invalid_arg(
            "Compress_server_to_client is empty, 'none' should be provided at least",
        );
    }

    if config.crypt_client_to_server.is_empty() {
        return invalid_arg("Crypt client to server is empty");
    }

    if config.crypt_server_to_client.is_empty() {
        return invalid_arg("Crypt server to client is empty");
    }

    if config.mac_client_to_server.is_empty() {
        return invalid_arg("Mac client to server is empty");
    }

    if config.mac_server_to_client.is_empty() {
        return invalid_arg("Mac server to client is empty");
    }

    if config.hostkey.is_empty() {
        return invalid_arg("Hostkey is empty");
    }

    if config.key_exchange.is_empty() {
        return invalid_arg("Key exhange is empty");
    }

    let client_methods = Methods::from_config(config);

    let mut kex = client_methods.kex.clone();

    if client_methods.kex_strict {
        kex.push(KEX_STRICT_CLIENT.to_string());
    }

    if client_methods.ext {
        kex.push(EXT_INFO_CLIENT.to_string());
    }

    let mut randbytes = [0; 16];

    rand_bytes(&mut randbytes).context(builder::Openssl)?;

    let mut buffer = Buffer::new();
    buffer.put_u8(SSH_MSG_KEXINIT);
    buffer.put_bytes(randbytes);
    buffer.put_one(kex.join(","));
    buffer.put_one(client_methods.host_key.join(","));
    buffer.put_one(client_methods.en_client_to_server.join(","));
    buffer.put_one(client_methods.en_server_to_client.join(","));
    buffer.put_one(client_methods.mac_client_to_server.join(","));
    buffer.put_one(client_methods.mac_server_to_client.join(","));
    buffer.put_one(client_methods.com_client_to_server.join(","));
    buffer.put_one(client_methods.com_server_to_client.join(","));
    buffer.put_one(client_methods.lang_client_to_server.join(","));
    buffer.put_one(client_methods.lang_server_to_client.join(","));

    buffer.put_u8(0); // ssh.first_kex_packet_follows
    buffer.put_bytes([0; 4]); // ssh.kex.reserved

    stream.send_payload(buffer.as_ref()).await?;

    // let reply = stream.recv_packet().await?;

    if bytes.is_empty() || bytes[0] != SSH_MSG_KEXINIT {
        // return Err(Error::ProtocolError(
        //     "Failed to receive kex msg".to_string(),
        // ));
        return builder::Protocol {
            tip: "Failed to receive kex msg",
        }
        .fail();
    }

    let parser = || {
        let reply = Buffer::from_slice(bytes);

        reply.take_u8()?;

        reply.take_bytes(16)?;

        let get = || {
            let (_, methods) = reply.take_one()?;
            let methods = std::str::from_utf8(methods).ok()?;

            Some(
                methods
                    .split(',')
                    .map(|v| v.to_string())
                    .collect::<Vec<String>>(),
            )
        };
        let mut kex = get()?;

        let mut kex_strict = false;
        let mut ext = false;
        if let Some(index) = kex.iter().position(|v| v == KEX_STRICT_SERVER) {
            kex.remove(index);
            kex_strict = true;
        };

        if let Some(index) = kex.iter().position(|v| v == EXT_INFO_SERVER) {
            kex.remove(index);
            ext = true;
        };

        let methods = Methods::new(
            kex,
            get()?,
            get()?,
            get()?,
            get()?,
            get()?,
            get()?,
            get()?,
            get()?,
            get()?,
            kex_strict,
            ext,
        );

        let _ = reply.take_u8()?;
        let _ = reply.take_bytes(4)?;

        Some(methods)
    };

    let server_methods = parser().ok_or(Error::invalid_format("Invalid packet"))?;

    let client = Summary::new(buffer.into_vec(), client_methods);
    let server = Summary::new(bytes.to_vec(), server_methods);

    Ok(MethodExchange::new(client, server))
}

#[allow(clippy::too_many_arguments)]
#[derive(new)]
pub(crate) struct MatchMethod {
    pub kex: Box<dyn KeyExChange + Send>,
    pub hostkey: Box<dyn Verify + Send>,
    pub server_crypt: Box<dyn Decrypt + Send>,
    pub client_crypt: Box<dyn Encrypt + Send>,
    pub server_mac: Box<dyn Mac + Send>,
    pub client_mac: Box<dyn Mac + Send>,
    pub server_compress: Box<dyn Decode + Send>,
    pub client_compress: Box<dyn Encode + Send>,
}

impl MatchMethod {
    pub(crate) fn initialize(&mut self, result: &mut DHSumary) -> Result<()> {
        let secret_key = Buffer::from_one(&result.secret_key);

        let local_iv = calculate(
            &mut result.hash,
            secret_key.as_ref(),
            &result.session_id,
            &result.client_hash,
            b'A',
            self.client_crypt.iv_len(),
        )?;

        let local_key = calculate(
            &mut result.hash,
            secret_key.as_ref(),
            &result.session_id,
            &result.client_hash,
            b'C',
            self.client_crypt.key_len(),
        )?;

        self.client_crypt.initialize(&local_iv, &local_key)?;

        let remote_iv = calculate(
            &mut result.hash,
            secret_key.as_ref(),
            &result.session_id,
            &result.client_hash,
            b'B',
            self.server_crypt.iv_len(),
        )?;
        let remote_key = calculate(
            &mut result.hash,
            secret_key.as_ref(),
            &result.session_id,
            &result.client_hash,
            b'D',
            self.server_crypt.key_len(),
        )?;

        self.server_crypt.initialize(&remote_iv, &remote_key)?;

        let local_key = calculate(
            &mut result.hash,
            secret_key.as_ref(),
            &result.session_id,
            &result.client_hash,
            b'E',
            self.client_mac.key_len(),
        )?;

        self.client_mac.initialize(&local_key)?;

        let remote_key = calculate(
            &mut result.hash,
            secret_key.as_ref(),
            &result.session_id,
            &result.client_hash,
            b'F',
            self.server_mac.key_len(),
        )?;

        self.server_mac.initialize(&remote_key)?;

        Ok(())
    }
}

// todo: ignore mac when cipher has tag
pub(crate) fn match_method<B: Behavior>(
    client: &Methods,
    server: &Methods,
    config: &Config<B>,
) -> Result<MatchMethod> {
    let mut kex = None;
    let mut hostkey = None;
    let mut server_crypt = None;
    let mut client_crypt = None;
    let mut server_compress = None;
    let mut client_compress = None;
    for i in &client.kex {
        if server.kex.contains(i) {
            kex = config.key_exchange.get(i);
            break;
        }
    }

    for i in &client.host_key {
        if server.host_key.contains(i) {
            hostkey = config.hostkey.get(i);
            break;
        }
    }

    for i in &client.en_client_to_server {
        if server.en_client_to_server.contains(i) {
            client_crypt = config.crypt_client_to_server.get(i);
            break;
        }
    }

    for i in &client.en_server_to_client {
        if server.en_server_to_client.contains(i) {
            server_crypt = config.crypt_server_to_client.get(i);
            break;
        }
    }

    // for i in &client.mac_client_to_server {
    //     if server.mac_client_to_server.contains(i) {
    //         client_mac = config.mac_client_to_server.get(i);
    //         break;
    //     }
    // }

    // for i in &client.mac_server_to_client {
    //     if server.mac_server_to_client.contains(i) {
    //         server_mac = config.mac_server_to_client.get(i);
    //         break;
    //     }
    // }

    for i in &client.com_client_to_server {
        if server.com_client_to_server.contains(i) {
            client_compress = config.compress_client_to_server.get(i);
            break;
        }
    }

    for i in &client.com_server_to_client {
        if server.com_server_to_client.contains(i) {
            server_compress = config.compress_server_to_client.get(i);
            break;
        }
    }

    /*
    Ok(Algorithm::new(
            kex.create(),
            hostkey.create(),
            server_crypt.create(),
            client_crypt.create(),
            server_mac.create(),
            client_mac.create(),
            server_compress.create(),
            client_compress.create(),
        ))
     */
    match (
        kex,
        hostkey,
        server_crypt,
        client_crypt,
        server_compress,
        client_compress,
    ) {
        (
            Some(kex),
            Some(hostkey),
            Some(server_crypt),
            Some(client_crypt),
            Some(server_compress),
            Some(client_compress),
        ) => {
            let mut server_mac = None;
            let mut client_mac = None;
            let server_crypt = server_crypt.create();
            if server_crypt.has_tag() {
                server_mac = Some(mac::none().create());
            } else {
                for i in &client.mac_server_to_client {
                    if server.mac_server_to_client.contains(i) {
                        server_mac = config.mac_server_to_client.get(i).map(|v| v.create());
                        break;
                    }
                }
            }

            let client_crypt = client_crypt.create();

            if client_crypt.has_tag() {
                client_mac = Some(mac::none().create());
            } else {
                for i in &client.mac_client_to_server {
                    if server.mac_client_to_server.contains(i) {
                        client_mac = config.mac_server_to_client.get(i).map(|v| v.create());
                        break;
                    }
                }
            }

            match (client_mac, server_mac) {
                (Some(client_mac), Some(server_mac)) => Ok(MatchMethod::new(
                    kex.create(),
                    hostkey.create(),
                    server_crypt,
                    client_crypt,
                    server_mac,
                    client_mac,
                    server_compress.create(),
                    client_compress.create(),
                )),
                _ => builder::NegotiationFailed.fail(), //Err(Error::NegotiationFailed),
            }
        }
        _ => builder::NegotiationFailed.fail(), //Err(Error::NegotiationFailed),
                                                // _ => Err(Error::NegotiationFailed),
    }
}

pub(crate) async fn new_keys(stream: &mut dyn Stream) -> Result<()> {
    stream.send_new_keys().await?;
    let packet = stream.recv_packet().await?;
    if packet.payload[0] != SSH_MSG_NEWKEYS {
        // Err(Error::ProtocolError(
        //     "Failed to receive new keys".to_string(),
        // ))
        builder::Protocol {
            tip: "Failed to receive new keys",
        }
        .fail()
    } else {
        Ok(())
    }
}

fn calculate(
    hash: &mut Box<dyn Hash + Send>,
    key: &[u8],
    session_id: &[u8],
    h: &[u8],
    version: u8,
    len: usize,
) -> Result<Vec<u8>> {
    let mut out = vec![];

    hash.update(key)?;
    hash.update(h)?;
    hash.update(&[version])?;
    hash.update(session_id)?;

    let tmp = hash.finalize()?;

    out.extend(tmp);

    while out.len() < len {
        hash.update(key)?;
        hash.update(h)?;
        hash.update(&out)?;

        let tmp = hash.finalize()?;

        out.extend(tmp);
    }

    out.truncate(len);

    Ok(out)
}
