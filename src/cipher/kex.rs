use crate::ssh::{
    MultiplePrecisionInteger,
    buffer::Producer,
    protocol::{
        SSH_MSG_KEX_DH_GEX_GROUP, SSH_MSG_KEX_DH_GEX_INIT, SSH_MSG_KEX_DH_GEX_REPLY,
        SSH_MSG_KEX_DH_GEX_REQUEST, SSH_MSG_KEX_ECDH_INIT, SSH_MSG_KEX_ECDH_REPLY,
        SSH_MSG_KEXDH_INIT, SSH_MSG_KEXDH_REPLY,
    },
};
use libcrux_ml_kem::{MlKemCiphertext, MlKemKeyPair};
use openssl::{
    bn::{BigNum, BigNumContext},
    derive::Deriver,
    dh::Dh,
    ec::{EcGroup, EcKey, EcPoint, PointConversionForm},
    md::{Md, MdRef},
    md_ctx::MdCtx,
    nid::Nid,
    pkey::{Id, PKey, Private},
    pkey_ctx::PkeyCtx,
};
use rand::RngExt;
use snafu::{OptionExt, ResultExt};

use super::Factory;
use crate::error::{Result, builder};
use indexmap::IndexMap;

algo_list!(
    all,
    new_all,
    new_kex_by_name,
    dyn KeyExchange + Send,
    "mlkem768x25519-sha256" => MlKem768X25519::new(),
    "curve25519-sha256@libssh.org" => Curve25519Impl::curve25519_sha256_libssh(),
    "curve25519-sha256" => Curve25519Impl::curve25519_sha256(),
    "ecdh-sha2-nistp256" => EllipticCurveDiffieHellmanImpl::ecdh_sha2_nistp256(),
    "ecdh-sha2-nistp384" => EllipticCurveDiffieHellmanImpl::ecdh_sha2_nistp384(),
    "ecdh-sha2-nistp521" => EllipticCurveDiffieHellmanImpl::ecdh_sha2_nistp521(),
    "diffie-hellman-group14-sha256" => StandardDiffieHellmanImpl::dh_group14_sha256(),
    "diffie-hellman-group16-sha512" => StandardDiffieHellmanImpl::dh_group16_sha512(),
    "diffie-hellman-group14-sha1" => StandardDiffieHellmanImpl::dh_group14_sha1(),
    "diffie-hellman-group18-sha512" => StandardDiffieHellmanImpl::dh_group18_sha512(),
    "diffie-hellman-group-exchange-sha256" => ExchangeDiffieHellmanImpl::sha256(),
    "diffie-hellman-group-exchange-sha1" => ExchangeDiffieHellmanImpl::sha1(),
    "diffie-hellman-group1-sha1" => StandardDiffieHellmanImpl::dh_group1_sha1(),
);

pub struct Information<'a> {
    pub client_version: &'a str,
    pub server_version: &'a str,
    pub client_kex_init: &'a [u8],
    pub server_kex_init: &'a [u8],
    pub server_host_key: &'a [u8],
    pub client_public_key: &'a [u8],
    pub server_public_key: &'a [u8],
    pub secret_key: &'a [u8],
}

// pub enum Algorithm {
//     Standard(Box<dyn StandardDiffieHellman + Send>),
//     Exchange(Box<dyn ExchangeDiffieHellman + Send>),
//     EllipticCurve(Box<dyn EllipticCurveDiffieHellman + Send>),
//     Curve25519(Box<dyn Curve25519 + Send>),
//     Streamlined(Box<dyn Streamlined + Send>),
//     Hybrid(Box<dyn Hybrid + Send>),
// }

// impl Algorithm {
//     pub fn as_super(&self) -> &dyn KeyExchange {
//         match self {
//             Algorithm::Standard(standard_diffie_hellman) => &**standard_diffie_hellman,
//             Algorithm::Exchange(exchange_diffie_hellman) => &**exchange_diffie_hellman,
//             Algorithm::EllipticCurve(elliptic_curve_diffie_hellman) => &**elliptic_curve_diffie_hellman,
//             Algorithm::Curve25519(curve25519) => &**curve25519,
//             Algorithm::Streamlined(streamlined) => &**streamlined,
//             Algorithm::Hybrid(hybrid) => &**hybrid,
//         }
//     }
// }

// pub trait KeyExchange {
//     fn name(&self) -> &str;
//     fn generate_key(&mut self) -> Result<Vec<u8>>;
//     fn compute_secret_key(&mut self, server_public_key: &[u8]) -> Result<Vec<u8>>;
//     fn compute_hash(&mut self, info: Information<'_>) -> Result<Vec<u8>>;
//     fn compute_communicate_key(
//         &self,
//         secret_key: &[u8],
//         session_id: &[u8],
//         hash: &[u8],
//         version: u8,
//         len: usize,
//     ) -> Result<Vec<u8>>;
// }

pub trait KeyExchange {
    fn name(&self) -> &str;
    fn generate_key(&mut self) -> Result<Vec<u8>>;
    fn compute_secret_key(&mut self, server_public_key: &[u8]) -> Result<Vec<u8>>;
    fn compute_hash(&mut self, info: Information<'_>) -> Result<Vec<u8>>;
    fn compute_communicate_key(
        &self,
        secret_key: &[u8],
        session_id: &[u8],
        hash: &[u8],
        version: u8,
        len: usize,
    ) -> Result<Vec<u8>>;
    fn request_code(&self) -> u8;
    fn response_code(&self) -> u8;
    fn exchange(&mut self) -> Option<&mut (dyn Exchange + Send)>;
}

pub trait Exchange {
    fn max(&self) -> u32;
    fn min(&self) -> u32;
    fn number_of_bits(&self) -> u32;
    fn set_recommended_number_of_bits(&mut self, bits: u32);
    fn initialize(&mut self, p: &[u8], g: &[u8]) -> Result<()>;
    fn request_code(&self) -> u8;
    fn response_code(&self) -> u8;
}

// pub trait StandardDiffieHellman: KeyExchange {}
// pub trait ExchangeDiffieHellman: KeyExchange {
//     fn max(&self) -> u32;
//     fn min(&self) -> u32;
//     fn number_of_bits(&self) -> u32;
//     fn set_recommended_number_of_bits(&mut self, bits: u32);
//     fn initialize(&mut self, p: &[u8], g: &[u8]) -> Result<()>;
// }

// pub trait EllipticCurveDiffieHellman: KeyExchange {}
// pub trait Curve25519: KeyExchange {}
// pub trait Streamlined: KeyExchange {}
// pub trait Hybrid: KeyExchange {}

pub struct EllipticCurveDiffieHellmanImpl {
    name: &'static str,
    nid: Nid,
    hasher: &'static MdRef,
    private_key: Option<PKey<openssl::pkey::Private>>,
}

impl EllipticCurveDiffieHellmanImpl {
    fn new(name: &'static str, nid: Nid, hasher: &'static MdRef) -> Self {
        Self {
            name,
            nid,
            hasher,
            private_key: None,
        }
    }

