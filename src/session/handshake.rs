use std::fmt;
use std::str::{Utf8Error, from_utf8};

use indexmap::IndexMap;
use rand::RngExt;
use snafu::ResultExt;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::cipher::signature::Signature;
use crate::session::Notifier;
use crate::{
    cipher::{
        Factory,
        compress::{self, Decode, Encode},
        crypt::{self, Decrypt, Encrypt},
        kex::{self, Information, KeyExchange},
        mac::{self, Mac},
        signature::{self, Verify},
    },
    error::{self, builder},
    ssh::{
        buffer::{Consumer, Producer},
        msg::{self, DisconnectReason, Message},
        protocol::{self},
        stream::{CipherStream, PlainStream, Stream},
    },
    stream::BufferStream,
};

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("Banner too long"))]
    BannerTooLong,
    #[snafu(display("Unsupported SSH version: {}", version))]
    UnsupportedVersion { version: String },
    #[snafu(display("Invalid banner: {}", source))]
    InvalidBanner { source: Utf8Error },
    #[snafu(display("Invalid string: {}", source))]
    InvalidString { source: Utf8Error },
    #[snafu(display("Negotiation failed"))]
    NegotiationFailed,
    #[snafu(display("Signature verification failed"))]
    SignatureVerificationFailed,
    #[snafu(display("Server host key rejected by user"))]
    ServerHostKeyRejectedByUser,
    #[snafu(display("Unexpected message in strict mode: code {}", code))]
    UnexpectedMessageInStrictMode { code: u8 },
    #[snafu(display("Unexpected disconnect message: {:?} - {}", reason, description))]
    UnexpectedDisconnectMessage {
        reason: DisconnectReason,
        description: String,
    },
    #[snafu(display("Unexpected server banner: {}", banner))]
    UnexpectedServerBanner { banner: String },
}

pub struct Handshaker<T, N>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
    N: Notifier + Send + 'static,
{
    pub client_version: String,
    pub banner: Vec<String>,
    pub config: Config,
    pub notifier: N,
    pub socket: Option<BufferStream<T>>,
    pub plain_stream: Option<PlainStream<T>>,
    pub cipher_stream: Option<CipherStream<T>>,
    pub server_version: Option<String>,
    pub server_banner: Option<Vec<String>>,
    pub server_kex_msg: Option<Vec<u8>>,
    pub client_kex_msg: Option<Vec<u8>>,
    pub matched_methods: Option<MatchedMethods>,
    pub compat_options: CompatOptions,
    pub session_id: Option<Vec<u8>>,
}

pub struct Config {
    pub kex: IndexMap<String, Factory<dyn KeyExchange + Send>>,
    pub host_key: IndexMap<String, Factory<dyn Verify + Send>>,
    pub crypt_client_to_server: IndexMap<String, Factory<dyn Encrypt + Send>>,
    pub crypt_server_to_client: IndexMap<String, Factory<dyn Decrypt + Send>>,
    pub mac_client_to_server: IndexMap<String, Factory<dyn Mac + Send>>,
    pub mac_server_to_client: IndexMap<String, Factory<dyn Mac + Send>>,
    pub compress_client_to_server: IndexMap<String, Factory<dyn Encode + Send>>,
    pub compress_server_to_client: IndexMap<String, Factory<dyn Decode + Send>>,
    pub signer: IndexMap<String, Factory<dyn Signature + Send>>,
    pub key_strict: bool,
    pub ext: bool,
    pub disable_compat: bool,
}

struct IndexMapKeys<'a, K, V, S>(&'a IndexMap<K, V, S>);

impl<K, V, S> fmt::Debug for IndexMapKeys<'_, K, V, S>
where
    K: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.keys()).finish()
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("kex", &IndexMapKeys(&self.kex))
            .field("host_key", &IndexMapKeys(&self.host_key))
            .field(
                "crypt_client_to_server",
                &IndexMapKeys(&self.crypt_client_to_server),
            )
            .field(
                "crypt_server_to_client",
                &IndexMapKeys(&self.crypt_server_to_client),
            )
            .field(
                "mac_client_to_server",
                &IndexMapKeys(&self.mac_client_to_server),
            )
            .field(
                "mac_server_to_client",
                &IndexMapKeys(&self.mac_server_to_client),
            )
            .field(
                "compress_client_to_server",
                &IndexMapKeys(&self.compress_client_to_server),
            )
            .field(
                "compress_server_to_client",
                &IndexMapKeys(&self.compress_server_to_client),
            )
            .field("signer", &IndexMapKeys(&self.signer))
            .field("key_strict", &self.key_strict)
            .field("ext", &self.ext)
            .field("disable_compat", &self.disable_compat)
            .finish()
    }
}

#[derive(derive_more::Debug)]
pub struct MatchedMethods {
    #[debug("{}", kex.name())]
    kex: Box<dyn KeyExchange + Send>,
    #[debug("{}", host_key.name())]
    host_key: Box<dyn Verify + Send>,
    #[debug("{}", crypt_client_to_server.name())]
    crypt_client_to_server: Box<dyn Encrypt + Send>,
    #[debug("{}", crypt_server_to_client.name())]
    crypt_server_to_client: Box<dyn Decrypt + Send>,
    #[debug("{}", mac_client_to_server.name())]
    mac_client_to_server: Box<dyn Mac + Send>,
    #[debug("{}", mac_server_to_client.name())]
    mac_server_to_client: Box<dyn Mac + Send>,
    #[debug("{}", compress_client_to_server.name())]
    compress_client_to_server: Box<dyn Encode + Send>,
    #[debug("{}", compress_server_to_client.name())]
    compress_server_to_client: Box<dyn Decode + Send>,
}

