use std::str::{Utf8Error, from_utf8};

use indexmap::IndexMap;
use rand::RngExt;
use snafu::ResultExt;
use tokio::io::{AsyncRead, AsyncWrite};

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
use crate::cipher::signature::Signature;
use crate::session::Notifier;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    BannerTooLong,
    UnsupportedVersion {
        version: String,
    },
    InvalidBanner {
        source: Utf8Error,
    },
    InvalidString {
        source: Utf8Error,
    },
    NegotiationFailed,
    SignatureVerificationFailed,
    ServerHostKeyRejectedByUser,
    UnexpectedMessageInStrictMode {
        code: u8,
    },
    UnexpectedDisconnectMessage {
        reason: DisconnectReason,
        description: String,
    },
    UnexpectedServerBanner {
        banner: String,
    },
}

pub struct Handshaker<T, N>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
    N: Notifier + Send + 'static,
{
    pub version: String,
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
    // pub version: String,
    // pub banner: Vec<String>,
    pub kex: IndexMap<String, Factory<kex::Algorithm>>,
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
}

pub struct MatchedMethods {
    kex: Box<kex::Algorithm>,
    host_key: Box<dyn Verify + Send>,
    crypt_client_to_server: Box<dyn Encrypt + Send>,
    crypt_server_to_client: Box<dyn Decrypt + Send>,
    mac_client_to_server: Box<dyn Mac + Send>,
    mac_server_to_client: Box<dyn Mac + Send>,
    compress_client_to_server: Box<dyn Encode + Send>,
    compress_server_to_client: Box<dyn Decode + Send>,
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
        }
    }
}

#[derive(Default, Clone, Copy)]
pub struct CompatOptions {
    ext_eof: bool,
    old_forward_addr: bool,
    bug_signature_type: bool,
    bug_signature_type74: bool,
    old_session_id: bool,
    bug_debug: bool,
    bug_scanner: bool,
    old_dhgex: bool,
    bug_norkey: bool,
    bug_ext_eof: bool,
    bug_probe: bool,
    new_openssh: bool,
    bug_dynamic_prort: bool,
    bug_curve25519_pad: bool,
    bug_hostkeys: bool,
    bug_dn_gex_large: bool,
}

impl CompatOptions {
    fn parse(software: &str) -> Self {
        CompatOptions::default()
    }
}