    fn ecdh_sha2_nistp256() -> Self {
        Self::new("ecdh-sha2-nistp256", Nid::X9_62_PRIME256V1, Md::sha256())
    }

    fn ecdh_sha2_nistp384() -> Self {
        Self::new("ecdh-sha2-nistp384", Nid::SECP384R1, Md::sha384())
    }

    fn ecdh_sha2_nistp521() -> Self {
        Self::new("ecdh-sha2-nistp521", Nid::SECP521R1, Md::sha512())
    }
}

impl KeyExchange for EllipticCurveDiffieHellmanImpl {
    fn name(&self) -> &str {
        self.name
    }

    fn generate_key(&mut self) -> Result<Vec<u8>> {
        let group = EcGroup::from_curve_name(self.nid).context(builder::OpenSSL)?;
        let ec_key = EcKey::generate(&group).context(builder::OpenSSL)?;
        let mut ctx = BigNumContext::new().context(builder::OpenSSL)?;
        let pubkey = ec_key
            .public_key()
            .to_bytes(&group, PointConversionForm::UNCOMPRESSED, &mut ctx)
            .context(builder::OpenSSL)?;
        self.private_key = Some(ec_key.try_into().context(builder::OpenSSL)?);
        Ok(pubkey)
    }

    fn compute_secret_key(&mut self, server_public_key: &[u8]) -> Result<Vec<u8>> {
        let private_key = self.private_key.take().context(builder::InvalidOperation {
            detail: "Genreate key first",
        })?;
        let group = EcGroup::from_curve_name(self.nid).context(builder::OpenSSL)?;
        let mut ctx = BigNumContext::new().context(builder::OpenSSL)?;
        let server_point =
            EcPoint::from_bytes(&group, server_public_key, &mut ctx).context(builder::OpenSSL)?;
        let server_key = EcKey::from_public_key(&group, &server_point).context(builder::OpenSSL)?;
        let server_pkey: PKey<_> = server_key.try_into().context(builder::OpenSSL)?;

        let mut deriver = Deriver::new(&private_key).context(builder::OpenSSL)?;
        deriver.set_peer(&server_pkey).context(builder::OpenSSL)?;
        Ok(deriver
            .derive_to_vec()
            .context(builder::OpenSSL)?
            .into_integer())
    }

    fn compute_hash(&mut self, info: Information<'_>) -> Result<Vec<u8>> {
        let mut producer = Producer::default();

        let client_version = info.client_version;
        let server_version = info.server_version;

        producer.put_one(client_version);
        producer.put_one(server_version);
        producer.put_one(info.client_kex_init);
        producer.put_one(info.server_kex_init);

        producer.put_one(info.server_host_key);
        producer.put_one(info.client_public_key);
        producer.put_one(info.server_public_key);

        producer.put_one(info.secret_key); //  tbd

        let mut ctx = MdCtx::new().context(builder::OpenSSL)?;

        ctx.digest_init(self.hasher).context(builder::OpenSSL)?;

        ctx.digest_update(producer.as_bytes())
            .context(builder::OpenSSL)?;

        let mut output = vec![0; ctx.size()];

        ctx.digest_final(&mut output).context(builder::OpenSSL)?;

        Ok(output)
    }

    fn compute_communicate_key(
        &self,
        secret_key: &[u8],
        session_id: &[u8],
        hash: &[u8],
        version: u8,
        len: usize,
    ) -> Result<Vec<u8>> {
        compute_keys(self.hasher, secret_key, session_id, hash, version, len)
    }

    fn request_code(&self) -> u8 {
        SSH_MSG_KEX_ECDH_INIT
    }

    fn response_code(&self) -> u8 {
        SSH_MSG_KEX_ECDH_REPLY
    }

    fn exchange(&mut self) -> Option<&mut (dyn Exchange + Send)> {
        None
    }
}

pub struct ExchangeDiffieHellmanImpl {
    name: &'static str,
    min: u32,
    number_of_bits: u32,
    max: u32,
    hasher: &'static MdRef,
    ctx: Option<BigNumContext>,
    key: Option<Dh<Private>>,
}

impl ExchangeDiffieHellmanImpl {
    fn new(
        name: &'static str,
        min: u32,
        number_of_bits: u32,
        max: u32,
        hasher: &'static MdRef,
    ) -> Self {
        Self {
            name,
            min,
            number_of_bits,
            max,
            hasher,
            ctx: None,
            key: None,
        }
    }

    fn sha1() -> Self {
        Self::new(
            "diffie-hellman-group-exchange-sha1",
            2048,
            4096,
            8192,
            Md::sha1(),
        )
    }
    fn sha256() -> Self {
        Self::new(
            "diffie-hellman-group-exchange-sha256",
            2048,
            4096,
            8192,
            Md::sha256(),
        )
    }
}

impl Exchange for ExchangeDiffieHellmanImpl {
    fn max(&self) -> u32 {
        self.max
    }

    fn min(&self) -> u32 {
        self.min
    }

    fn number_of_bits(&self) -> u32 {
        self.number_of_bits
    }

    fn set_recommended_number_of_bits(&mut self, bits: u32) {
        if bits >= self.min && bits <= self.max {
            self.number_of_bits = bits;
        }
    }