impl MatchedMethods {
    fn initialize(
        &mut self,
        hash: &[u8],
        session_id: &[u8],
        secret_key: &[u8],
    ) -> error::Result<()> {
        let local_iv = self.kex.compute_communicate_key(
            secret_key,
            session_id,
            hash,
            b'A',
            self.crypt_client_to_server.iv_len(),
        )?;

        let local_key = self.kex.compute_communicate_key(
            secret_key,
            session_id,
            hash,
            b'C',
            self.crypt_client_to_server.key_len(),
        )?;

        self.crypt_client_to_server
            .initialize(&local_iv, &local_key)?;

        let remote_iv = self.kex.compute_communicate_key(
            secret_key,
            session_id,
            hash,
            b'B',
            self.crypt_server_to_client.iv_len(),
        )?;
        let remote_key = self.kex.compute_communicate_key(
            secret_key,
            session_id,
            hash,
            b'D',
            self.crypt_server_to_client.key_len(),
        )?;

        self.crypt_server_to_client
            .initialize(&remote_iv, &remote_key)?;
        let local_key = self.kex.compute_communicate_key(
            secret_key,
            session_id,
            hash,
            b'E',
            self.mac_client_to_server.key_len(),
        )?;
        let remote_key = self.kex.compute_communicate_key(
            secret_key,
            session_id,
            hash,
            b'F',
            self.mac_server_to_client.key_len(),
        )?;
        self.mac_client_to_server.initialize(&local_key)?;
        self.mac_server_to_client.initialize(&remote_key)?;

        Ok(())
    }
}

