use std::collections::HashMap;
use std::str::Utf8Error;

use openssl::base64::decode_block;
use openssl::bn::BigNumContext;
use openssl::ec::PointConversionForm;
use openssl::pkey::{Id, PKey};
use snafu::{OptionExt, ResultExt};

use crate::error::Result;
use crate::ssh::MultiplePrecisionInteger;
use crate::ssh::buffer::{Consumer, Producer};
use crate::{
    cipher::{
        Factory,
        crypt::{self, Decrypt},
    },
    error::builder,
};
//
// /// OpenSSH 支持的 20 种密钥类型（key_type）常量定义
//
// // ===== Ed25519 (2 种) =====
// pub const KEY_TYPE_ED25519: &str = "ssh-ed25519";
// pub const KEY_TYPE_ED25519_CERT: &str = "ssh-ed25519-cert-v01@openssh.com";
//
// // ===== Ed25519-SK (2 种) =====
// pub const KEY_TYPE_SK_ED25519: &str = "sk-ssh-ed25519@openssh.com";
// pub const KEY_TYPE_SK_ED25519_CERT: &str = "sk-ssh-ed25519-cert-v01@openssh.com";
//
// // ===== ECDSA (6 种) =====
// pub const KEY_TYPE_ECDSA_NISTP256: &str = "ecdsa-sha2-nistp256";
// pub const KEY_TYPE_ECDSA_NISTP256_CERT: &str = "ecdsa-sha2-nistp256-cert-v01@openssh.com";
// pub const KEY_TYPE_ECDSA_NISTP384: &str = "ecdsa-sha2-nistp384";
// pub const KEY_TYPE_ECDSA_NISTP384_CERT: &str = "ecdsa-sha2-nistp384-cert-v01@openssh.com";
// pub const KEY_TYPE_ECDSA_NISTP521: &str = "ecdsa-sha2-nistp521";
// pub const KEY_TYPE_ECDSA_NISTP521_CERT: &str = "ecdsa-sha2-nistp521-cert-v01@openssh.com";
//
// // ===== ECDSA-SK (4 种) =====
// pub const KEY_TYPE_SK_ECDSA_NISTP256: &str = "sk-ecdsa-sha2-nistp256@openssh.com";
// pub const KEY_TYPE_SK_ECDSA_NISTP256_CERT: &str = "sk-ecdsa-sha2-nistp256-cert-v01@openssh.com";
// pub const KEY_TYPE_WEBAUTHN_SK_ECDSA_NISTP256: &str = "webauthn-sk-ecdsa-sha2-nistp256@openssh.com";
// pub const KEY_TYPE_WEBAUTHN_SK_ECDSA_NISTP256_CERT: &str =
//     "webauthn-sk-ecdsa-sha2-nistp256-cert-v01@openssh.com";
//
// // ===== RSA (6 种) =====
// pub const KEY_TYPE_RSA: &str = "ssh-rsa";
// pub const KEY_TYPE_RSA_CERT: &str = "ssh-rsa-cert-v01@openssh.com";
// pub const KEY_TYPE_RSA_SHA256: &str = "rsa-sha2-256";
// pub const KEY_TYPE_RSA_SHA256_CERT: &str = "rsa-sha2-256-cert-v01@openssh.com";
// pub const KEY_TYPE_RSA_SHA512: &str = "rsa-sha2-512";
// pub const KEY_TYPE_RSA_SHA512_CERT: &str = "rsa-sha2-512-cert-v01@openssh.com";
//
// /// 所有 20 种 key_type 的数组，方便遍历
// pub const ALL_KEY_TYPES: [&str; 16] = [
//     KEY_TYPE_ED25519,
//     KEY_TYPE_ED25519_CERT,
//     KEY_TYPE_SK_ED25519,
//     KEY_TYPE_SK_ED25519_CERT,
//     KEY_TYPE_ECDSA_NISTP256,
//     KEY_TYPE_ECDSA_NISTP256_CERT,
//     KEY_TYPE_ECDSA_NISTP384,
//     KEY_TYPE_ECDSA_NISTP384_CERT,
//     KEY_TYPE_ECDSA_NISTP521,
//     KEY_TYPE_ECDSA_NISTP521_CERT,
//     KEY_TYPE_SK_ECDSA_NISTP256,
//     KEY_TYPE_SK_ECDSA_NISTP256_CERT,
//     KEY_TYPE_WEBAUTHN_SK_ECDSA_NISTP256,
//     KEY_TYPE_WEBAUTHN_SK_ECDSA_NISTP256_CERT,
//     KEY_TYPE_RSA,
//     KEY_TYPE_RSA_CERT, // KEY_TYPE_RSA_SHA256,
//                        // KEY_TYPE_RSA_SHA256_CERT,
//                        // KEY_TYPE_RSA_SHA512,
//                        // KEY_TYPE_RSA_SHA512_CERT,
// ];

/// 证书后缀常量
pub const CERT_SUFFIX: &str = "-cert-v01@openssh.com";

#[derive(derive_more::Debug)]
pub struct Private {
    pub r#type: String,
    #[debug(skip)]
    pub public: Vec<u8>,
    #[debug(skip)]
    pub private: Vec<u8>,
    pub comment: String,
}

#[derive(derive_more::Debug)]
pub enum Public {
    Normal {
        r#type: String,
        #[debug(skip)]
        content: Vec<u8>,
        comment: Option<String>,
    },
    Certificate {
        r#type: String,
        #[debug(skip)]
        content: Vec<u8>,
        comment: Option<String>,
        #[debug(skip)]
        public: Vec<u8>,
        principals: Vec<String>,
    },
}