    fn initialize(&mut self, p: &[u8], g: &[u8]) -> Result<()> {
        let p = BigNum::from_slice(p).context(builder::OpenSSL)?;

        let bits = p.num_bits();

        if bits < self.min as i32 || bits > self.max as i32 {
            super::InvalidPrimeSnafu.fail()?;
        }

        let g = BigNum::from_slice(g).context(builder::OpenSSL)?;

        let dh = Dh::from_pqg(p, None, g).context(builder::OpenSSL)?;

        // Unable to call DH_set_length to limit key length
        // Maybe slower than other ssh clients, but safer

        let key = dh.generate_key().context(builder::OpenSSL)?;

        // let private = key.private_key().to_vec();

        self.key = Some(key);
        Ok(())
    }

    fn request_code(&self) -> u8 {
        SSH_MSG_KEX_DH_GEX_REQUEST
    }

    fn response_code(&self) -> u8 {
        SSH_MSG_KEX_DH_GEX_GROUP
    }
}
impl KeyExchange for ExchangeDiffieHellmanImpl {
    fn name(&self) -> &str {
        self.name
    }
    fn generate_key(&mut self) -> Result<Vec<u8>> {
        let key = self.key.as_ref().context(builder::InvalidOperation {
            detail: "Initialize first",
        })?;

        Ok(key.public_key().to_integer())
    }
    // compute shared secret key from server public key and client_private key that genrated by generate_key
    fn compute_secret_key(&mut self, server_public_key: &[u8]) -> Result<Vec<u8>> {
        let key = self.key.as_ref().context(builder::InvalidOperation {
            detail: "Generate key first",
        })?;

        let client_private_key = key.private_key();

        let server_public_key = BigNum::from_slice(server_public_key).context(builder::OpenSSL)?;

        let mut secret = BigNum::new().context(builder::OpenSSL)?;

        if self.ctx.is_none() {
            self.ctx = Some(BigNumContext::new().context(builder::OpenSSL)?);
        }

        secret
            .mod_exp(
                &server_public_key,
                client_private_key,
                key.prime_p(),
                self.ctx.as_mut().unwrap(),
            )
            .context(builder::OpenSSL)?;

        Ok(secret.to_integer())
    }

    fn compute_hash(&mut self, info: Information<'_>) -> Result<Vec<u8>> {
        let mut producer = Producer::default();

        let key = self.key.as_ref().context(builder::InvalidOperation {
            detail: "Generate key first",
        })?;

        producer.put_one(info.client_version);
        producer.put_one(info.server_version);
        producer.put_one(info.client_kex_init);
        producer.put_one(info.server_kex_init);

        producer.put_one(info.server_host_key);

        producer.put_u32(self.min);
        producer.put_u32(self.number_of_bits);
        producer.put_u32(self.max);

        producer.put_one(key.prime_p().to_integer());
        producer.put_one(key.generator().to_integer());
        producer.put_one(info.client_public_key);

        producer.put_one(info.server_public_key);

        producer.put_one(info.secret_key); //  tbd

        let mut ctx = MdCtx::new().context(builder::OpenSSL)?;

        ctx.digest_init(self.hasher).context(builder::OpenSSL)?;

        ctx.digest_update(producer.as_bytes())
            .context(builder::OpenSSL)?;

        let mut output = vec![0; ctx.size()];

        ctx.digest_final(&mut output).context(builder::OpenSSL)?;

        Ok(output)
    }

    fn compute_communicate_key(
        &self,
        secret_key: &[u8],
        session_id: &[u8],
        hash: &[u8],
        version: u8,
        len: usize,
    ) -> Result<Vec<u8>> {
        compute_keys(self.hasher, secret_key, session_id, hash, version, len)
    }

    fn request_code(&self) -> u8 {
        SSH_MSG_KEX_DH_GEX_INIT
    }

    fn response_code(&self) -> u8 {
        SSH_MSG_KEX_DH_GEX_REPLY
    }

    fn exchange(&mut self) -> Option<&mut (dyn Exchange + Send)> {
        Some(self)
    }
}

pub struct StandardDiffieHellmanImpl<'a> {
    name: &'static str,
    p: &'a [u8],
    g: u32,
    ctx: Option<BigNumContext>,
    hasher: &'static MdRef,
    key: Option<Dh<Private>>,
}
impl StandardDiffieHellmanImpl<'static> {
    fn new(name: &'static str, p: &'static [u8], g: u32, hasher: &'static MdRef) -> Self {
        Self {
            name,
            p,
            g,
            ctx: None,
            hasher,
            key: None,
        }
    }

    fn dh_group1_sha1() -> Self {
        Self::new(
            "diffie-hellman-group1-sha1",
            &value::P_GROUP1_VALUE,
            value::G_VALUE,
            Md::sha1(),
        )
    }
    fn dh_group14_sha1() -> Self {
        Self::new(
            "diffie-hellman-group14-sha1",
            &value::P_GROUP14_VALUE,
            value::G_VALUE,
            Md::sha1(),
        )
    }
    fn dh_group14_sha256() -> Self {
        Self::new(
            "diffie-hellman-group14-sha256",
            &value::P_GROUP14_VALUE,
            value::G_VALUE,
            Md::sha256(),
        )
    }
    // fn dh_group15_sha512() -> Self {
    //     Self::new(
    //         "diffie-hellman-group15-sha512",
    //         &value::P_GROUP15_VALUE,
    //         2,
    //         Md::sha512(),
    //     )
    // }
    fn dh_group16_sha512() -> Self {
        Self::new(
            "diffie-hellman-group16-sha512",
            &value::P_GROUP16_VALUE,
            value::G_VALUE,
            Md::sha512(),
        )
    }
    // fn dh_group16_sha256() -> Self {
    //     Self::new(
    //         "diffie-hellman-group16-sha256",
    //         &value::P_GROUP16_VALUE,
    //         2,
    //         Md::sha256(),
    //     )
    // }
    // fn dh_group17_sha512() -> Self {
    //     Self::new(
    //         "diffie-hellman-group17-sha512",
    //         &value::P_GROUP17_VALUE,
    //         2,
    //         Md::sha512(),
    //     )
    // }
    fn dh_group18_sha512() -> Self {
        Self::new(
            "diffie-hellman-group18-sha512",
            &value::P_GROUP18_VALUE,
            value::G_VALUE,
            Md::sha512(),
        )
    }
}

