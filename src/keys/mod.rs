use crate::cipher::crypt;
use crate::cipher::crypt::Decrypt;
use crate::error::Error;
use crate::error::Result;
use crate::ssh::buffer::Buffer;
use crate::BigNumExt;
use crate::Boxtory;
use derive_new::new;
use openssl::base64::decode_block;
use openssl::bn::BigNumContext;
use openssl::ec::PointConversionForm;
use openssl::nid::Nid;
use openssl::pkey;
use std::collections::HashMap;

#[derive(new)]
pub struct PrivateKey {
    pub key_type: String,
    pub public_key: Vec<u8>,
    pub private_key: Vec<u8>,
    pub comment: String,
}

#[derive(new)]
pub struct PublicKey {
    pub key_type: String,
    pub key: Vec<u8>,
}

// pub trait KeyParser {
//     fn parse_publickey(&self, binary: &[u8]) -> Result<PublicKey>;
//     fn parse_privatekey(&self, binary: &[u8], passphrase: Option<&[u8]>) -> Result<PrivateKey>;
//     fn add_cipher(&mut self, name: &str, cipher: Boxtory<dyn Decrypt + Send>);
// }

pub struct KeyParser {
    cipher: HashMap<String, Boxtory<dyn Decrypt + Send>>,
}

impl Default for KeyParser {
    fn default() -> Self {
        let cipher = crypt::new_decrypt_all();
        let cipher = cipher
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect::<HashMap<_, _>>();
        Self { cipher }
    }
}

impl KeyParser {
    pub fn parse_publickey(&self, binary: &[u8]) -> Result<PublicKey> {
        let content = std::str::from_utf8(binary)?;

        let parts: Vec<_> = content.trim().split(' ').collect();

        if parts.len() != 3 {
            return Err(Error::invalid_format("invaild key format"));
        }
        Ok(PublicKey::new(
            parts[0].to_string(),
            decode_block(parts[1])?,
        ))
    }