// pub struct Public {
//     pub method: String,
//     pub content: Vec<u8>,
//     pub comment: Option<String>,
// }

#[derive(snafu::Snafu, Debug)]
pub enum Error {
    WrongPassphrase,

    #[snafu(display("Unsupported key type: {}", r#type))]
    UnsupportedKeyType {
        r#type: String,
    },

    #[snafu(display("Failed to decrypt key: {}", source))]
    DecryptionError {
        source: bcrypt_pbkdf::Error,
    },

    FormatError {
        detail: String,
        #[snafu(implicit)]
        location: snafu::Location,
    },
    UnsupportedFeature {
        detail: String,
    },
    UnsupportedAlgorithm {
        detail: String,
    },
    TextError {
        source: Utf8Error,
    },
}

pub struct Parser {
    cipher: HashMap<String, Factory<dyn Decrypt + Send>>,
}

impl Default for Parser {
    fn default() -> Self {
        let cipher = crypt::new_decrypt_all();
        let cipher = cipher
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect::<HashMap<_, _>>();
        Self { cipher }
    }
}

// #[derive(Debug)]
// pub struct Private {
//     r#type: String,
//     public_key: Vec<u8>,
//     content: PrivateKeyContent,
//     cert: Option<Vec<u8>>,
//     comment: Vec<u8>,
// }
//
// impl Private {
//     pub fn ssh_public_key(&self) -> &[u8] {
//         &self.public_key
//     }
//
//     pub fn ssh_private_key(&self) -> Vec<u8> {
//         match &self.content {
//             PrivateKeyContent::Ed25519 { secret, .. } => secret.to_vec(),
//             PrivateKeyContent::Rsa {
//                 n,
//                 e,
//                 d,
//                 iqmp,
//                 p,
//                 q,
//                 dmp1,
//                 dmq1,
//             } => todo!(),
//             PrivateKeyContent::EcdsaSha2 {
//                 curve,
//                 ec_pt,
//                 private,
//             } => todo!(),
//             PrivateKeyContent::SkEd25519 {
//                 public,
//                 app,
//                 flags,
//                 key_handle,
//                 reserved,
//             } => todo!(),
//             PrivateKeyContent::SkEcdsa {
//                 curve,
//                 ec_pt,
//                 app,
//                 flags,
//                 key_handle,
//                 reserved,
//             } => todo!(),
//         }
//     }
//
//     pub fn signature_method(&self) -> Result<String> {
//         if ALL_KEY_TYPES
//             .iter()
//             .position(|&e| e == self.r#type)
//             .is_some()
//         {
//             if self.r#type.starts_with(KEY_TYPE_ED25519) {
//                 return Ok(KEY_TYPE_ED25519.to_string());
//             } else if self.r#type.starts_with(KEY_TYPE_RSA) {
//                 return Ok(KEY_TYPE_RSA.to_string());
//             } else if self.r#type.starts_with("ecdsa-sha2-") {
//                 return Ok(KEY_TYPE_ECDSA_NISTP256.to_string());
//             }
//         }
//
//         Err(UnsupportedKeyTypeSnafu {
//             r#type: self.r#type.clone(),
//         }
//         .build()
//         .into())
//
//         // builder::InvalidArgument {
//         //     tip: "Unsupported key type",
//         // }
//         // .fail()
//     }
//
//     fn parse(r#type: &str, public_key: &[u8], content: &[u8]) -> Result<Self> {
//         if !ALL_KEY_TYPES.contains(&r#type) {
//             return Err(UnsupportedKeyTypeSnafu { r#type }.build().into());
//         }
//
//         let mut consumer = Consumer::new(content);
//         let mut cert = None;
//         let method = if let Some(method) = r#type.strip_suffix(CERT_SUFFIX) {
//             cert = Some(consumer.consume_one()?.to_vec());
//             method
//         } else {
//             r#type
//         };
//
//         let content =
//             if method == KEY_TYPE_ED25519 {
//                 let mut public: Option<[u8; 32]> = None;
//                 if cert.is_none() {
//                     public = Some(consumer.consume_one()?.try_into().ok().context(
//                         FormatSnafu {
//                             detail: "Invalid ed25519 length",
//                         },
//                     )?);
//                 }
//                 let secret: [u8; 64] =
//                     consumer
//                         .consume_one()?
//                         .try_into()
//                         .ok()
//                         .context(FormatSnafu {
//                             detail: "Invalid ed25519 length",
//                         })?;
//
//                 PrivateKeyContent::Ed25519 { public, secret }
//             } else if method.contains("rsa") {
//                 let mut n = None;
//                 let mut e = None;
//
//                 if cert.is_none() {
//                     n = Some(consumer.consume_one()?.to_vec());
//                     e = Some(consumer.consume_one()?.to_vec());
//                 }
//
//                 let d = consumer.consume_one()?.to_vec();
//                 let iqmp = consumer.consume_one()?.to_vec();
//                 let p = consumer.consume_one()?.to_vec();
//                 let q = consumer.consume_one()?.to_vec();
//
//                 let (dmp1, dmq1) = {
//                     let p = num_bigint::BigUint::from_bytes_be(&p);
//                     let q = num_bigint::BigUint::from_bytes_be(&q);
//                     let d = num_bigint::BigUint::from_bytes_be(&d);
//
//                     let dmp1 = &d % &(&p - 1u32);
//                     let dmq1 = &d % &(&q - 1u32);
//                     (dmp1.to_bytes_be(), dmq1.to_bytes_be())
//                 };
//
//                 PrivateKeyContent::Rsa {
//                     n,
//                     e,
//                     d,
//                     iqmp,
//                     p,
//                     q,
//                     dmp1,
//                     dmq1,
//                 }
//             } else if method.contains("ecdsa-sha2-") {
//                 let mut curve = None;
//                 let mut ec_pt = None;
//
//                 if cert.is_none() {
//                     curve = Some(consumer.consume_one()?.to_vec());
//                     ec_pt = Some(consumer.consume_one()?.to_vec());
//                 }
//
//                 let private = consumer.consume_one()?.to_vec();
//
//                 PrivateKeyContent::EcdsaSha2 {
//                     curve,
//                     ec_pt,
//                     private,
//                 }
//             } else if method.contains("sk-") {
//                 let mut public = None;
//                 let mut curve = None;
//                 let mut ec_pt = None;
//                 let mut is_edcsa = false;
//                 if method.contains("ed25519") {
//                     if cert.is_none() {
//                         public = Some(consumer.consume_one()?.try_into().ok().context(
//                             FormatSnafu {
//                                 detail: "Invalid ed25519 length",
//                             },
//                         )?);
//                     }
//                 } else if method.contains("edcsa") {
//                     if cert.is_none() {
//                         curve = Some(consumer.consume_one()?.to_vec());
//                         ec_pt = Some(consumer.consume_one()?.to_vec());
//                     }
//                     is_edcsa = true;
//                 } else {
//                     // return builder::InvalidFormat {
//                     //     tip: "Unknown sk- method",
//                     // }
//                     // .fail();
//                     return Err(UnsupportedKeyTypeSnafu { r#type: method }.build().into());
//                 }
//                 let app = consumer.consume_one()?.to_vec();
//                 let flags = consumer.consume_u8()?;
//                 let key_handle = consumer.consume_one()?.to_vec();
//                 let reserved = consumer.consume_one()?.to_vec();
//                 if is_edcsa {
//                     PrivateKeyContent::SkEcdsa {
//                         curve,
//                         ec_pt,
//                         app,
//                         flags,
//                         key_handle,
//                         reserved,
//                     }
//                 } else {
//                     PrivateKeyContent::SkEd25519 {
//                         public,
//                         app,
//                         flags,
//                         key_handle,
//                         reserved,
//                     }
//                 }
//             } else {
//                 // return builder::InvalidFormat {
//                 //     tip: "Unknown sk- method",
//                 // }
//                 // .fail();
//                 return Err(UnsupportedKeyTypeSnafu { r#type: method }.build().into());
//             };
//
//         let comment = consumer.consume_one()?.to_vec();
//
//         for i in 0..consumer.peek().len() {
//             if consumer.peek()[i] != (i + 1) as u8 & 0xff {
//                 // return builder::InvalidFormat {
//                 //     tip: "Unexpected byte in private key",
//                 // }
//                 // .fail();
//                 println!("padding: i={}, expected={}, actual={}", i, i & 0xff, consumer.peek()[i]);
//                 return Err(FormatSnafu {
//                     detail: "Unexpected padding in private key",
//                 }
//                 .build()
//                 .into());
//             }
//         }
//
//         let public_key = public_key.to_vec();
//         let r#type = r#type.to_string();
//
//         Ok(Self {
//             r#type,
//             public_key,
//             content,
//             cert,
//             comment,
//         })
//     }
// }
//
// #[derive(Debug)]
// pub enum PrivateKeyContent {
//     Ed25519 {
//         public: Option<[u8; 32]>,
//         secret: [u8; 64],
//     },
//     Rsa {
//         n: Option<Vec<u8>>,
//         e: Option<Vec<u8>>,
//         d: Vec<u8>,
//         iqmp: Vec<u8>,
//         p: Vec<u8>,
//         q: Vec<u8>,
//         dmp1: Vec<u8>,
//         dmq1: Vec<u8>,
//     },
//     EcdsaSha2 {
//         curve: Option<Vec<u8>>,
//         ec_pt: Option<Vec<u8>>,
//         private: Vec<u8>,
//     },
//     SkEd25519 {
//         public: Option<[u8; 32]>,
//         app: Vec<u8>,
//         flags: u8,
//         key_handle: Vec<u8>,
//         reserved: Vec<u8>,
//     },
//     SkEcdsa {
//         curve: Option<Vec<u8>>,
//         ec_pt: Option<Vec<u8>>,
//         app: Vec<u8>,
//         flags: u8,
//         key_handle: Vec<u8>,
//         reserved: Vec<u8>,
//     },
// }