impl<'a> KeyExchange for StandardDiffieHellmanImpl<'a> {
    fn generate_key(&mut self) -> Result<Vec<u8>> {
        let p = BigNum::from_slice(self.p).context(builder::OpenSSL)?;
        let g = BigNum::from_u32(self.g).context(builder::OpenSSL)?;

        let dh = Dh::from_pqg(p, None, g).context(builder::OpenSSL)?;

        // Unable to call DH_set_length to limit key length
        // Maybe slower than other ssh clients, but safer

        let key = dh.generate_key().context(builder::OpenSSL)?;

        let public = key.public_key().to_integer();
        // let private = key.private_key().to_vec();

        self.key = Some(key);

        Ok(public)
    }

    fn compute_secret_key(&mut self, server_public_key: &[u8]) -> Result<Vec<u8>> {
        let client_private_key = self
            .key
            .as_ref()
            .context(builder::InvalidOperation {
                detail: "Generate key first",
            })?
            .private_key();

        let server_public_key = BigNum::from_slice(server_public_key).context(builder::OpenSSL)?;

        let mut secret = BigNum::new().context(builder::OpenSSL)?;

        // let client_private_key = BigNum::from_slice(client_private_key)?;

        let p = BigNum::from_slice(self.p).context(builder::OpenSSL)?;

        if self.ctx.is_none() {
            let ctx = BigNumContext::new().context(builder::OpenSSL)?;
            self.ctx = Some(ctx);
        }

        secret
            .mod_exp(
                &server_public_key,
                client_private_key,
                &p,
                self.ctx.as_mut().unwrap(),
            )
            .context(builder::OpenSSL)?;

        Ok(secret.to_integer())
    }

    fn compute_hash(&mut self, info: Information<'_>) -> Result<Vec<u8>> {
        let mut producer = Producer::default();

        producer.put_one(info.client_version);
        producer.put_one(info.server_version);
        producer.put_one(info.client_kex_init);
        producer.put_one(info.server_kex_init);

        producer.put_one(info.server_host_key);
        producer.put_one(info.client_public_key);
        producer.put_one(info.server_public_key);

        producer.put_one(info.secret_key); //  tbd

        let mut ctx = MdCtx::new().context(builder::OpenSSL)?;

        ctx.digest_init(self.hasher).context(builder::OpenSSL)?;

        ctx.digest_update(producer.as_bytes())
            .context(builder::OpenSSL)?;

        let mut output = vec![0; ctx.size()];

        ctx.digest_final(&mut output).context(builder::OpenSSL)?;

        Ok(output)
    }

    fn name(&self) -> &str {
        self.name
    }

    fn compute_communicate_key(
        &self,
        secret_key: &[u8],
        session_id: &[u8],
        hash: &[u8],
        version: u8,
        len: usize,
    ) -> Result<Vec<u8>> {
        compute_keys(self.hasher, secret_key, session_id, hash, version, len)
    }

    fn request_code(&self) -> u8 {
        SSH_MSG_KEXDH_INIT
    }

    fn response_code(&self) -> u8 {
        SSH_MSG_KEXDH_REPLY
    }

    fn exchange(&mut self) -> Option<&mut (dyn Exchange + Send)> {
        None
    }

    // fn compute_communicate_key(
    //     &self,
    //     secret_key: &[u8],
    //     session_id: &[u8],
    //     hash: &[u8],
    //     version: u8,
    //     len: usize,
    // ) -> super::Result<Vec<u8>> {
    //     let mut output = vec![];

    //     let mut ctx = MdCtx::new()?;
    //     ctx.digest_init(self.hasher)?;

    //     ctx.digest_update(secret_key)?;
    //     ctx.digest_update(hash)?;
    //     ctx.digest_update(&[version])?;
    //     ctx.digest_update(session_id)?;

    //     output.resize(ctx.size(), 0);

    //     ctx.digest_final(&mut output)?;
    //     ctx.digest_init(self.hasher)?;

    //     while output.len() < len {
    //         ctx.digest_update(secret_key)?;
    //         ctx.digest_update(hash)?;
    //         ctx.digest_update(&output)?;

    //         let l = output.len();

    //         output.resize(l + ctx.size(), 0);

    //         ctx.digest_final(&mut output[l..])?;
    //         ctx.digest_init(self.hasher)?;
    //     }

    //     output.truncate(len);

    //     Ok(output)
    // }

    // fn compute_session_key(&self, secret: &[u8], hash: &[u8]) -> super::Result<Vec<u8>> {
    //     todo!()
    // }
}

pub struct Curve25519Impl {
    name: &'static str,
    hasher: &'static MdRef,
    key: Option<PKey<openssl::pkey::Private>>,
}

impl Curve25519Impl {
    fn new(name: &'static str, hasher: &'static MdRef) -> Self {
        Self {
            name,
            hasher,
            key: None,
        }
    }
    fn curve25519_sha256() -> Self {
        Self::new("curve25519-sha256", Md::sha256())
    }

    fn curve25519_sha256_libssh() -> Self {
        Self::new("curve25519-sha256@libssh.org", Md::sha256())
    }
}

impl KeyExchange for Curve25519Impl {
    fn name(&self) -> &str {
        self.name
    }

    fn generate_key(&mut self) -> Result<Vec<u8>> {
        let mut ctx = PkeyCtx::new_id(Id::X25519).context(builder::OpenSSL)?;
        ctx.keygen_init().context(builder::OpenSSL)?;
        let key = ctx.keygen().context(builder::OpenSSL)?;
        let public_key = key.raw_public_key().context(builder::OpenSSL)?;
        self.key = Some(key);
        Ok(public_key)
    }