    pub fn parse_privatekey(&self, binary: &[u8], passphrase: Option<&[u8]>) -> Result<PrivateKey> {
        let invalid_key_format = || Error::invalid_format("invalid public key format");
        let content =
            std::str::from_utf8(binary).map_err(|_| Error::invalid_format("not uft-8 string"))?;
        let mut content = content.trim().to_string();

        content.retain(|v| v != '\r' && v != '\n');

        if (content.starts_with("-----BEGIN RSA PRIVATE KEY-----")
            && content.ends_with("-----END RSA PRIVATE KEY-----"))
            || (content.starts_with("-----BEGIN DSA PRIVATE KEY-----")
                && content.ends_with("-----END DSA PRIVATE KEY-----"))
            || (content.starts_with("-----BEGIN EC PRIVATE KEY-----")
                && content.ends_with("-----END EC PRIVATE KEY-----"))
        {
            let pkey = if let Some(passphrase) = passphrase {
                pkey::PKey::private_key_from_pem_passphrase(binary, passphrase)
            } else {
                pkey::PKey::private_key_from_pem(binary)
            }?;

            match pkey.id() {
                pkey::Id::RSA => {
                    let rsa = pkey.rsa()?;
                    let e = rsa.e().to_ssh_bytes();
                    let n = rsa.n().to_ssh_bytes();

                    let d = rsa.d().to_ssh_bytes();
                    let iqmp = rsa.iqmp().map(|v| v.to_ssh_bytes()).unwrap_or_default();
                    let p = rsa.p().map(|v| v.to_ssh_bytes()).unwrap_or_default();

                    let q = rsa.q().map(|v| v.to_ssh_bytes()).unwrap_or_default();

                    let public_key = make_buffer_without_header! {
                        one: "ssh-rsa",
                        one: &e,
                        one: &n
                    };

                    let private_key = make_buffer_without_header! {
                        one: "ssh-rsa",
                        one: n,
                        one: e,
                        one: d,
                        one: iqmp,
                        one: p,
                        one: q,
                    };

                    return Ok(PrivateKey::new(
                        "ssh-rsa".to_string(),
                        public_key.into_vec(),
                        private_key.into_vec(),
                        "".to_string(),
                    ));
                }
                pkey::Id::DSA => {
                    let dsa = pkey.dsa()?;
                    let p = dsa.p().to_ssh_bytes();
                    let q = dsa.q().to_ssh_bytes();
                    let g = dsa.g().to_ssh_bytes();
                    let y = dsa.pub_key().to_ssh_bytes();

                    let x = dsa.priv_key().to_ssh_bytes();

                    let public_key = make_buffer_without_header! {
                        one: "ssh-dss",
                        one: &p,
                        one: &q,
                        one: &g,
                        one: &y
                    };

                    let private_key = make_buffer_without_header! {
                        one: "ssh-dss",
                        one: p,
                        one: q,
                        one: g,
                        one: y,
                        one: x,
                    };

                    return Ok(PrivateKey::new(
                        "ssh-dss".to_string(),
                        public_key.into_vec(),
                        private_key.into_vec(),
                        "".to_string(),
                    ));
                }
                pkey::Id::EC => {
                    let ec = pkey.ec_key()?;
                    let group = ec.group();
                    let curve = group
                        .curve_name()
                        .ok_or(Error::invalid_format("No nid was found"))?;

                    let (curve, nid) = match curve {
                        Nid::X9_62_PRIME256V1 => ("ecdsa-sha2-nistp256", "nistp256"),
                        Nid::SECP384R1 => ("ecdsa-sha2-nistp384", "nistp384"),
                        Nid::SECP521R1 => ("ecdsa-sha2-nistp521", "nistp521"),
                        _ => return Err(Error::invalid_format("Invalid SSH key type")),
                    };

                    let mut ctx = BigNumContext::new()?;
                    let point = ec.public_key().to_bytes(
                        group,
                        PointConversionForm::UNCOMPRESSED,
                        &mut ctx,
                    )?;

                    let e = ec.private_key().to_ssh_bytes();

                    let private_key = make_buffer_without_header! {
                        one: curve,
                        one: nid,
                        one: &point,
                        one: e,
                    };

                    let public_key = make_buffer_without_header! {
                        one: curve,
                        one: nid,
                        one: point
                    };

                    return Ok(PrivateKey::new(
                        curve.to_string(),
                        public_key.into_vec(),
                        private_key.into_vec(),
                        "".to_string(),
                    ));
                }
                id => {
                    return Err(Error::invalid_format(format!(
                        "Unknown PEM format key: {:?}",
                        id
                    )))
                }
            }
        }

        let content = content
            .trim_start_matches("-----BEGIN OPENSSH PRIVATE KEY-----")
            .trim_end_matches("-----END OPENSSH PRIVATE KEY-----");

        let decode = decode_block(content)?;
        let decode = Buffer::from_slice(&decode);

        let key = b"openssh-key-v1\0";

        let auth_magic = decode
            .take_bytes(key.len())
            .ok_or(Error::invalid_format("invalid binary format"))?;
        if auth_magic != key {
            return Err(Error::invalid_format("unsupport key file type"));
        }

        let (_, cipher) = decode
            .take_one()
            .ok_or(Error::invalid_format("invalid binary format"))?;

        let (_, kdfname) = decode
            .take_one()
            .ok_or(Error::invalid_format("invalid binary format"))?;

        let (_, kdfopts) = decode
            .take_one()
            .ok_or(Error::invalid_format("invalid binary format"))?;

        let numofkeys = decode
            .take_u32()
            .ok_or(Error::invalid_format("invalid binary format"))?;

        if numofkeys != 1 {
            return Err(Error::invalid_format("num for keys must be 1"));
        }

        let (_, pbuf) = decode
            .take_one()
            .ok_or(Error::invalid_format("invalid binary format"))?;

        let public_key = pbuf;
        let pbuf = Buffer::from_slice(pbuf);

        let (_, keytype) = pbuf
            .take_one()
            .ok_or(Error::invalid_format("invalid binary format"))?;

        let (_, section) = decode
            .take_one()
            .ok_or(Error::invalid_format("invalid binary format"))?;

        let mut section = section.to_vec();

        match passphrase {
            Some(passphrase) => {
                if cipher != b"none" {
                    if kdfname != b"bcrypt" {
                        // In the current version only bcrtypt is legal
                        return Err(Error::invalid_format("invalid kdfname"));
                    }
                    let kdfopts = Buffer::from_slice(kdfopts);
                    let (_, salt) = kdfopts
                        .take_one()
                        .ok_or(Error::invalid_format("invalid binary format"))?;
                    let rounds = kdfopts
                        .take_u32()
                        .ok_or(Error::invalid_format("invalid binary format"))?;

                    let mut cipher = self
                        .cipher
                        .get(std::str::from_utf8(cipher)?)
                        .ok_or(Error::invalid_format("unsupport cipher type"))?
                        .create();

                    cipher.enable_increase_iv(false);

                    let mut key_and_iv = vec![0; cipher.key_len() + cipher.iv_len()];
                    bcrypt_pbkdf::bcrypt_pbkdf(passphrase, salt, rounds, &mut key_and_iv)
                        .map_err(|e| Error::invalid_format(e.to_string()))?;

                    cipher.initialize(
                        &key_and_iv[cipher.key_len()..],
                        &key_and_iv[..cipher.key_len()],
                    )?;

                    if section.len() % cipher.block_size() != 0 {
                        return Err(Error::invalid_format("invalid binary format"));
                    }

                    let mut plain_text = vec![];

                    cipher.update(&section, &mut plain_text)?;

                    if cipher.has_tag() {
                        let tag = decode
                            .take_bytes(cipher.tag_len())
                            .ok_or(Error::invalid_format("invalid binary format"))?;

                        cipher.set_authentication_tag(tag)?;
                    }

                    cipher.finalize(&mut plain_text)?;

                    section = plain_text;

                    // bcrypt_pbkdf::bcrypt_pbkdf_with_memory(passphrase, &salt, rounds, &mut [], &mut []);
                }
            }
            None => {
                if cipher != b"none" {
                    return Err(Error::invalid_format("passphrase was required"));
                }
            }
        }

        let section = Buffer::from_slice(&section);

        let checkint1 = section
            .take_u32()
            .ok_or(Error::invalid_format("invalid binary format"))?;

        let checkint2 = section
            .take_u32()
            .ok_or(Error::invalid_format("invalid binary format"))?;

        if checkint1 != checkint2 {
            return Err(Error::invalid_format("checkint1 != checkint2"));
        }
        let (private, public) = if keytype == b"ssh-ed25519" {
            let (_, pubkey1) = pbuf
                .take_one()
                .ok_or(Error::invalid_format("invalid binary format"))?;

            let (_, keytype2) = section
                .take_one()
                .ok_or(Error::invalid_format("invalid binary format"))?;

            if keytype != keytype2 {
                return Err(Error::invalid_format("invalid key format"));
            }

            let (_, pubkey2) = section
                .take_one()
                .ok_or(Error::invalid_format("invalid binary format"))?;

            if pubkey1 != pubkey2 {
                return Err(Error::invalid_format("pubkey1 != pubkey2"));
            }

            let (secret_len, secret) = section
                .take_one()
                .ok_or(Error::invalid_format("invalid binary format"))?;

            if secret_len != 64 {
                return Err(Error::invalid_format("invalid binary format"));
            }
            let pubkey3 = &secret[32..];
            if pubkey1 != pubkey3 {
                return Err(Error::invalid_format("pubkey1 != pubkey3"));
            }

            let prikey = &secret[..32];

            let (_, _comment) = section
                .take_one()
                .ok_or(Error::invalid_format("invalid binary format"))?;

            // let mut private_key = Buffer::new();
            // private_key.put_one(keytype);
            // private_key.put_one(prikey);

            let private_key = make_buffer_without_header! {
                one: keytype,
                one: prikey
            };

            (private_key.into_vec(), public_key)
        } else if keytype == b"ssh-rsa" {
            let take_one = || Result::Ok(section.take_one().ok_or_else(invalid_key_format)?.1);

            if take_one()? != keytype {
                return Err(invalid_key_format());
            }

            let n = take_one()?;
            let e = take_one()?;

            // if e1 != e2 || n1 != n2 {
            //     return Err(Error::invalid_format("invalid public key"));
            // }

            let d = take_one()?;
            let iqmp = take_one()?;
            let p = take_one()?;
            let q = take_one()?;

            // let mut prikey = Buffer::new();

            // prikey.put_one(keytype);
            // prikey.put_one(n);
            // prikey.put_one(e);
            // prikey.put_one(d);
            // prikey.put_one(iqmp);
            // prikey.put_one(p);
            // prikey.put_one(q);

            let prikey = make_buffer_without_header! {
                one: keytype,
                one: n,
                one: e,
                one: d,
                one: iqmp,
                one: p,
                one: q
            };

            (prikey.into_vec(), public_key)
        } else if keytype == b"ssh-dss" {
            let take_one = || Result::Ok(section.take_one().ok_or_else(invalid_key_format)?.1);

            if take_one()? != keytype {
                return Err(invalid_key_format());
            }

            let p = take_one()?;
            let q = take_one()?;
            let g = take_one()?;
            let y = take_one()?;
            let x = take_one()?;

            // let mut private_key = Buffer::new();
            // private_key.put_one(p);
            // private_key.put_one(q);
            // private_key.put_one(g);
            // private_key.put_one(y);
            // private_key.put_one(x);

            let private_key = make_buffer_without_header! {
                one: p,
                one: q,
                one: g,
                one: y,
                one: x
            };

            (private_key.into_vec(), public_key)
        } else if keytype.starts_with(b"ecdsa-sha2-nistp") {
            let curve = section
                .take_one()
                .ok_or(Error::invalid_format("invalid binary format"))?
                .1;

            let nid = section
                .take_one()
                .ok_or(Error::invalid_format("invalid binary format"))?
                .1;

            let point = section
                .take_one()
                .ok_or(Error::invalid_format("invalid binary format"))?
                .1;
            let e = section
                .take_one()
                .ok_or(Error::invalid_format("invalid binary format"))?
                .1;

            // let mut private_key = Buffer::new();

            // private_key.put_one(curve);
            // private_key.put_one(nid);
            // private_key.put_one(point);
            // private_key.put_one(e);

            let private_key = make_buffer_without_header! {
                one: curve,
                one: nid,
                one: point,
                one: e
            };

            (private_key.into_vec(), public_key)
        } else {
            return Err(Error::invalid_format(format!(
                "unsupport key type => {}",
                std::str::from_utf8(keytype)?
            )));
        };

        let comment = section.take_one().ok_or_else(invalid_key_format)?.1;
        let comment = std::str::from_utf8(comment)?;

        Ok(PrivateKey::new(
            std::str::from_utf8(keytype)?.to_string(),
            public.to_vec(),
            private,
            comment.to_string(),
        ))
    }

    pub fn add_cipher(&mut self, name: &str, cipher: Boxtory<dyn Decrypt + Send>) {
        self.cipher.insert(name.to_string(), cipher);
    }
}