impl Parser {
    const OPEN_SSH_HEADER: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";
    const OPEN_SSH_FOOTER: &str = "-----END OPENSSH PRIVATE KEY-----";
    const OPEN_SSH_MAGIC: &str = "openssh-key-v1\0";

    const SSH2_PUBLIC_KEY_HEADER: &str = "---- BEGIN SSH2 PUBLIC KEY ----";
    const SSH2_PUBLIC_KEY_FOOTER: &str = "---- END SSH2 PUBLIC KEY ----";
    // pub const SSH_ED25519: &str = "ssh-ed25519";
    // pub const SSH_RSA: &str = "ssh-rsa";
    // pub const ECDSA_SHA2_PREFIX: &str = "ecdsa-sha2-";

    // pub fn parse_private_key_file_ed25519_cert_open_ssh(
    //     &self,
    //     consumer: &mut Consumer<'_>,
    // ) -> Result<(Vec<u8>, [u8; 64])> {
    //     let cert = consumer.consume_one()?.to_vec();
    //     let second = consumer.consume_one()?;
    //
    //     if second.len() != 64 {
    //         // return builder::InvalidArgument {
    //         //     tip: "Invalid ED25519 key length".to_string(),
    //         // }
    //         // .fail();
    //         return Err(FormatSnafu {
    //             detail: "Invalid ED25519 key length",
    //         }
    //         .build()
    //         .into());
    //     }
    //     Ok((cert, second.try_into().unwrap()))
    // }
    //
    // pub fn parse_private_key_file_ed25519(
    //     &self,
    //     consumer: &mut Consumer<'_>,
    // ) -> Result<([u8; 32], [u8; 64])> {
    //     let first = consumer.consume_one()?;
    //     let second = consumer.consume_one()?;
    //
    //     if first.len() != 32 || second.len() != 64 {
    //         // return builder::InvalidArgument {
    //         //     tip: "Invalid ED25519 key length".to_string(),
    //         // }
    //         // .fail();
    //         return Err(FormatSnafu {
    //             detail: "Invalid ED25519 key length",
    //         }
    //         .build()
    //         .into());
    //     }
    //
    //     if first != &second[32..] {
    //         // return builder::InvalidArgument {
    //         //     tip: "Invalid ED25519 key ".to_string(),
    //         // }
    //         // .fail();
    //         return Err(FormatSnafu {
    //             detail: "Invalid ED25519 key",
    //         }
    //         .build()
    //         .into());
    //     }
    //
    //     Ok((first.try_into().unwrap(), second.try_into().unwrap()))
    // }