impl Config {
    fn negotiate(&self, server: &Methods) -> error::Result<MatchedMethods> {
        let mut kex = None;
        for (k, v) in self.kex.iter() {
            if server.kex.contains(k) {
                kex = Some(v());
                break;
            }
        }
        let Some(kex) = kex else {
            return Err(NegotiationFailedSnafu.build().into());
        };

        let mut host_key = None;
        for (k, v) in self.host_key.iter() {
            if server.host_key.contains(k) {
                host_key = Some(v());
                break;
            }
        }
        let Some(host_key) = host_key else {
            return Err(NegotiationFailedSnafu.build().into());
        };

        let mut crypt_client_to_server = None;
        for (k, v) in self.crypt_client_to_server.iter() {
            if server.crypt_client_to_server.contains(k) {
                crypt_client_to_server = Some(v());
                break;
            }
        }
        let Some(crypt_client_to_server) = crypt_client_to_server else {
            return Err(NegotiationFailedSnafu.build().into());
        };

        let mut crypt_server_to_client = None;
        for (k, v) in self.crypt_server_to_client.iter() {
            if server.crypt_server_to_client.contains(k) {
                crypt_server_to_client = Some(v());
                break;
            }
        }
        let Some(crypt_server_to_client) = crypt_server_to_client else {
            return Err(NegotiationFailedSnafu.build().into());
        };

        let mut mac_client_to_server = None;
        for (k, v) in self.mac_client_to_server.iter() {
            if server.mac_client_to_server.contains(k) {
                mac_client_to_server = Some(v());
                break;
            }
        }
        let Some(mac_client_to_server) = mac_client_to_server else {
            return Err(NegotiationFailedSnafu.build().into());
        };

        let mut mac_server_to_client = None;
        for (k, v) in self.mac_server_to_client.iter() {
            if server.mac_server_to_client.contains(k) {
                mac_server_to_client = Some(v());
                break;
            }
        }
        let Some(mac_server_to_client) = mac_server_to_client else {
            return Err(NegotiationFailedSnafu.build().into());
        };
        let mut compress_client_to_server = None;
        for (k, v) in self.compress_client_to_server.iter() {
            if server.compress_client_to_server.contains(k) {
                compress_client_to_server = Some(v());
                break;
            }
        }
        let Some(compress_client_to_server) = compress_client_to_server else {
            return Err(NegotiationFailedSnafu.build().into());
        };

        let mut compress_server_to_client = None;
        for (k, v) in self.compress_server_to_client.iter() {
            if server.compress_server_to_client.contains(k) {
                compress_server_to_client = Some(v());
                break;
            }
        }
        let Some(compress_server_to_client) = compress_server_to_client else {
            return Err(NegotiationFailedSnafu.build().into());
        };

        Ok(MatchedMethods {
            kex,
            host_key,
            crypt_client_to_server,
            crypt_server_to_client,
            mac_client_to_server,
            mac_server_to_client,
            compress_client_to_server,
            compress_server_to_client,
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        fn convert<K: ToString, V>(value: IndexMap<K, V>) -> IndexMap<String, V> {
            value.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
        }

        // let version = format!(
        //     "SSH-2.0-{}_{}",
        //     env!("CARGO_PKG_NAME"),
        //     env!("CARGO_PKG_VERSION")
        // );
        Self {
            // version,
            // banner: vec![],
            kex: convert(kex::new_all()),
            host_key: convert(signature::new_verify_all()),
            crypt_server_to_client: convert(crypt::new_decrypt_all()),
            crypt_client_to_server: convert(crypt::new_encrypt_all()),
            mac_client_to_server: convert(mac::new_all()),
            mac_server_to_client: convert(mac::new_all()),
            compress_client_to_server: convert(compress::new_encode_all()),
            compress_server_to_client: convert(compress::new_decode_all()),
            signer: convert(signature::new_signature_all()),
            key_strict: true,
            ext: true,
            disable_compat: false,
        }
    }
}

#[derive(Default, Clone, Copy)]
pub(super) struct CompatOptions {
    pub unsupported_rekey: bool,
    pub curve25519_pad: bool,
    pub specify_server_sign_algorithm: bool,
    pub old_session_id: bool,
    pub limited_dh_ex: bool,
}

impl CompatOptions {
    fn parse(version_suffix: &str) -> Self {
        let mut options = CompatOptions::default();

        if version_suffix.starts_with("Sun_SSH_1.0") {
            options.unsupported_rekey = true;
        }

        if version_suffix.starts_with("OpenSSH_6.5") || version_suffix.starts_with("OpenSSH_6.6") {
            options.curve25519_pad = true;
        }

        if version_suffix.starts_with("OpenSSH_7.4") {
            options.specify_server_sign_algorithm = true;
        }

        if version_suffix.starts_with("3.0 SecureCRT") || version_suffix.starts_with("1.7 SecureFX")
        {
            options.old_session_id = true;
        }

        if version_suffix.starts_with("Cisco-1.") {
            options.limited_dh_ex = true;
        }

        options
    }
}

impl<T, N> Handshaker<T, N>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
    N: Notifier + Send + 'static,
{
    pub fn new(socket: T, notifier: N, config: Config) -> Self {
        let client_version = format!(
            "SSH-2.0-{}_{}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        );
        Self {
            client_version,
            banner: vec![],
            config,
            notifier,
            socket: Some(BufferStream::new(socket)),
            plain_stream: None,
            cipher_stream: None,
            server_version: None,
            server_banner: None,
            matched_methods: None,
            client_kex_msg: None,
            server_kex_msg: None,
            compat_options: Default::default(),
            session_id: None,
        }
    }
    pub async fn banner_version_exchange(&mut self) -> error::Result<()> {
        use regex::Regex;
        let re = Regex::new(
            r"^SSH-(?P<version>[0-9]+(?:\.[0-9]+)*)-(?P<software>[!-~]+)(?: (?P<comment>[ -~]*))?\r?\n?$"
        ).expect("Invalid reggular expression");
        let stream = self.socket.as_mut().unwrap();

        if !self.banner.is_empty() {
            let banner = self.banner.join(protocol::BANNER_ENDING);
            stream.put_bytes(banner);
            stream.put_bytes(protocol::BANNER_ENDING);
        }
        stream.put_bytes(&self.client_version);
        stream.put_bytes(protocol::BANNER_ENDING);
        stream.flush().await.context(builder::IO)?;
        // let mut banner = self.config.banner.join(protocol::BANNER_ENDING);
        // banner.push_str(protocol::BANNER_ENDING);
        // banner.push_str(&self.config.version);
        // banner.push_str(protocol::BANNER_ENDING);

        // stream.write_all(&banner).await.context(builder::IO)?;
        // stream
        //     .write(self.config.version.as_bytes())
        //     .await
        //     .context(builder::IO)?;

        let mut count = 0;
        let mut lines = vec![];
        const MAX: usize = 255;
        loop {
            snafu::ensure!(count < MAX, BannerTooLongSnafu);
            let line = stream
                .read_line_crlf(MAX - count)
                .await
                .context(builder::IO)?;
            // if count > MAX {
            //     return Err(Error::BannerExchange("server banner too long".to_string()));
            // }
            count += line.len();
            // let line = String::from_utf8(line).context(InvalidBannerSnafu)?;
            let line = std::str::from_utf8(&line)
                .context(InvalidBannerSnafu)?
                .trim_end_matches(protocol::BANNER_ENDING);
            // .to_string();

            // let line_no_ending = line.trim_end_matches(protocol::BANNER_ENDING);

            if line.starts_with("SSH-") {
                return if let Some(caps) = re.captures(line) {
                    let version = &caps["version"];
                    if version != "2.0" && version != "1.99" {
                        return Err(UnsupportedVersionSnafu {
                            version: line.to_string(),
                        }
                        .build()
                        .into());
                    }
                    let software = &caps["software"];
                    let comment = caps.name("comment").map(|v| v.as_str());
                    tracing::info!(
                        "Server banner: version={}, software={}, comment={:?}",
                        version,
                        software,
                        comment
                    );

                    if let Some(comment) = comment {
                        self.compat_options =
                            CompatOptions::parse(format!("{} {}", software, comment).as_str());
                    } else {
                        self.compat_options = CompatOptions::parse(software);
                    }

                    self.server_version = Some(line.to_string());
                    self.server_banner = Some(lines);

                    let mut plain_stream = PlainStream::new(self.socket.take().unwrap());

                    plain_stream.client_mut().ext = self.config.ext;
                    plain_stream.client_mut().kex_strict = self.config.key_strict;
                    self.plain_stream = Some(plain_stream);
                    Ok(())
                } else {
                    Err(UnexpectedServerBannerSnafu { banner: line }.build().into())
                };
            }

            lines.push(line.to_string());

            // if line.starts_with("SSH-2.0") || line.starts_with("SSH-1.99") {
            //     self.server_version = Some(line);
            //     self.server_banner = Some(lines);

            //     let mut plain_stream = PlainStream::new(self.socket.take().unwrap());

            //     plain_stream.client_mut().ext = self.config.ext;
            //     plain_stream.client_mut().kex_strict = self.config.key_strict;
            //     self.plain_stream = Some(plain_stream);
            //     return Ok(());
            // } else if line.starts_with("SSH-") {
            //     return Err(UnsupportedVersionSnafu { version: line }.build().into());
            // }
            // lines.push(line);
        }
    }

    pub async fn negotiate_methods(&mut self) -> error::Result<()> {
        let mut client_methods = Methods::from_config(&self.config);
        client_methods.do_compat(!self.config.disable_compat && self.compat_options.curve25519_pad);
        tracing::info!("Client methods: {:?}", client_methods);

        let v = client_methods.build();

        {
            let stream = self.plain_stream.as_mut().unwrap();

            stream.send_payload(&v).await?;
            self.client_kex_msg = Some(v);
        }

        loop {
            let packet = self.plain_stream.as_mut().unwrap().recv_packet().await?;
            let mut consumer = Consumer::new(&packet.payload);
            if consumer.consume_u8()? == protocol::SSH_MSG_KEXINIT {
                let server_method = Methods::parse(&packet.payload)?;

                tracing::info!("Server methods: {:?}", server_method);

                self.plain_stream.as_mut().unwrap().server_mut().kex_strict =
                    server_method.kex_strict;
                self.plain_stream.as_mut().unwrap().server_mut().ext = server_method.ext;
                let matched = self.config.negotiate(&server_method)?;

                tracing::info!("Matched methods: {:?}", matched);

                self.matched_methods = Some(matched);
                self.server_kex_msg = Some(packet.payload);
                break;
            } else {
                self.plain_stream
                    .as_mut()
                    .unwrap()
                    .handle_msg(packet)
                    .await?;
            }
        }

        Ok(())
    }

    pub async fn key_exchange(&mut self) -> error::Result<()> {
        let matched = self.matched_methods.as_mut().unwrap();

        if let Some(exchange) = matched.kex.exchange() {
            let need = matched
                .crypt_client_to_server
                .key_len()
                .max(matched.crypt_client_to_server.block_size())
                .max(matched.crypt_client_to_server.iv_len())
                .max(matched.mac_client_to_server.key_len())
                .max(matched.crypt_server_to_client.key_len())
                .max(matched.crypt_server_to_client.block_size())
                .max(matched.crypt_server_to_client.iv_len())
                .max(matched.mac_server_to_client.key_len());

            let need = need * 8;
            let mut bits = if need <= 112 {
                2048
            } else if need <= 128 {
                3072
            } else if need <= 192 {
                7680
            } else {
                8192
            };

            if !self.config.disable_compat && self.compat_options.limited_dh_ex && bits > 4096 {
                bits = 4096;
            }

            exchange.set_recommended_number_of_bits(bits);

            let mut producer = Producer::default();
            producer.put_u8(exchange.request_code());
            producer.put_u32(exchange.min());
            producer.put_u32(exchange.number_of_bits());
            producer.put_u32(exchange.max());

            self.plain_stream
                .as_mut()
                .unwrap()
                .send_payload(&producer[..])
                .await?;

            loop {
                let packet = self.plain_stream.as_mut().unwrap().recv_packet().await?;
                let mut consumer = Consumer::new(&packet.payload);
                if consumer.consume_u8()? == exchange.response_code() {
                    let p = consumer.consume_one()?;
                    let g = consumer.consume_one()?;
                    exchange.initialize(p, g)?;
                    break;
                } else {
                    self.plain_stream
                        .as_mut()
                        .unwrap()
                        .handle_msg(packet)
                        .await?;
                }
            }
        }

        let client_public_key = matched.kex.generate_key()?;
        let mut producer = Producer::default();
        producer.put_u8(matched.kex.request_code());
        producer.put_one(&client_public_key);

        self.plain_stream
            .as_mut()
            .unwrap()
            .send_payload(&producer[..])
            .await?;

        loop {
            let packet = self.plain_stream.as_mut().unwrap().recv_packet().await?;
            let mut consumer = Consumer::new(&packet.payload);
            if consumer.consume_u8()? == matched.kex.response_code() {
                let host_key = consumer.consume_one()?;
                let server_public_key = consumer.consume_one()?;
                let signature = consumer.consume_one()?;

                let secret_key = matched.kex.compute_secret_key(server_public_key)?;

                let info = Information {
                    client_version: &self.client_version,
                    server_version: self.server_version.as_ref().unwrap(),
                    client_kex_init: self.client_kex_msg.as_ref().unwrap(),
                    server_kex_init: self.server_kex_msg.as_ref().unwrap(),
                    server_host_key: host_key,
                    client_public_key: &client_public_key,
                    server_public_key,
                    secret_key: &secret_key,
                };

                let hash = matched.kex.compute_hash(info)?;

                tracing::info!("Using host_key algorithm: {}", matched.host_key.name());
                matched.host_key.initialize(host_key)?;

                let res = matched.host_key.verify(signature, &hash)?;

                snafu::ensure!(res, SignatureVerificationFailedSnafu);

                if !self
                    .notifier
                    .verify_server_host_key(matched.host_key.name(), host_key)
                    .await
                {
                    return Err(ServerHostKeyRejectedByUserSnafu.build().into());
                }

                let secret_key = {
                    let mut p = Producer::default();
                    p.put_one(secret_key);
                    p
                };

                matched.initialize(&hash, &hash, &secret_key[..])?;

                self.session_id = Some(hash);

                self.plain_stream
                    .as_mut()
                    .unwrap()
                    .send_payload(&[protocol::SSH_MSG_NEWKEYS])
                    .await?;

                loop {
                    let packet = self.plain_stream.as_mut().unwrap().recv_packet().await?;
                    let mut consomuer = Consumer::new(&packet.payload);
                    if consomuer.consume_u8()? == protocol::SSH_MSG_NEWKEYS {
                        break;
                    } else {
                        // we have sent new keys, so if we try to handle this message from server, we should use cipher stream to send message to server and use plain stream to receive message.
                        // But we are not ready, so we have to ignore the message during new keys.
                        tracing::warn!("Ignore message: {}", packet.payload[0]);
                        // self.plain_stream.as_mut().unwrap().handle_msg(packet).await?;
                    }
                }

                let matched = self.matched_methods.take().unwrap();
                let cipher_stream = self.plain_stream.take().unwrap().upgrade(
                    matched.crypt_client_to_server,
                    matched.crypt_server_to_client,
                    matched.compress_client_to_server,
                    matched.compress_server_to_client,
                    matched.mac_client_to_server,
                    matched.mac_server_to_client,
                );

                self.cipher_stream = Some(cipher_stream);

                break Ok(());
            } else {
                self.plain_stream
                    .as_mut()
                    .unwrap()
                    .handle_msg(packet)
                    .await?;
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct Methods {
    pub kex: Vec<String>,
    pub host_key: Vec<String>,
    pub crypt_client_to_server: Vec<String>,
    pub crypt_server_to_client: Vec<String>,
    pub mac_client_to_server: Vec<String>,
    pub mac_server_to_client: Vec<String>,
    pub compress_client_to_server: Vec<String>,
    pub compress_server_to_client: Vec<String>,
    pub lang_client_to_server: Vec<String>,
    pub lang_server_to_client: Vec<String>,
    pub kex_strict: bool,
    pub ext: bool,
}

impl Methods {
    fn new(
        kex: Vec<String>,
        host_key: Vec<String>,
        en_client_to_server: Vec<String>,
        en_server_to_client: Vec<String>,
        mac_client_to_server: Vec<String>,
        mac_server_to_client: Vec<String>,
        com_client_to_server: Vec<String>,
        com_server_to_client: Vec<String>,
        lang_client_to_server: Vec<String>,
        lang_server_to_client: Vec<String>,
        kex_strict: bool,
        ext: bool,
    ) -> Self {
        Self {
            kex,
            host_key,
            crypt_client_to_server: en_client_to_server,
            crypt_server_to_client: en_server_to_client,
            mac_client_to_server,
            mac_server_to_client,
            compress_client_to_server: com_client_to_server,
            compress_server_to_client: com_server_to_client,
            lang_client_to_server,
            lang_server_to_client,
            kex_strict,
            ext,
        }
    }

    fn from_config(config: &Config) -> Self {
        fn convert(methods: impl IntoIterator<Item = impl ToString>) -> Vec<String> {
            methods.into_iter().map(|v| v.to_string()).collect()
        }
        let lang: [&str; 0] = [];
        Self::new(
            convert(config.kex.keys()),
            convert(config.host_key.keys()),
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
    fn parse(data: &[u8]) -> error::Result<Self> {
        let mut consumer = Consumer::new(data);
        if consumer.consume_u8()? != protocol::SSH_MSG_KEXINIT {
            return builder::InvalidOperation {
                detail: "expected SSH_MSG_KEXINIT",
            }
            .fail();
        }

        consumer.consume_bytes(16)?;

        let kex = consumer.consume_one()?;
        let kex = from_utf8(kex).context(InvalidStringSnafu)?;
        let mut kex: Vec<String> = kex.split(',').map(|s| s.to_string()).collect();

        let mut kex_strict = false;
        let mut ext = false;
        if let Some(index) = kex.iter().position(|v| v == protocol::KEX_STRICT_SERVER) {
            kex.remove(index);
            kex_strict = true;
        };

        if let Some(index) = kex.iter().position(|v| v == protocol::EXT_INFO_SERVER) {
            kex.remove(index);
            ext = true;
        };

        let host_key = consumer.consume_one()?;
        let host_key = from_utf8(host_key).context(InvalidStringSnafu)?;
        let host_key: Vec<String> = host_key.split(',').map(|s| s.to_string()).collect();

        let en_client_to_server = consumer.consume_one()?;
        let en_client_to_server = from_utf8(en_client_to_server).context(InvalidStringSnafu)?;
        let en_client_to_server: Vec<String> = en_client_to_server
            .split(',')
            .map(|s| s.to_string())
            .collect();

        let en_server_to_client = consumer.consume_one()?;
        let en_server_to_client = from_utf8(en_server_to_client).context(InvalidStringSnafu)?;
        let en_server_to_client: Vec<String> = en_server_to_client
            .split(',')
            .map(|s| s.to_string())
            .collect();

        let mac_client_to_server = consumer.consume_one()?;
        let mac_client_to_server = from_utf8(mac_client_to_server).context(InvalidStringSnafu)?;
        let mac_client_to_server: Vec<String> = mac_client_to_server
            .split(',')
            .map(|s| s.to_string())
            .collect();

        let mac_server_to_client = consumer.consume_one()?;
        let mac_server_to_client = from_utf8(mac_server_to_client).context(InvalidStringSnafu)?;
        let mac_server_to_client: Vec<String> = mac_server_to_client
            .split(',')
            .map(|s| s.to_string())
            .collect();

        let com_client_to_server = consumer.consume_one()?;
        let com_client_to_server = from_utf8(com_client_to_server).context(InvalidStringSnafu)?;
        let com_client_to_server: Vec<String> = com_client_to_server
            .split(',')
            .map(|s| s.to_string())
            .collect();

        let com_server_to_client = consumer.consume_one()?;
        let com_server_to_client = from_utf8(com_server_to_client).context(InvalidStringSnafu)?;
        let com_server_to_client: Vec<String> = com_server_to_client
            .split(',')
            .map(|s| s.to_string())
            .collect();

        let lang_client_to_server = consumer.consume_one()?;
        let lang_client_to_server = from_utf8(lang_client_to_server).context(InvalidStringSnafu)?;
        let lang_client_to_server: Vec<String> = lang_client_to_server
            .split(',')
            .map(|s| s.to_string())
            .collect();

        let lang_server_to_client = consumer.consume_one()?;
        let lang_server_to_client = from_utf8(lang_server_to_client).context(InvalidStringSnafu)?;
        let lang_server_to_client: Vec<String> = lang_server_to_client
            .split(',')
            .map(|s| s.to_string())
            .collect();

        consumer.consume_u8()?;
        consumer.consume_bytes(4)?; // check all zero

        Ok(Self {
            kex,
            host_key,
            crypt_client_to_server: en_client_to_server,
            crypt_server_to_client: en_server_to_client,
            mac_client_to_server,
            mac_server_to_client,
            compress_client_to_server: com_client_to_server,
            compress_server_to_client: com_server_to_client,
            lang_client_to_server,
            lang_server_to_client,
            kex_strict,
            ext,
        })
    }

    fn build(&self) -> Vec<u8> {
        let mut producer = Producer::default();
        producer.put_u8(protocol::SSH_MSG_KEXINIT);

        producer.resize(17, 0);

        {
            let mut rng = rand::rng();
            rng.fill(&mut producer[1..]);
        }

        {
            let mut kex = self.kex.clone();

            if self.kex_strict {
                kex.push(protocol::KEX_STRICT_CLIENT.to_string());
            }
            if self.ext {
                kex.push(protocol::EXT_INFO_CLIENT.to_string());
            }
            producer.put_one(kex.join(","));
        }

        // producer.put_one(self.kex.join(","));
        producer.put_one(self.host_key.join(","));
        producer.put_one(self.crypt_client_to_server.join(","));
        producer.put_one(self.crypt_server_to_client.join(","));
        producer.put_one(self.mac_client_to_server.join(","));
        producer.put_one(self.mac_server_to_client.join(","));
        producer.put_one(self.compress_client_to_server.join(","));
        producer.put_one(self.compress_server_to_client.join(","));
        producer.put_one(self.lang_client_to_server.join(","));
        producer.put_one(self.lang_server_to_client.join(","));

        producer.put_u8(0); // ssh.first_kex_packet_follows
        producer.put_bytes([0; 4]); // ssh.kex.reserved

        producer.into_vec()
    }

    fn do_compat(&mut self, compat: bool) {
        if !compat {
            return;
        }
        let method = "curve25519-sha256@libssh.org";
        tracing::info!("Remove {}", method);
        if let Some(index) = self.kex.iter().position(|i| i == method) {
            self.kex.remove(index);
        }
    }
}

#[easy_ext::ext]
impl<T: AsyncRead + AsyncWrite + Unpin + Send> PlainStream<T> {
    async fn handle_msg(&mut self, packet: msg::Packet) -> error::Result<()> {
        // let stream = self.plain_stream.as_mut().unwrap();
        let stream = self;
        if stream.server_mut().kex_strict && stream.client_mut().kex_strict {
            return Err(UnexpectedMessageInStrictModeSnafu {
                code: packet.payload[0],
            }
            .build()
            .into());
        }
        let msg = Message::parse(&packet.payload)?;
        tracing::info!("Handling message on handshake: {:?}", msg);
        match msg {
            Message::Debug { .. } => return Ok(()),
            Message::Ignore { .. } => return Ok(()),
            Message::Disconnect {
                reason,
                description,
                language,
            } => {
                tracing::info!(
                    "Server disconnect: reason: {:?}, description: {}, language: {}",
                    reason,
                    description,
                    language
                );
                return Err(UnexpectedDisconnectMessageSnafu {
                    reason,
                    description,
                }
                .build()
                .into());
            }
            Message::Unimplemented { .. } => return Ok(()),
            Message::Ping { .. } => return Ok(()),
            Message::Pong { .. } => return Ok(()),
            _ => {}
        }
        let sequence_number = stream.server_mut().sequence_number;
        let buffer = make_buffer_without_header! {
            u8: protocol::SSH_MSG_UNIMPLEMENTED,
            u32: sequence_number,
        };
        stream.send_payload(&buffer[..]).await?;
        Ok(())
    }
}

pub(super) struct RekeyExchange<'a, T, N>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
    N: super::Notifier,
{
    session: &'a mut super::SessionInner<T, N>,
    client_kex_msg: Option<Vec<u8>>,
    matched_methods: Option<MatchedMethods>,
    server_kex_msg: Option<Vec<u8>>,
}

impl<'a, T, N> RekeyExchange<'a, T, N>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
    N: super::Notifier + Send,
{
    pub fn new(session: &'a mut super::SessionInner<T, N>) -> Self {
        Self {
            session,
            client_kex_msg: None,
            matched_methods: None,
            server_kex_msg: None,
        }
    }

    pub async fn exec(&mut self, server_kex_msg: Option<Vec<u8>>) -> error::Result<()> {
        self.negotiate_methods(server_kex_msg).await?;
        self.key_exchange().await?;
        Ok(())
    }

    pub async fn negotiate_methods(
        &mut self,
        server_kex_msg: Option<Vec<u8>>,
    ) -> error::Result<()> {
        let mut client_methods = Methods::from_config(self.session.config());

        client_methods.do_compat(
            !self.session.config().disable_compat && self.session.compat_options().curve25519_pad,
        );

        tracing::info!("Client methods: {:?}", client_methods);

        {
            let v = client_methods.build();
            self.session.socket_mut().send_payload(&v).await?;
            self.client_kex_msg = Some(v);
        }
        if let Some(payload) = server_kex_msg {
            assert_eq!(payload[0], protocol::SSH_MSG_KEXDH_INIT);

            let server_method = Methods::parse(&payload[1..])?;
            let matched = self.session.config().negotiate(&server_method)?;

            tracing::info!("Matched methods: {:?}", matched);

            self.matched_methods = Some(matched);
            self.server_kex_msg = Some(payload);
        } else {
            loop {
                let packet = self.session.socket_mut().recv_packet().await?;
                let mut consumer = Consumer::new(&packet.payload);
                if consumer.consume_u8()? == protocol::SSH_MSG_KEXINIT {
                    let server_method = Methods::parse(&packet.payload)?;

                    tracing::info!("Server methods: {:?}", server_method);

                    let matched = self.session.config().negotiate(&server_method)?;

                    tracing::info!("Matched methods: {:?}", matched);

                    self.matched_methods = Some(matched);
                    self.server_kex_msg = Some(packet.payload);
                    break;
                } else {
                    let message = Message::parse(&packet.payload)?;

                    self.session.handle_msg(message).await?;
                }
            }
        }

        Ok(())
    }

    pub async fn key_exchange(&mut self) -> error::Result<()> {
        let mut matched = self.matched_methods.take().unwrap();

        if let Some(exchange) = matched.kex.exchange() {
            let need = matched
                .crypt_client_to_server
                .key_len()
                .max(matched.crypt_client_to_server.block_size())
                .max(matched.crypt_client_to_server.iv_len())
                .max(matched.mac_client_to_server.key_len())
                .max(matched.crypt_server_to_client.key_len())
                .max(matched.crypt_server_to_client.block_size())
                .max(matched.crypt_server_to_client.iv_len())
                .max(matched.mac_server_to_client.key_len());

            let need = need * 8;
            let mut bits = if need <= 112 {
                2048
            } else if need <= 128 {
                3072
            } else if need <= 192 {
                7680
            } else {
                8192
            };

            if self.session.compat_options().limited_dh_ex
                && !self.session.config().disable_compat
                && bits > 4096
            {
                bits = 4096;
            }

            exchange.set_recommended_number_of_bits(bits);

            let mut producer = Producer::default();
            producer.put_u8(exchange.request_code());
            producer.put_u32(exchange.min());
            producer.put_u32(exchange.number_of_bits());
            producer.put_u32(exchange.max());

            self.session
                .socket_mut()
                .send_payload(&producer[..])
                .await?;

            loop {
                let packet = self.session.socket_mut().recv_packet().await?;
                let mut consumer = Consumer::new(&packet.payload);
                if consumer.consume_u8()? == exchange.response_code() {
                    let p = consumer.consume_one()?;
                    let g = consumer.consume_one()?;
                    exchange.initialize(p, g)?;
                    break;
                } else {
                    let msg = Message::parse(&packet.payload)?;
                    self.session.handle_msg(msg).await?;
                }
            }
        }

        let client_public_key = matched.kex.generate_key()?;
        let mut producer = Producer::default();
        producer.put_u8(matched.kex.request_code());
        producer.put_one(&client_public_key);

        self.session
            .socket_mut()
            .send_payload(&producer[..])
            .await?;

        loop {
            let packet = self.session.socket_mut().recv_packet().await?;
            let mut consumer = Consumer::new(&packet.payload);
            if consumer.consume_u8()? == matched.kex.response_code() {
                let host_key = consumer.consume_one()?;
                let server_public_key = consumer.consume_one()?;
                let signature = consumer.consume_one()?;

                let secret_key = matched.kex.compute_secret_key(server_public_key)?;

                let info = Information {
                    client_version: self.session.client_version(),
                    server_version: self.session.server_version(),
                    client_kex_init: self.client_kex_msg.as_ref().unwrap(),
                    server_kex_init: self.server_kex_msg.as_ref().unwrap(),
                    server_host_key: host_key,
                    client_public_key: &client_public_key,
                    server_public_key,
                    secret_key: &secret_key,
                };

                let hash = matched.kex.compute_hash(info)?;

                tracing::info!("Using host_key algorithm: {}", matched.host_key.name());

                matched.host_key.initialize(host_key)?;

                let res = matched.host_key.verify(signature, &hash)?;

                snafu::ensure!(res, SignatureVerificationFailedSnafu);

                if !self
                    .session
                    .notifier_mut()
                    .verify_server_host_key(matched.host_key.name(), host_key)
                    .await
                {
                    return Err(ServerHostKeyRejectedByUserSnafu.build().into());
                }

                let secret_key = {
                    let mut p = Producer::default();
                    p.put_one(secret_key);
                    p
                };
                // secret_key.put_one(&secret_key[..]);

                // let local_iv = calculate(
                //     &mut result.hash,
                //     secret_key.as_ref(),
                //     &result.session_id,
                //     &result.client_hash,
                //     b'A',
                //     self.client_crypt.iv_len(),
                // )?;

                matched.initialize(&hash, self.session.session_id(), &secret_key[..])?;

                self.session
                    .socket_mut()
                    .send_payload(&[protocol::SSH_MSG_NEWKEYS])
                    .await?;

                self.session.socket_mut().upgrade_client(
                    matched.crypt_client_to_server,
                    matched.compress_client_to_server,
                    matched.mac_client_to_server,
                );

                loop {
                    let packet = self.session.socket_mut().recv_packet().await?;
                    let mut consomuer = Consumer::new(&packet.payload);
                    if consomuer.consume_u8()? == protocol::SSH_MSG_NEWKEYS {
                        self.session.socket_mut().upgrade_server(
                            matched.crypt_server_to_client,
                            matched.compress_server_to_client,
                            matched.mac_server_to_client,
                        );
                        break;
                    } else {
                        let msg = Message::parse(&packet.payload)?;
                        self.session.handle_msg(msg).await?;
                    }
                }

                break Ok(());
            } else {
                let msg = Message::parse(&packet.payload)?;
                self.session.handle_msg(msg).await?;
            }
        }
    }
}

// pub struct DecryptNothing;

// impl Decrypt for DecryptNothing {
//     fn name(&self) -> &str {
//         "nothing"
//     }

//     fn iv_len(&self) -> usize {
//         0
//     }

//     fn key_len(&self) -> usize {
//         0
//     }

//     fn block_size(&self) -> usize {
//         8
//     }

//     fn initialize(&mut self, _: &[u8], _: &[u8]) -> error::Result<()> {
//         Ok(())
//     }

//     fn update(&mut self, data: &[u8], out: &mut Vec<u8>) -> error::Result<usize> {
//         out.extend_from_slice(data);
//         Ok(data.len())
//     }

//     fn finalize(&mut self, _: &mut Vec<u8>) -> error::Result<usize> {
//         Ok(0)
//     }

//     fn is_galois_counter_mode(&self) -> bool {
//         false
//     }

//     fn tag_len(&self) -> usize {
//         0
//     }

//     fn update_sequence_number(&mut self, _: u32) -> error::Result<()> {
//         Ok(())
//     }

//     fn additional_authenticated_data(&mut self, _: &mut [u8]) -> error::Result<()> {
//         Ok(())
//     }

//     fn authentication_tag(&mut self, _: &[u8]) -> error::Result<()> {
//         Ok(())
//     }
// }

// struct EncryptNothing;

// impl Encrypt for EncryptNothing {
//     fn name(&self) -> &str {
//         "nothing"
//     }

//     fn iv_len(&self) -> usize {
//         0
//     }

//     fn key_len(&self) -> usize {
//         0
//     }

//     fn block_size(&self) -> usize {
//         8
//     }

//     fn initialize(&mut self, _: &[u8], _: &[u8]) -> error::Result<()> {
//         Ok(())
//     }

//     fn update(&mut self, data: &[u8], buf: &mut Vec<u8>) -> error::Result<usize> {
//         buf.extend_from_slice(data);
//         Ok(data.len())
//     }

//     fn finalize(&mut self, _: &mut Vec<u8>) -> error::Result<usize> {
//         Ok(0)
//     }

//     fn is_galois_counter_mode(&self) -> bool {
//         false
//     }

//     fn tag_len(&self) -> usize {
//         0
//     }

//     fn update_sequence_number(&mut self, _: u32) -> error::Result<()> {
//         Ok(())
//     }

//     fn additional_authenticated_data(&mut self, _: &mut [u8]) -> error::Result<()> {
//         Ok(())
//     }

//     fn authentication_tag(&mut self, _: &mut [u8]) -> error::Result<()> {
//         Ok(())
//     }
// }
// pub struct MacNothing;

// impl Mac for MacNothing {
//     fn name(&self) -> &str {
//         "nothing"
//     }

//     fn encrypt_then_mac(&self) -> bool {
//         false
//     }

//     fn key_len(&self) -> usize {
//         0
//     }

//     fn mac_len(&self) -> usize {
//         0
//     }

//     fn initialize(&mut self, _: &[u8]) -> error::Result<()> {
//         Ok(())
//     }

//     fn update(&mut self, _: &[u8]) -> error::Result<()> {
//         Ok(())
//     }

//     fn finalize(&mut self) -> error::Result<Vec<u8>> {
//         Ok(Default::default())
//     }
// }