impl<T, N> Handshaker<T, N>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
    N: Notifier + Send + 'static,
{
    pub fn new(socket: T, notifier: N,  config: Config) -> Self {
        let version = format!(
            "SSH-2.0-{}_{}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        );
        Self {
            version,
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
            r"^SSH-(?P<version>2\.0|1\.99)-(?P<software>[\x21-\x2B\x2E-\x7E]+)(?: (?P<comment>[\x20-\x7E]*))?$"
        ).expect("Invalid reggular expression");
        let stream = self.socket.as_mut().unwrap();

        if !self.banner.is_empty() {
            let banner = self.banner.join(protocol::BANNER_ENDING);
            stream.put_bytes(banner);
            stream.put_bytes(protocol::BANNER_ENDING);
        }
        stream.put_bytes(&self.version);
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
                    println!("version is:{}", version);
                    if version != "2.0" && version != "1.99" {
                        return Err(UnsupportedVersionSnafu {
                            version: line.to_string(),
                        }
                            .build()
                            .into());
                    }
                    let software = &caps["software"];
                    let comment = caps.name("comment").map(|v| v.as_str().to_string());
                    tracing::info!("Server banner: version={}, software={}, comment={:?}", version, software, comment);

                    self.compat_options = CompatOptions::parse(software);
                    self.server_version = Some(line.to_string());
                    self.server_banner = Some(lines);

                    let mut plain_stream = PlainStream::new(self.socket.take().unwrap());

                    plain_stream.client_mut().ext = self.config.ext;
                    plain_stream.client_mut().kex_strict = self.config.key_strict;
                    self.plain_stream = Some(plain_stream);
                    Ok(())
                } else {
                    Err(UnexpectedServerBannerSnafu { banner: line }.build().into())
                }
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
        let client_methods = Methods::from_config(&self.config);

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
                self.plain_stream.as_mut().unwrap().server_mut().kex_strict =
                    server_method.kex_strict;
                self.plain_stream.as_mut().unwrap().server_mut().ext = server_method.ext;
                let matched = self.config.negotiate(&server_method)?;
                self.matched_methods = Some(matched);
                self.server_kex_msg = Some(packet.payload);
                println!("server methods: {:#?}", server_method);
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
        let mut is_exchange = false;
        let mut is_curve = false;
        let kex: &mut dyn KeyExchange = match &mut *matched.kex {
            kex::Algorithm::Standard(standard_diffie_hellman) => {
                // let client_public_key = standard_diffie_hellman.generate_key()?;
                // let mut producer = Producer::default();
                // producer.put_u8(protocol::SSH_MSG_KEXDH_INIT);
                // producer.put_one(&client_public_key);

                // self.plain_stream
                //     .as_mut()
                //     .unwrap()
                //     .send_payload(&producer[..])
                //     .await?;

                // let packet = self.plain_stream.as_mut().unwrap().recv_packet().await?;
                &mut **standard_diffie_hellman
            }
            kex::Algorithm::Exchange(exchange_diffie_hellman) => {
                //

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

                if self.server_version.as_ref().unwrap().contains("Cisco-1") && bits > 4096 {
                    bits = 4096;
                }

                exchange_diffie_hellman.set_recommended_number_of_bits(bits);

                let mut producer = Producer::default();
                producer.put_u8(protocol::SSH_MSG_KEX_DH_GEX_REQUEST);
                producer.put_u32(exchange_diffie_hellman.min());
                producer.put_u32(exchange_diffie_hellman.number_of_bits());
                producer.put_u32(exchange_diffie_hellman.max());

                self.plain_stream
                    .as_mut()
                    .unwrap()
                    .send_payload(&producer[..])
                    .await?;

                loop {
                    let packet = self.plain_stream.as_mut().unwrap().recv_packet().await?;
                    let mut consumer = Consumer::new(&packet.payload);
                    if consumer.consume_u8()? == protocol::SSH_MSG_KEX_DH_GEX_GROUP {
                        let p = consumer.consume_one()?;
                        let g = consumer.consume_one()?;
                        exchange_diffie_hellman.initialize(p, g)?;
                        break;
                    } else {
                        self.plain_stream
                            .as_mut()
                            .unwrap()
                            .handle_msg(packet)
                            .await?;
                    }
                }

                is_exchange = true;

                &mut **exchange_diffie_hellman
            }
            kex::Algorithm::EllipticCurve(elliptic_curve_diffie_hellman) => {
                is_curve = true;
                &mut **elliptic_curve_diffie_hellman
            }
            kex::Algorithm::Curve25519(curve25519) => {
                is_curve = true;
                &mut **curve25519
            }
            kex::Algorithm::Streamlined(streamlined) => {
                is_curve = true;
                &mut **streamlined
            },
            kex::Algorithm::Hybrid(hybrid) => {
                is_curve = true;
                &mut **hybrid
            }
        };

        let (request_code, reply_code) = if is_exchange {
            (
                protocol::SSH_MSG_KEX_DH_GEX_INIT,
                protocol::SSH_MSG_KEX_DH_GEX_REPLY,
            )
        } else if is_curve {
            (
                protocol::SSH_MSG_KEX_ECDH_INIT,
                protocol::SSH_MSG_KEX_ECDH_REPLY,
            )
        } else {
            (protocol::SSH_MSG_KEXDH_INIT, protocol::SSH_MSG_KEXDH_REPLY)
        };
        let client_public_key = kex.generate_key()?;
        let mut producer = Producer::default();
        producer.put_u8(request_code);
        producer.put_one(&client_public_key);

        self.plain_stream
            .as_mut()
            .unwrap()
            .send_payload(&producer[..])
            .await?;

        loop {
            let packet = self.plain_stream.as_mut().unwrap().recv_packet().await?;
            let mut consumer = Consumer::new(&packet.payload);
            if consumer.consume_u8()? == reply_code {
                let host_key = consumer.consume_one()?;
                let server_public_key = consumer.consume_one()?;
                let signature = consumer.consume_one()?;

                let secret_key = kex.compute_secret_key(server_public_key)?;

                let info = Information {
                    client_version: &self.version,
                    server_version: self.server_version.as_ref().unwrap(),
                    client_kex_init: self.client_kex_msg.as_ref().unwrap(),
                    server_kex_init: self.server_kex_msg.as_ref().unwrap(),
                    server_host_key: &host_key,
                    client_public_key: &client_public_key,
                    server_public_key,
                    secret_key: &secret_key,
                };

                let hash = kex.compute_hash(info)?;

                tracing::info!("Using host_key algorithm: {}", matched.host_key.name());
                matched.host_key.initialize(&host_key)?;

                let res = matched.host_key.verify(signature, &hash)?;

                snafu::ensure!(res, SignatureVerificationFailedSnafu);

                if !self.notifier.verify_server_host_key(matched.host_key.name(), host_key).await {
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

                {
                    let hash = &hash;
                    let session_id = hash;

                    let local_iv = kex.compute_communicate_key(
                        &secret_key[..],
                        session_id,
                        hash,
                        b'A',
                        matched.crypt_client_to_server.iv_len(),
                    )?;

                    let local_key = kex.compute_communicate_key(
                        &secret_key[..],
                        session_id,
                        hash,
                        b'C',
                        matched.crypt_client_to_server.key_len(),
                    )?;

                    matched
                        .crypt_client_to_server
                        .initialize(&local_iv, &local_key)?;

                    let remote_iv = kex.compute_communicate_key(
                        &secret_key[..],
                        session_id,
                        hash,
                        b'B',
                        matched.crypt_server_to_client.iv_len(),
                    )?;
                    let remote_key = kex.compute_communicate_key(
                        &secret_key[..],
                        session_id,
                        hash,
                        b'D',
                        matched.crypt_server_to_client.key_len(),
                    )?;

                    matched
                        .crypt_server_to_client
                        .initialize(&remote_iv, &remote_key)?;
                    let local_key = kex.compute_communicate_key(
                        &secret_key[..],
                        session_id,
                        hash,
                        b'E',
                        matched.mac_client_to_server.key_len(),
                    )?;
                    let remote_key = kex.compute_communicate_key(
                        &secret_key[..],
                        session_id,
                        hash,
                        b'F',
                        matched.mac_server_to_client.key_len(),
                    )?;
                    matched.mac_client_to_server.initialize(&local_key)?;
                    matched.mac_server_to_client.initialize(&remote_key)?;
                }


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
                        println!("handle msg");
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