    fn compute_secret_key(&mut self, server_public_key: &[u8]) -> Result<Vec<u8>> {
        let private_key = self.key.as_ref().context(builder::InvalidOperation {
            detail: "Generate key first",
        })?;
        let server_public = PKey::public_key_from_raw_bytes(server_public_key, Id::X25519)
            .context(builder::OpenSSL)?;

        let mut ctx = PkeyCtx::new(&private_key).context(builder::OpenSSL)?;
        ctx.derive_init().context(builder::OpenSSL)?;
        ctx.derive_set_peer(&server_public)
            .context(builder::OpenSSL)?;
        let size = ctx.derive(None).context(builder::OpenSSL)?;
        let mut secret_key = vec![0; size];
        ctx.derive(Some(&mut secret_key))
            .context(builder::OpenSSL)?;
        Ok(secret_key.into_integer())
    }

    fn compute_hash(&mut self, info: Information<'_>) -> Result<Vec<u8>> {
        let mut producer = Producer::default();

        let client_version = info.client_version;
        let server_version = info.server_version;

        producer.put_one(client_version);
        producer.put_one(server_version);
        producer.put_one(info.client_kex_init);
        producer.put_one(info.server_kex_init);

        producer.put_one(info.server_host_key);
        producer.put_one(info.client_public_key);
        producer.put_one(info.server_public_key);

        producer.put_one(info.secret_key); //  tbd

        let mut ctx = MdCtx::new().context(builder::OpenSSL)?;

        ctx.digest_init(self.hasher).context(builder::OpenSSL)?;

        ctx.digest_update(producer.as_bytes())
            .context(builder::OpenSSL)?;

        let mut output = vec![0; ctx.size()];

        ctx.digest_final(&mut output).context(builder::OpenSSL)?;

        Ok(output)
    }

    fn compute_communicate_key(
        &self,
        secret_key: &[u8],
        session_id: &[u8],
        hash: &[u8],
        version: u8,
        len: usize,
    ) -> Result<Vec<u8>> {
        compute_keys(self.hasher, secret_key, session_id, hash, version, len)
    }

    fn request_code(&self) -> u8 {
        SSH_MSG_KEX_ECDH_INIT
    }

    fn response_code(&self) -> u8 {
        SSH_MSG_KEX_ECDH_REPLY
    }

    fn exchange(&mut self) -> Option<&mut (dyn Exchange + Send)> {
        None
    }
}

pub struct MlKem768X25519 {
    mlkem: Option<MlKemKeyPair<2400, 1184>>,
    x25519: Option<PKey<Private>>,
}

impl MlKem768X25519 {
    fn new() -> Self {
        Self {
            mlkem: None,
            x25519: None,
        }
    }
}

impl KeyExchange for MlKem768X25519 {
    fn name(&self) -> &str {
        "mlkem768x25519-sha256"
    }

    fn generate_key(&mut self) -> Result<Vec<u8>> {
        let mut bytes = [0u8; 64];
        let mut rng = rand::rng();
        rng.fill(&mut bytes);
        let key = libcrux_ml_kem::mlkem768::portable::generate_key_pair(bytes);

        let mut public_key = key.public_key().as_slice().to_vec();

        self.mlkem = Some(key);
        {
            let mut ctx = PkeyCtx::new_id(Id::X25519).context(builder::OpenSSL)?;
            ctx.keygen_init().context(builder::OpenSSL)?;
            let key = ctx.keygen().context(builder::OpenSSL)?;
            public_key.extend(key.raw_public_key().context(builder::OpenSSL)?);

            self.x25519 = Some(key);
        }

        Ok(public_key)
    }

    fn compute_secret_key(&mut self, server_public_key: &[u8]) -> Result<Vec<u8>> {
        let mlkem = self.mlkem.as_ref().context(builder::InvalidOperation {
            detail: "mlkem key not generated",
        })?;
        // assert_eq!(server_public_key.len(), 1088 + 32);
        if server_public_key.len() != 1088 + 32 {
            return Err(super::KeyLengthMismatchSnafu.build().into());
        }
        let s: [u8; 1088] = server_public_key[..1088].try_into().unwrap();
        let mlkem_ciphertext: MlKemCiphertext<1088> = MlKemCiphertext::from(s);
        let secret_key1 =
            libcrux_ml_kem::mlkem768::portable::decapsulate(mlkem.private_key(), &mlkem_ciphertext);
        let secret_key2 = {
            let private_key = self.x25519.as_ref().context(builder::InvalidOperation {
                detail: "Generate key first",
            })?;
            let server_public =
                PKey::public_key_from_raw_bytes(&server_public_key[1088..], Id::X25519)
                    .context(builder::OpenSSL)?;

            let mut ctx = PkeyCtx::new(&private_key).context(builder::OpenSSL)?;
            ctx.derive_init().context(builder::OpenSSL)?;
            ctx.derive_set_peer(&server_public)
                .context(builder::OpenSSL)?;
            let size = ctx.derive(None).context(builder::OpenSSL)?;
            let mut secret_key = vec![0; size];
            ctx.derive(Some(&mut secret_key))
                .context(builder::OpenSSL)?;
            secret_key
        };

        let mut hasher = MdCtx::new().context(builder::OpenSSL)?;

        hasher.digest_init(Md::sha256()).context(builder::OpenSSL)?;
        hasher
            .digest_update(&secret_key1)
            .context(builder::OpenSSL)?;

        hasher
            .digest_update(&secret_key2)
            .context(builder::OpenSSL)?;

        let mut output = vec![0; hasher.size()];
        hasher.digest_final(&mut output).context(builder::OpenSSL)?;

        Ok(output)
    }

    fn compute_hash(&mut self, info: Information<'_>) -> Result<Vec<u8>> {
        let mut producer = Producer::default();

        let client_version = info.client_version;
        let server_version = info.server_version;

        producer.put_one(client_version);
        producer.put_one(server_version);
        producer.put_one(info.client_kex_init);
        producer.put_one(info.server_kex_init);

        producer.put_one(info.server_host_key);
        producer.put_one(info.client_public_key);
        producer.put_one(info.server_public_key);

        producer.put_one(info.secret_key); //  tbd

        let mut ctx = MdCtx::new().context(builder::OpenSSL)?;

        ctx.digest_init(Md::sha256()).context(builder::OpenSSL)?;

        ctx.digest_update(producer.as_bytes())
            .context(builder::OpenSSL)?;

        let mut output = vec![0; ctx.size()];

        ctx.digest_final(&mut output).context(builder::OpenSSL)?;

        Ok(output)
    }