    pub fn parse_private_key_file_open_ssh(
        &self,
        content: &[u8],
        passphrase: Option<&[u8]>,
    ) -> Result<Private> {
        let mut consumer = Consumer::new(content);

        if consumer.consume_bytes(Self::OPEN_SSH_MAGIC.len())? != Self::OPEN_SSH_MAGIC.as_bytes() {
            // return builder::InvalidArgument {
            //     tip: "Invalid OpenSSH magic".to_string(),
            // }
            // .fail();

            return Err(FormatSnafu {
                detail: "Invalid OpenSSH magic".to_string(),
            }
            .build()
            .into());
        }

        let cipher_name = consumer.consume_one()?;

        let kdf_name = consumer.consume_one()?;

        let kdf_options = consumer.consume_one()?;

        let number_of_keys = consumer.consume_u32()?;

        if number_of_keys != 1 {
            // return builder::InvalidArgument {
            //     tip: "Unsupported multiple keys".to_string(),
            // }
            // .fail();
            return Err(UnsupportedFeatureSnafu {
                detail: "Unsupported multiple keys",
            }
            .build()
            .into());
        }

        let public_key_data = consumer.consume_one()?;

        let mut encrypted_data = consumer.consume_one()?.to_vec();

        if cipher_name == b"none" {
            if encrypted_data.len() % 8 != 0 {
                // return builder::InvalidFormat {
                //     tip: "Invalid cipher data".to_string(),
                // }
                // .fail();
                return Err(FormatSnafu {
                    detail: "Invalid cipher data",
                }
                .build()
                .into());
            }
        } else {
            if kdf_name == b"none" {
                // return builder::InvalidFormat {
                //     tip: "No kv provided",
                // }
                // .fail();
                return Err(FormatSnafu {
                    detail: "No kv provided",
                }
                .build()
                .into());
            } else if kdf_name != b"bcrypt" {
                return Err(FormatSnafu {
                    detail: "Unsupported kdf".to_string(),
                }
                .build()
                .into());
            }
            let passphrase = match passphrase {
                Some(p) if !p.is_empty() => p,
                _ => {
                    // return builder::InvalidArgument {
                    //     tip: "Wrong passphrase",
                    // }
                    // .fail();
                    return Err(WrongPassphraseSnafu.build().into());
                }
            };
            let (salt, round) = {
                let mut consumer = Consumer::new(kdf_options);
                let salt = consumer.consume_one()?;
                let round = consumer.consume_u32()?;

                (salt, round)
            };

            let cipher_name = std::str::from_utf8(cipher_name).context(TextSnafu)?;

            let mut cipher = self
                .cipher
                .get(cipher_name)
                .context(UnsupportedAlgorithmSnafu {
                    detail: format!("Unsupported algorithm: {}", cipher_name),
                })?();

            if encrypted_data.len() % cipher.block_size() != 0 {
                // return builder::InvalidFormat {
                //     tip: "Invalid cipher data".to_string(),
                // }
                // .fail();
                return Err(FormatSnafu {
                    detail: "Invalid cipher data",
                }
                .build()
                .into());
            }

            let mut output = vec![0; cipher.key_len() + cipher.iv_len()];

            bcrypt_pbkdf::bcrypt_pbkdf(passphrase, salt, round, &mut output)
                .context(DecryptionSnafu)?;

            cipher.initialize(&output[cipher.key_len()..], &output[..cipher.key_len()])?;

            let mut plain_text = vec![];

            cipher.update(&encrypted_data, &mut plain_text)?;

            if cipher.is_galois_counter_mode() {
                let tag = consumer.consume_bytes(cipher.tag_len())?;

                cipher.authentication_tag(tag)?;
            }

            cipher.finalize(&mut plain_text)?;

            encrypted_data = plain_text;
        }

        if !consumer.peek().is_empty() {
            // return builder::InvalidFormat {
            //     tip: "Unexpected data after cipher data".to_string(),
            // }
            // .fail();
            return Err(FormatSnafu {
                detail: "Unexpected data after cipher data",
            }
            .build()
            .into());
        }

        {
            let mut consumer = Consumer::new(&encrypted_data);
            let check1 = consumer.consume_u32()?;
            let check2 = consumer.consume_u32()?;
            if check1 != check2 {
                // return builder::InvalidFormat {
                //     tip: "Checksum mismatch".to_string(),
                // }
                // .fail();
                return Err(FormatSnafu {
                    detail: "Checksum mismatch",
                }
                .build()
                .into());
            }
            let method = consumer.consume_one()?;

            // if method == KEY_TYPE_ED25519.as_bytes() {
            //     let (public, secret) = self.parse_private_key_file_ed25519(&mut consumer)?;
            //     let comment = consumer.consume_one()?;

            //     return Ok(Private::Ed25519 {
            //         public_key: public_key_data.to_vec(),
            //         public,
            //         secret,
            //         comment: comment.to_vec(),
            //     });
            // } else if method == KEY_TYPE_RSA.as_bytes() {
            // }
            //

            let method = std::str::from_utf8(method).context(TextSnafu)?;

            let result = match method {
                "ssh-rsa" => {
                    let n = consumer.consume_one()?.to_vec();
                    let e = consumer.consume_one()?.to_vec();

                    {
                        let mut consumer = Consumer::new(public_key_data);
                        if consumer.consume_one()? != method.as_bytes() {
                            return Err(FormatSnafu {
                                detail: "Key type mismatched",
                            }
                            .build()
                            .into());
                        }
                        if consumer.consume_one()? != e {
                            return Err(FormatSnafu {
                                detail: "Rsa e mismatched",
                            }
                            .build()
                            .into());
                        }
                        if consumer.consume_one()? != n {
                            return Err(FormatSnafu {
                                detail: "Rsa e mismatched",
                            }
                            .build()
                            .into());
                        }
                    }

                    let d = consumer.consume_one()?.to_vec();
                    let iqmp = consumer.consume_one()?.to_vec();
                    let p = consumer.consume_one()?.to_vec();
                    let q = consumer.consume_one()?.to_vec();

                    let private = make_buffer_without_header! {
                        one: method,
                        one: n,
                        one: e,
                        one: d,
                        one: iqmp,
                        one: p,
                        one: q
                    }
                    .into_vec();

                    let comment = consumer.consume_one()?;

                    let comment = std::str::from_utf8(comment).context(TextSnafu)?.to_string();

                    Private {
                        public: public_key_data.to_vec(),
                        private,
                        r#type: method.to_string(),
                        comment,
                    }
                }
                "ssh-ed25519" => {
                    // let r#type = consumer.consume_one()?;
                    // let s = String::from_utf8_lossy(r#type);
                    // if r#type != method.as_bytes() {
                    //     return Err(FormatSnafu {
                    //         detail: "Type mismatched"
                    //     }.build().into());
                    // }
                    let public = consumer.consume_one()?;
                    let secret = consumer.consume_one()?;
                    {
                        let mut consumer = Consumer::new(public_key_data);
                        if consumer.consume_one()? != method.as_bytes() {
                            return Err(FormatSnafu {
                                detail: "Key type mismatched",
                            }
                            .build()
                            .into());
                        }
                        if consumer.consume_one()? != public {
                            return Err(FormatSnafu {
                                detail: "Public key mismatched",
                            }
                            .build()
                            .into());
                        }
                    }
                    if secret.len() != 64 {
                        return Err(FormatSnafu {
                            detail: "Unexpected secret length",
                        }
                        .build()
                        .into());
                    }

                    if public != &secret[32..] {
                        return Err(FormatSnafu {
                            detail: "Unexpected secret key",
                        }
                        .build()
                        .into());
                    }

                    let private = make_buffer_without_header!(
                        one: method,
                        one: &secret[..32]
                    )
                    .into_vec();

                    let comment = consumer.consume_one()?;

                    let comment = std::str::from_utf8(comment).context(TextSnafu)?.to_string();

                    Private {
                        public: public_key_data.to_vec(),
                        private,
                        r#type: method.to_string(),
                        comment,
                    }
                }
                "ecdsa-sha2-nistp256" | "ecdsa-sha2-nistp384" | "ecdsa-sha2-nistp521" => {
                    let curve = consumer.consume_one()?;

                    if !method.as_bytes().ends_with(curve) {
                        return Err(FormatSnafu {
                            detail: "Curve mismatched",
                        }
                        .build()
                        .into());
                    }

                    let public_key = consumer.consume_one()?;
                    let private_key = consumer.consume_one()?;
                    {
                        let mut consumer = Consumer::new(public_key_data);
                        if consumer.consume_one()? != method.as_bytes() {
                            return Err(FormatSnafu {
                                detail: "Key type mismatched",
                            }
                            .build()
                            .into());
                        }
                        if consumer.consume_one()? != curve {
                            return Err(FormatSnafu {
                                detail: "Curve mismatched",
                            }
                            .build()
                            .into());
                        }
                        if consumer.consume_one()? != public_key {
                            return Err(FormatSnafu {
                                detail: "Public key dismatched",
                            }
                            .build()
                            .into());
                        }
                    }
                    let private = make_buffer_without_header! {
                        one: method,
                        one: curve,
                        one: public_key,
                        one: private_key
                    }
                    .into_vec();

                    let comment = consumer.consume_one()?;

                    let comment = std::str::from_utf8(comment).context(TextSnafu)?.to_string();
                    Private {
                        public: public_key_data.to_vec(),
                        private,
                        r#type: method.to_string(),
                        comment,
                    }
                }
                _ => {
                    return Err(UnsupportedKeyTypeSnafu { r#type: method }.build().into());
                }
            };

            for i in 0..consumer.peek().len() {
                if consumer.peek()[i] != (i + 1) as u8 {
                    return Err(FormatSnafu {
                        detail: "Unexpected padding in private key",
                    }
                    .build()
                    .into());
                }
            }

            Ok(result)

            // let parsers = Self::private_key_content_parser();

            // let parse = parsers.get(method).context(builder::InvalidFormat {
            //     tip: "Unsupported method",
            // })?;

            // parse(public_key_data, content)

            // match method {
            //     KEY_TYPE_ED25519 => {
            //         let (public, secret) = self.parse_private_key_file_ed25519(&mut consumer)?;
            //         let comment = consumer.consume_one()?;

            //         Ok(Private::Ed25519 {
            //             public_key: public_key_data.to_vec(),
            //             public,
            //             secret,
            //             comment: comment.to_vec(),
            //         })
            //     }
            //     KEY_TYPE_ED25519_CERT => {
            //         let (cert, secret) =
            //             self.parse_private_key_file_ed25519_cert_open_ssh(&mut consumer)?;
            //         let comment = consumer.consume_one()?;

            //         Ok(Private::Ed25519CertOpenSSH {
            //             public_key: public_key_data.to_vec(),
            //             cert,
            //             secret,
            //             comment: comment.to_vec(),
            //         })
            //     }
            //     _ => builder::InvalidFormat {
            //         tip: "Unsupported key type",
            //     }
            //     .fail(),
            // }
        }
    }

    pub fn parse_private_key_file(
        &self,
        content: &[u8],
        passphrase: Option<&[u8]>,
    ) -> Result<Private> {
        let content = std::str::from_utf8(content).context(TextSnafu)?;
        let trim = content.replace("\r", "").replace("\n", "");

        if let Some(content) = trim.strip_prefix(Self::OPEN_SSH_HEADER)
            && let Some(content) = content.strip_suffix(Self::OPEN_SSH_FOOTER)
        {
            let content = decode_block(content).context(builder::OpenSSL)?;

            return self.parse_private_key_file_open_ssh(&content, passphrase);
        }
        let key = if let Some(passphrase) = passphrase {
            PKey::private_key_from_pem_passphrase(content.as_bytes(), passphrase)
                .context(builder::OpenSSL)?
        } else {
            PKey::private_key_from_pem(content.as_bytes()).context(builder::OpenSSL)?
        };

        match key.id() {
            Id::RSA => {
                let rsa = key.rsa().context(builder::OpenSSL)?;

                let iqmp = rsa.iqmp().context(FormatSnafu {
                    detail: "RSA private key is invalid",
                })?;
                let p = rsa.p().context(FormatSnafu {
                    detail: "RSA private key is invalid",
                })?;
                let q = rsa.q().context(FormatSnafu {
                    detail: "RSA private key is invalid",
                })?;

                let private = make_buffer_without_header! {
                    one: "ssh-rsa",
                    one: rsa.n().to_integer(),
                    one: rsa.e().to_integer(),
                    one: rsa.d().to_integer(),
                    one: iqmp.to_integer(),
                    one: p.to_integer(),
                    one: q.to_integer(),
                }
                .into_vec();

                let public = make_buffer_without_header! {
                    one: "ssh-rsa",
                    one: rsa.e().to_integer(),
                    one: rsa.n().to_integer(),
                }
                .into_vec();

                // let public = Rsa::from_public_components(rsa.n().to_owned().context(builder::OpenSSL)?, rsa.e().to_owned().context(builder::OpenSSL)?).context(builder::OpenSSL)?;

                Ok(Private {
                    r#type: "ssh-rsa".to_string(),
                    public,
                    private,
                    comment: "".to_string(),
                })
            }
            Id::ED25519 => {
                let private = key.raw_private_key().context(builder::OpenSSL)?;
                let public = key.raw_public_key().context(builder::OpenSSL)?;

                let private = make_buffer_without_header! {
                    one: "ssh-ed25519",
                    one: private
                }
                .into_vec();

                let public = make_buffer_without_header! {
                    one: "ssh-ed25519",
                    one: public
                }
                .into_vec();

                Ok(Private {
                    r#type: "ssh-ed25519".to_string(),
                    public,
                    private,
                    comment: "".to_string(),
                })
            }
            Id::EC => {
                let ec_key = key.ec_key().context(builder::OpenSSL)?;
                let private_key = ec_key.private_key();
                let public_key = ec_key.public_key();
                let group = ec_key.group();
                let curve = group
                    .curve_name()
                    .context(FormatSnafu {
                        detail: "EC private key is invalid",
                    })?
                    .short_name()
                    .context(builder::OpenSSL)?;

                let mut ctx = BigNumContext::new().context(builder::OpenSSL)?;

                let public_key = public_key
                    .to_bytes(group, PointConversionForm::UNCOMPRESSED, &mut ctx)
                    .context(builder::OpenSSL)?;

                let private = make_buffer_without_header! {
                    one: format!("ecdsa-sha2-{}", curve),
                    one: curve,
                    one: &public_key,
                    one: private_key.to_integer()
                }
                .into_vec();

                let public = make_buffer_without_header! {
                    one: format!("ecdsa-sha2-{}", curve),
                    one: curve,
                    one: public_key,
                }
                .into_vec();

                Ok(Private {
                    r#type: format!("ecdsa-sha2-{}", curve),
                    public,
                    private,
                    comment: "".to_string(),
                })
            }
            _ => Err(UnsupportedKeyTypeSnafu {
                r#type: format!("{:?}", key.id()),
            }
            .build()
            .into()),
        }
    }

    pub fn parse_public_key_file(&self, content: &[u8]) -> Result<Public> {
        {
            let content = std::str::from_utf8(content).context(TextSnafu)?;

            let content = content.trim().replace("\r\n", "\n");
            let content = content.replace("\r", "\n");

            if let Some(content) = content.strip_prefix(Self::SSH2_PUBLIC_KEY_HEADER)
                && let Some(content) = content.strip_suffix(Self::SSH2_PUBLIC_KEY_FOOTER)
            {
                let content = content.trim();

                let lines = content.split("\n").collect::<Vec<&str>>();
                if lines.len() == 2
                    && let Some(comment) = lines[0].strip_prefix("Comment:")
                {
                    let decoded = decode_block(lines[1]).context(builder::OpenSSL)?;
                    let mut consumer = Consumer::new(&decoded);
                    let r#type = consumer.consume_one()?;
                    let r#type = std::str::from_utf8(r#type).context(TextSnafu)?;
                    return Ok(Public::Normal {
                        r#type: r#type.to_string(),
                        content: decoded,
                        comment: Some(comment.trim_matches('\"').to_string()),
                    });
                }
            }
        }

        fn is_space_or_tab(byte: u8) -> bool {
            byte == b' ' || byte == b'\t'
        }

        let mut consumer = Consumer::new(content);

        let method = 'out: loop {
            let byte = consumer.peek_u8()?;
            if is_space_or_tab(byte) {
                consumer.consume(1);
                continue;
            }
            let data = consumer.peek();
            for i in 0..data.len() {
                if is_space_or_tab(data[i]) {
                    consumer.consume(i);
                    break 'out std::str::from_utf8(&data[..i]).context(TextSnafu)?;
                }
            }

            return Err(FormatSnafu {
                detail: "Invalid public key",
            }
            .build()
            .into());
        };

        let content = 'method: loop {
            let byte = consumer.peek_u8()?;
            if is_space_or_tab(byte) {
                consumer.consume(1);
                continue;
            }
            let data = consumer.peek();
            for i in 0..data.len() {
                if is_space_or_tab(data[i]) {
                    consumer.consume(i);
                    break 'method std::str::from_utf8(&data[..i]).context(TextSnafu)?;
                }
            }
            consumer.consume_all();

            break std::str::from_utf8(data).context(TextSnafu)?;
        };

        let mut comment = None;
        if !consumer.peek().is_empty() {
            'out: while let Ok(byte) = consumer.consume_u8() {
                // let Ok(byte) = consumer.peek_u8() else {
                //     break;
                // };
                if is_space_or_tab(byte) {
                    consumer.consume(1);
                    continue;
                }
                let data = consumer.peek();
                for i in 0..data.len() {
                    if is_space_or_tab(data[i]) {
                        consumer.consume(i);
                        comment = Some(std::str::from_utf8(&data[..i]).context(TextSnafu)?);
                        break 'out;
                    }
                }
                consumer.consume_all();

                comment = Some(std::str::from_utf8(data).context(TextSnafu)?);
                break;
            }
        }