    fn compute_communicate_key(
        &self,
        secret_key: &[u8],
        session_id: &[u8],
        hash: &[u8],
        version: u8,
        len: usize,
    ) -> Result<Vec<u8>> {
        compute_keys(Md::sha256(), secret_key, session_id, hash, version, len)
    }

    fn request_code(&self) -> u8 {
        SSH_MSG_KEX_ECDH_INIT
    }

    fn response_code(&self) -> u8 {
        SSH_MSG_KEX_ECDH_REPLY
    }

    fn exchange(&mut self) -> Option<&mut (dyn Exchange + Send)> {
        None
    }
}

fn compute_keys(
    md: &'static MdRef,
    secret_key: &[u8],
    session_id: &[u8],
    hash: &[u8],
    version: u8,
    len: usize,
) -> Result<Vec<u8>> {
    let mut output = vec![];

    let mut ctx = MdCtx::new().context(builder::OpenSSL)?;
    ctx.digest_init(md).context(builder::OpenSSL)?;

    ctx.digest_update(secret_key).context(builder::OpenSSL)?;
    ctx.digest_update(hash).context(builder::OpenSSL)?;
    ctx.digest_update(&[version]).context(builder::OpenSSL)?;
    ctx.digest_update(session_id).context(builder::OpenSSL)?;

    output.resize(ctx.size(), 0);

    ctx.digest_final(&mut output).context(builder::OpenSSL)?;
    ctx.digest_init(md).context(builder::OpenSSL)?;

    while output.len() < len {
        ctx.digest_update(secret_key).context(builder::OpenSSL)?;
        ctx.digest_update(hash).context(builder::OpenSSL)?;
        ctx.digest_update(&output).context(builder::OpenSSL)?;

        let l = output.len();

        output.resize(l + ctx.size(), 0);

        ctx.digest_final(&mut output[l..])
            .context(builder::OpenSSL)?;
        ctx.digest_init(md).context(builder::OpenSSL)?;
    }

    output.truncate(len);

    Ok(output)
}

mod value {
    const fn is_hex_ws(b: u8) -> bool {
        matches!(b, b' ' | b'\n' | b'\r' | b'\t')
    }