        let content = decode_block(content).context(builder::OpenSSL)?;

        let method = method.to_string();
        let comment = comment.map(|v| v.to_string());
        if method.ends_with(CERT_SUFFIX) {
            let mut consumer = Consumer::new(&content);
            if consumer.consume_one()? != method.as_bytes() {
                return Err(FormatSnafu {
                    detail: "Mismatch key type",
                }
                .build()
                .into());
            }
            let _nonce = consumer.consume_one()?;
            // let public = consumer.consume_one()?.to_vec();
            let public = match method.as_str() {
                "ssh-ed25519-cert-v01@openssh.com" => {
                    // ed25519: 只有一个 32 字节的字符串
                    let public = consumer.consume_one()?;
                    make_buffer_without_header!(
                        one: public
                    )
                }
                "ssh-rsa-cert-v01@openssh.com" => {
                    // RSA: 两个 bignum
                    let e = consumer.consume_one()?; // 指数
                    let n = consumer.consume_one()?; // 模数
                    make_buffer_without_header!(
                        one: e,
                        one: n
                    )
                }
                "ecdsa-sha2-nistp256-cert-v01@openssh.com"
                | "ecdsa-sha2-nistp384-cert-v01@openssh.com"
                | "ecdsa-sha2-nistp521-cert-v01@openssh.com" => {
                    // ECDSA: 有 curve 和 public key 两个字段
                    let curve = consumer.consume_one()?;
                    let public = consumer.consume_one()?;
                    make_buffer_without_header! {
                        one: curve,
                        one: public
                    }
                }
                "ssh-dss-cert-v01@openssh.com" => {
                    let p = consumer.consume_one()?;
                    let q = consumer.consume_one()?;
                    let g = consumer.consume_one()?;
                    let y = consumer.consume_one()?;

                    make_buffer_without_header! {
                        one: p,
                        one: q,
                        one: g,
                        one: y
                    }
                }
                _ => {
                    return Err(UnsupportedKeyTypeSnafu { r#type: method }.build().into());
                }
            }
            .into_vec();
            let _serial = consumer.consume_u64()?;
            let _type = consumer.consume_u32()?;
            let _key_id = consumer.consume_one()?;
            let principals = {
                let mut result = Vec::new();
                let principals = consumer.consume_one()?;
                let mut consumer = Consumer::new(principals);
                while !consumer.peek().is_empty() {
                    let name = std::str::from_utf8(consumer.consume_one()?).context(TextSnafu)?;
                    result.push(name.to_string());
                }
                result
            };
            let _valid_after = consumer.consume_u64()?;
            let _valid_before = consumer.consume_u64()?;
            let _critical_options = consumer.consume_one()?;
            let _extensions = consumer.consume_one()?;
            let _reserved = consumer.consume_one()?;
            let _ca_public = consumer.consume_one()?;
            let _ca_signature = consumer.consume_one()?;

            // let principals = std::str::from_utf8(principals).context(TextSnafu)?.split(',').map(|v| v.to_string()).collect::<Vec<_>>();

            Ok(Public::Certificate {
                r#type: method,
                public,
                content,
                comment,
                principals,
            })
        } else {
            Ok(Public::Normal {
                r#type: method,
                content,
                comment,
            })
        }

        // Ok(Public {
        //     method: method.to_string(),
        //     content,
        //     comment: comment.map(|v| v.to_string()),
        // })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use rand::RngExt;
    use rand::distr::Alphanumeric;
    use std::path::Path;

    #[test]
    fn test_parse_private_key_file() {
        tracing_subscriber::fmt::init();

        let parse = |file: &Path, passphrase: Option<&str>| {
            let content = std::fs::read(file).unwrap();
            let parser = Parser::default();
            let private = parser
                .parse_private_key_file(&content, passphrase.map(|v| v.as_bytes()))
                .unwrap();
            tracing::info!("private: {:?}", private);
        };

        let ciphers = ["rsa", "ed25519", "ecdsa"];

        let run = |passphrase: Option<&str>| {
            let path = tempfile::tempdir().unwrap();
            for i in ciphers {
                if i == "ecdsa" {
                    for j in ["521", "384", "256"] {
                        let status = std::process::Command::new("ssh-keygen")
                            .arg("-t")
                            .arg(i)
                            .arg("-f")
                            .arg(path.path().join(format!("{}_{}", i, j)))
                            .arg("-b")
                            .arg(j)
                            .arg("-N")
                            .arg(passphrase.unwrap_or_default())
                            .status()
                            .unwrap();

                        assert!(status.success());

                        parse(
                            path.path().join(format!("{}_{}", i, j)).as_path(),
                            passphrase,
                        );
                    }

                    continue;
                }
                let status = std::process::Command::new("ssh-keygen")
                    .arg("-t")
                    .arg(i)
                    .arg("-f")
                    .arg(path.path().join(i))
                    .arg("-N")
                    .arg(passphrase.unwrap_or_default())
                    .status()
                    .unwrap();

                assert!(status.success());

                parse(path.path().join(i).as_path(), passphrase);
            }
        };

        run(None);
        let mut rng = rand::rng();

        let s: String = (0..10).map(|_| rng.sample(Alphanumeric) as char).collect();

        tracing::info!("passphrase: {}", s);
        run(Some(s.as_str()));
    }

    #[test]
    fn test_parse_public_key_file() {
        tracing_subscriber::fmt::init();
        let parse = |file: &Path| {
            let content = std::fs::read(file).unwrap();
            let parser = Parser::default();
            let public = parser.parse_public_key_file(&content).unwrap();
            tracing::info!("public: {:?}", public);
        };

        let ciphers = ["rsa", "ed25519", "ecdsa"];

        let path = tempfile::tempdir().unwrap();

        let status = std::process::Command::new("ssh-keygen")
            .arg("-t")
            .arg("ed25519")
            .arg("-f")
            .arg(path.path().join("ca"))
            .arg("-N")
            .arg("")
            .status()
            .unwrap();

        assert!(status.success());

        parse(path.path().join("ca.pub").as_path());

        let users = vec!["root", "user1", "user2"];
        let host = "user@example.com";

        let sign = || {
            let mut cmd = std::process::Command::new("ssh-keygen");
            cmd.arg("-s")
                .arg(path.path().join("ca"))
                .arg("-I")
                .arg(host)
                .arg("-n")
                .arg(users.join(","));
            cmd
        };

        for i in ciphers {
            if i == "ecdsa" {
                for j in ["521", "384", "256"] {
                    let status = std::process::Command::new("ssh-keygen")
                        .arg("-t")
                        .arg(i)
                        .arg("-f")
                        .arg(path.path().join(format!("{}_{}", i, j)))
                        .arg("-b")
                        .arg(j)
                        .arg("-N")
                        .arg("")
                        .status()
                        .unwrap();

                    assert!(status.success());

                    parse(path.path().join(format!("{}_{}.pub", i, j)).as_path());

                    let status = sign()
                        .arg(path.path().join(format!("{}_{}", i, j)))
                        .status()
                        .unwrap();

                    assert!(status.success());
                    parse(path.path().join(format!("{}_{}-cert.pub", i, j)).as_path());
                }

                continue;
            }
            let status = std::process::Command::new("ssh-keygen")
                .arg("-t")
                .arg(i)
                .arg("-f")
                .arg(path.path().join(i))
                .arg("-N")
                .arg("")
                .status()
                .unwrap();

            assert!(status.success());

            parse(path.path().join(format!("{}.pub", i)).as_path());

            let status = sign().arg(path.path().join(i)).status().unwrap();

            assert!(status.success());
            parse(path.path().join(format!("{}-cert.pub", i)).as_path());
        }
    }
}