    const fn hex_val(b: u8) -> u8 {
        match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => panic!("invalid hex character"),
        }
    }

    const fn hex_len(s: &str) -> usize {
        let bytes = s.as_bytes();
        let mut i = 0;
        let mut n = 0;

        while i < bytes.len() {
            let b = bytes[i];

            if !is_hex_ws(b) {
                let _ = hex_val(b);
                n += 1;
            }

            i += 1;
        }

        if n % 2 != 0 {
            panic!("hex string must contain an even number of digits");
        }

        n / 2
    }

    const fn parse_hex<const N: usize>(s: &str) -> [u8; N] {
        let bytes = s.as_bytes();
        let mut out = [0u8; N];

        let mut i = 0;
        let mut j = 0;

        let mut high = 0u8;
        let mut has_high = false;

        while i < bytes.len() {
            let b = bytes[i];

            if !is_hex_ws(b) {
                let v = hex_val(b);

                if has_high {
                    out[j] = (high << 4) | v;
                    j += 1;
                    has_high = false;
                } else {
                    high = v;
                    has_high = true;
                }
            }

            i += 1;
        }

        if has_high {
            panic!("hex string must contain an even number of digits");
        }

        out
    }

    // macro_rules! hex_bytes {
    //     ($s:literal) => {{
    //         const N: usize = hex_len($s);
    //         parse_hex::<N>($s)
    //     }};
    // }
    macro_rules! const_hex {
        ($vis:vis const $name:ident = $s:literal;) => {
            $vis const $name: [u8; hex_len($s)] = parse_hex::<{ hex_len($s) }>($s);
        };
    }

    const_hex!(
        pub const P_GROUP1_VALUE = r#"
            FFFFFFFF FFFFFFFF C90FDAA2 2168C234 C4C6628B 80DC1CD1
            29024E08 8A67CC74 020BBEA6 3B139B22 514A0879 8E3404DD
            EF9519B3 CD3A431B 302B0A6D F25F1437 4FE1356D 6D51C245
            E485B576 625E7EC6 F44C42E9 A637ED6B 0BFF5CB6 F406B7ED
            EE386BFB 5A899FA5 AE9F2411 7C4B1FE6 49286651 ECE65381
            FFFFFFFF FFFFFFFF
        "#;
    );
    // const_hex!(
    //     pub const P_GROUP2_VALUE = r#"
    //     FFFFFFFF FFFFFFFF C90FDAA2 2168C234 C4C6628B 80DC1CD1
    //     29024E08 8A67CC74 020BBEA6 3B139B22 514A0879 8E3404DD
    //     EF9519B3 CD3A431B 302B0A6D F25F1437 4FE1356D 6D51C245
    //     E485B576 625E7EC6 F44C42E9 A637ED6B 0BFF5CB6 F406B7ED
    //     EE386BFB 5A899FA5 AE9F2411 7C4B1FE6 49286651 ECE65381
    //     FFFFFFFF FFFFFFFF
    //     "#;
    // );
    // const_hex!(
    //     pub const P_GROUP5_VALUE = r#"
    //     FFFFFFFF FFFFFFFF C90FDAA2 2168C234 C4C6628B 80DC1CD1
    //     29024E08 8A67CC74 020BBEA6 3B139B22 514A0879 8E3404DD
    //     EF9519B3 CD3A431B 302B0A6D F25F1437 4FE1356D 6D51C245
    //     E485B576 625E7EC6 F44C42E9 A637ED6B 0BFF5CB6 F406B7ED
    //     EE386BFB 5A899FA5 AE9F2411 7C4B1FE6 49286651 ECE45B3D
    //     C2007CB8 A163BF05 98DA4836 1C55D39A 69163FA8 FD24CF5F
    //     83655D23 DCA3AD96 1C62F356 208552BB 9ED52907 7096966D
    //     670C354E 4ABC9804 F1746C08 CA237327 FFFFFFFF FFFFFFFF
    //     "#;
    // );
    const_hex!(
        pub const P_GROUP14_VALUE = r#"
            FFFFFFFF FFFFFFFF C90FDAA2 2168C234 C4C6628B 80DC1CD1
            29024E08 8A67CC74 020BBEA6 3B139B22 514A0879 8E3404DD
            EF9519B3 CD3A431B 302B0A6D F25F1437 4FE1356D 6D51C245
            E485B576 625E7EC6 F44C42E9 A637ED6B 0BFF5CB6 F406B7ED
            EE386BFB 5A899FA5 AE9F2411 7C4B1FE6 49286651 ECE45B3D
            C2007CB8 A163BF05 98DA4836 1C55D39A 69163FA8 FD24CF5F
            83655D23 DCA3AD96 1C62F356 208552BB 9ED52907 7096966D
            670C354E 4ABC9804 F1746C08 CA18217C 32905E46 2E36CE3B
            E39E772C 180E8603 9B2783A2 EC07A28F B5C55DF0 6F4C52C9
            DE2BCBF6 95581718 3995497C EA956AE5 15D22618 98FA0510
            15728E5A 8AACAA68 FFFFFFFF FFFFFFFF
        "#;
    );
    // const_hex!(
    //     pub const P_GROUP15_VALUE = r#"
    //     FFFFFFFF FFFFFFFF C90FDAA2 2168C234 C4C6628B 80DC1CD1
    //     29024E08 8A67CC74 020BBEA6 3B139B22 514A0879 8E3404DD
    //     EF9519B3 CD3A431B 302B0A6D F25F1437 4FE1356D 6D51C245
    //     E485B576 625E7EC6 F44C42E9 A637ED6B 0BFF5CB6 F406B7ED
    //     EE386BFB 5A899FA5 AE9F2411 7C4B1FE6 49286651 ECE45B3D
    //     C2007CB8 A163BF05 98DA4836 1C55D39A 69163FA8 FD24CF5F
    //     83655D23 DCA3AD96 1C62F356 208552BB 9ED52907 7096966D
    //     670C354E 4ABC9804 F1746C08 CA18217C 32905E46 2E36CE3B
    //     E39E772C 180E8603 9B2783A2 EC07A28F B5C55DF0 6F4C52C9
    //     DE2BCBF6 95581718 3995497C EA956AE5 15D22618 98FA0510
    //     15728E5A 8AAAC42D AD33170D 04507A33 A85521AB DF1CBA64
    //     ECFB8504 58DBEF0A 8AEA7157 5D060C7D B3970F85 A6E1E4C7
    //     ABF5AE8C DB0933D7 1E8C94E0 4A25619D CEE3D226 1AD2EE6B
    //     F12FFA06 D98A0864 D8760273 3EC86A64 521F2B18 177B200C
    //     BBE11757 7A615D6C 770988C0 BAD946E2 08E24FA0 74E5AB31
    //     43DB5BFC E0FD108E 4B82D120 A93AD2CA FFFFFFFF FFFFFFFF
    //     "#;
    // );
    const_hex!(
        pub const P_GROUP16_VALUE = r#"
        FFFFFFFF FFFFFFFF C90FDAA2 2168C234 C4C6628B 80DC1CD1
        29024E08 8A67CC74 020BBEA6 3B139B22 514A0879 8E3404DD
        EF9519B3 CD3A431B 302B0A6D F25F1437 4FE1356D 6D51C245
        E485B576 625E7EC6 F44C42E9 A637ED6B 0BFF5CB6 F406B7ED
        EE386BFB 5A899FA5 AE9F2411 7C4B1FE6 49286651 ECE45B3D
        C2007CB8 A163BF05 98DA4836 1C55D39A 69163FA8 FD24CF5F
        83655D23 DCA3AD96 1C62F356 208552BB 9ED52907 7096966D
        670C354E 4ABC9804 F1746C08 CA18217C 32905E46 2E36CE3B
        E39E772C 180E8603 9B2783A2 EC07A28F B5C55DF0 6F4C52C9
        DE2BCBF6 95581718 3995497C EA956AE5 15D22618 98FA0510
        15728E5A 8AAAC42D AD33170D 04507A33 A85521AB DF1CBA64
        ECFB8504 58DBEF0A 8AEA7157 5D060C7D B3970F85 A6E1E4C7
        ABF5AE8C DB0933D7 1E8C94E0 4A25619D CEE3D226 1AD2EE6B
        F12FFA06 D98A0864 D8760273 3EC86A64 521F2B18 177B200C
        BBE11757 7A615D6C 770988C0 BAD946E2 08E24FA0 74E5AB31
        43DB5BFC E0FD108E 4B82D120 A9210801 1A723C12 A787E6D7
        88719A10 BDBA5B26 99C32718 6AF4E23C 1A946834 B6150BDA
        2583E9CA 2AD44CE8 DBBBC2DB 04DE8EF9 2E8EFC14 1FBECAA6
        287C5947 4E6BC05D 99B2964F A090C3A2 233BA186 515BE7ED
        1F612970 CEE2D7AF B81BDD76 2170481C D0069127 D5B05AA9
        93B4EA98 8D8FDDC1 86FFB7DC 90A6C08F 4DF435C9 34063199
        FFFFFFFF FFFFFFFF
        "#;
    );

    // const_hex!(
    //     pub const P_GROUP17_VALUE = r#"
    //     FFFFFFFF FFFFFFFF C90FDAA2 2168C234 C4C6628B 80DC1CD1
    //     29024E08 8A67CC74 020BBEA6 3B139B22 514A0879 8E3404DD
    //     EF9519B3 CD3A431B 302B0A6D F25F1437 4FE1356D 6D51C245
    //     E485B576 625E7EC6 F44C42E9 A637ED6B 0BFF5CB6 F406B7ED
    //     EE386BFB 5A899FA5 AE9F2411 7C4B1FE6 49286651 ECE45B3D
    //     C2007CB8 A163BF05 98DA4836 1C55D39A 69163FA8 FD24CF5F
    //     83655D23 DCA3AD96 1C62F356 208552BB 9ED52907 7096966D
    //     670C354E 4ABC9804 F1746C08 CA18217C 32905E46 2E36CE3B
    //     E39E772C 180E8603 9B2783A2 EC07A28F B5C55DF0 6F4C52C9
    //     DE2BCBF6 95581718 3995497C EA956AE5 15D22618 98FA0510
    //     15728E5A 8AAAC42D AD33170D 04507A33 A85521AB DF1CBA64
    //     ECFB8504 58DBEF0A 8AEA7157 5D060C7D B3970F85 A6E1E4C7
    //     ABF5AE8C DB0933D7 1E8C94E0 4A25619D CEE3D226 1AD2EE6B
    //     F12FFA06 D98A0864 D8760273 3EC86A64 521F2B18 177B200C
    //     BBE11757 7A615D6C 770988C0 BAD946E2 08E24FA0 74E5AB31
    //     43DB5BFC E0FD108E 4B82D120 A9210801 1A723C12 A787E6D7
    //     88719A10 BDBA5B26 99C32718 6AF4E23C 1A946834 B6150BDA
    //     2583E9CA 2AD44CE8 DBBBC2DB 04DE8EF9 2E8EFC14 1FBECAA6
    //     287C5947 4E6BC05D 99B2964F A090C3A2 233BA186 515BE7ED
    //     1F612970 CEE2D7AF B81BDD76 2170481C D0069127 D5B05AA9
    //     93B4EA98 8D8FDDC1 86FFB7DC 90A6C08F 4DF435C9 34063199
    //     "#;
    // );

    const_hex!(
        pub const P_GROUP18_VALUE = r#"
        FFFFFFFF FFFFFFFF C90FDAA2 2168C234 C4C6628B 80DC1CD1
        29024E08 8A67CC74 020BBEA6 3B139B22 514A0879 8E3404DD
        EF9519B3 CD3A431B 302B0A6D F25F1437 4FE1356D 6D51C245
        E485B576 625E7EC6 F44C42E9 A637ED6B 0BFF5CB6 F406B7ED
        EE386BFB 5A899FA5 AE9F2411 7C4B1FE6 49286651 ECE45B3D
        C2007CB8 A163BF05 98DA4836 1C55D39A 69163FA8 FD24CF5F
        83655D23 DCA3AD96 1C62F356 208552BB 9ED52907 7096966D
        670C354E 4ABC9804 F1746C08 CA18217C 32905E46 2E36CE3B
        E39E772C 180E8603 9B2783A2 EC07A28F B5C55DF0 6F4C52C9
        DE2BCBF6 95581718 3995497C EA956AE5 15D22618 98FA0510
        15728E5A 8AAAC42D AD33170D 04507A33 A85521AB DF1CBA64
        ECFB8504 58DBEF0A 8AEA7157 5D060C7D B3970F85 A6E1E4C7
        ABF5AE8C DB0933D7 1E8C94E0 4A25619D CEE3D226 1AD2EE6B
        F12FFA06 D98A0864 D8760273 3EC86A64 521F2B18 177B200C
        BBE11757 7A615D6C 770988C0 BAD946E2 08E24FA0 74E5AB31
        43DB5BFC E0FD108E 4B82D120 A9210801 1A723C12 A787E6D7
        88719A10 BDBA5B26 99C32718 6AF4E23C 1A946834 B6150BDA
        2583E9CA 2AD44CE8 DBBBC2DB 04DE8EF9 2E8EFC14 1FBECAA6
        287C5947 4E6BC05D 99B2964F A090C3A2 233BA186 515BE7ED
        1F612970 CEE2D7AF B81BDD76 2170481C D0069127 D5B05AA9
        93B4EA98 8D8FDDC1 86FFB7DC 90A6C08F 4DF435C9 34028492
        36C3FAB4 D27C7026 C1D4DCB2 602646DE C9751E76 3DBA37BD
        F8FF9406 AD9E530E E5DB382F 413001AE B06A53ED 9027D831
        179727B0 865A8918 DA3EDBEB CF9B14ED 44CE6CBA CED4BB1B
        DB7F1447 E6CC254B 33205151 2BD7AF42 6FB8F401 378CD2BF
        5983CA01 C64B92EC F032EA15 D1721D03 F482D7CE 6E74FEF6
        D55E702F 46980C82 B5A84031 900B1C9E 59E7C97F BEC7E8F3
        23A97A7E 36CC88BE 0F1D45B7 FF585AC5 4BD407B2 2B4154AA
        CC8F6D7E BF48E1D8 14CC5ED2 0F8037E0 A79715EE F29BE328
        06A1D58B B7C5DA76 F550AA3D 8A1FBFF0 EB19CCB1 A313D55C
        DA56C9EC 2EF29632 387FE8D7 6E3C0468 043E8F66 3F4860EE
        12BF2D5B 0B7474D6 E694F91E 6DBE1159 74A3926F 12FEE5E4
        38777CB6 A932DF8C D8BEC4D0 73B931BA 3BC832B6 8D9DD300
        741FA7BF 8AFC47ED 2576F693 6BA42466 3AAB639C 5AE4F568
        3423B474 2BF1C978 238F16CB E39D652D E3FDB8BE FC848AD9
        22222E04 A4037C07 13EB57A8 1A23F0C7 3473FC64 6CEA306B
        4BCBC886 2F8385DD FA9D4B7F A2C087E8 79683303 ED5BDD3A
        062B3CF5 B3A278A6 6D2A13F8 3F44F82D DF310EE0 74AB6A36
        4597E899 A0255DC1 64F31CC5 0846851D F9AB4819 5DED7EA1
        B1D510BD 7EE74D73 FAF36BC3 1ECFA268 359046F4 EB879F92
        4009438B 481C6CD7 889A002E D5EE382B C9190DA6 FC026E47
        9558E447 5677E9AA 9E3050E2 765694DF C81F56E8 80B96E71
        60C980DD 98EDD3DF FFFFFFFF FFFFFFFF
        "#;
    );

    pub const G_VALUE: u32 = 2;
}
