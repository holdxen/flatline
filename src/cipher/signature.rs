use indexmap::IndexMap;
use openssl::{
    bn::{BigNum, BigNumContext},
    dsa::{self, DsaSig},
    ec::{EcKey, EcPoint},
    ecdsa::EcdsaSig,
    hash::MessageDigest,
    md::{Md, MdRef},
    md_ctx::MdCtx,
    nid::Nid,
    pkey::{Id, PKey, Private, Public},
    pkey_ctx::PkeyCtx,
    rsa::{self, Padding, RsaPrivateKeyBuilder},
    sign::{Signer, Verifier},
};
use snafu::{OptionExt, ResultExt};

use crate::{
    error::{Error, Result, builder},
    ssh::{
        MultiplePrecisionInteger,
        buffer::{Consumer, Producer, make_buffer_without_header, match_type, put_type},
    },
};

use super::*;

algo_list!(
    signature_all,
    new_signature_all,
    new_signature_by_name,
    dyn Signature + Send,
    "ssh-ed25519" => Ed25519::new(),
    "rsa-sha2-256" => Rsa::rsa_sha2_256(),
    "rsa-sha2-512" => Rsa::rsa_sha2_512(),
    "ssh-rsa" => Rsa::ssh_rsa(),
    "ssh-dss" => Dsa::ssh_dss(),
    "ecdsa-sha2-nistp521" => Ecdsa::ecdsa_sha2_nistp521(),
    "ecdsa-sha2-nistp256" => Ecdsa::ecdsa_sha2_nistp256(),
    "ecdsa-sha2-nistp384" => Ecdsa::ecdsa_sha2_nistp384(),
);

algo_list!(
    verify_all,
    new_verify_all,
    new_verify_by_name,
    dyn Verify + Send,
    "ssh-ed25519" => Ed25519::new(),
    "rsa-sha2-256" => Rsa::rsa_sha2_256(),
    "rsa-sha2-512" => Rsa::rsa_sha2_512(),
    "ssh-rsa" => Rsa::ssh_rsa(),
    "ssh-dss" => Dsa::ssh_dss(),
    "ecdsa-sha2-nistp521" => Ecdsa::ecdsa_sha2_nistp521(),
    "ecdsa-sha2-nistp256" => Ecdsa::ecdsa_sha2_nistp256(),
    "ecdsa-sha2-nistp384" => Ecdsa::ecdsa_sha2_nistp384(),
);

pub trait Signature {
    fn name(&self) -> &str;
    fn initialize(&mut self, key: &[u8]) -> Result<()>;
    fn signature(&mut self, data: &[u8]) -> Result<Vec<u8>>;
}

struct Ed25519 {
    ctx: Option<MdCtx>,
}

impl Ed25519 {
    fn new() -> Self {
        Self { ctx: None }
    }

    fn get_ctx_mut(&mut self) -> Result<&mut MdCtx> {
        self.ctx.as_mut().context(builder::InvalidOperation {
            detail: "Uninitialize",
        })
    }
}

impl Signature for Ed25519 {
    fn name(&self) -> &str {
        "ssh-ed25519"
    }

    fn initialize(&mut self, key: &[u8]) -> Result<()> {
        let mut key = Consumer::new(key);

        // let invalid_format_key = || Error::invalid_format("invalid key format");

        if key.consume_one()? != b"ssh-ed25519" {
            return Err(super::MismatchKeySnafu.build().into());
        }

        let key = key.consume_one()?;

        let pkey = PKey::private_key_from_raw_bytes(key, Id::ED25519).context(builder::OpenSSL)?;

        let mut ctx = MdCtx::new().context(builder::OpenSSL)?;
        ctx.digest_sign_init(None, &pkey)
            .context(builder::OpenSSL)?;
        self.ctx = Some(ctx);

        Ok(())
    }

    fn signature(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let ctx = self.get_ctx_mut()?;

        let len = ctx.digest_sign(data, None).context(builder::OpenSSL)?;

        let mut buffer = vec![0; len];

        ctx.digest_sign(data, Some(&mut buffer))
            .context(builder::OpenSSL)?;
        Ok(buffer)
    }
}

pub trait Verify {
    fn name(&self) -> &str;
    fn initialize(&mut self, key: &[u8]) -> Result<()>;
    fn verify(&mut self, signature: &[u8], data: &[u8]) -> Result<bool>;
}

impl Verify for Ed25519 {
    fn name(&self) -> &str {
        "ssh-ed25519"
    }

    fn initialize(&mut self, key: &[u8]) -> Result<()> {
        let mut key = Consumer::new(key);
        let key_type = key.consume_one()?;

        if key_type != b"ssh-ed25519" {
            return Err(super::MismatchKeySnafu.build().into());
        }

        let key = key.consume_one()?;

        let key = PKey::public_key_from_raw_bytes(key, Id::ED25519).context(builder::OpenSSL)?;

        let mut ctx = MdCtx::new().context(builder::OpenSSL)?;

        ctx.digest_verify_init(None, &key)
            .context(builder::OpenSSL)?;

        self.ctx = Some(ctx);

        Ok(())
    }

    fn verify(&mut self, signature: &[u8], data: &[u8]) -> Result<bool> {
        let mut signature = Consumer::new(signature);
        let key_type = signature.consume_one()?;
        if key_type != b"ssh-ed25519" {
            return Ok(false);
        }
        let signature = signature.consume_one()?;

        let res = self
            .get_ctx_mut()?
            .digest_verify(data, signature)
            .unwrap_or(false);
        Ok(res)
    }
}

struct Rsa<T> {
    name: String,
    hash: &'static MdRef,
    ctx: Option<PkeyCtx<T>>,
}

impl<T> Rsa<T> {
    fn new(name: String, hash: &'static MdRef) -> Self {
        Self {
            name,
            hash,
            ctx: None,
        }
    }
    fn rsa_sha2_256() -> Self {
        Self::new("rsa-sha2-256".to_string(), Md::sha256())
    }

    fn rsa_sha2_512() -> Self {
        Self::new("rsa-sha2-512".to_string(), Md::sha512())
    }

    fn ssh_rsa() -> Self {
        Self::new("ssh-rsa".to_string(), Md::sha1())
    }

    fn calculate_hash(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut out = vec![0; self.hash.size()];

        let mut ctx = MdCtx::new().context(builder::OpenSSL)?;

        ctx.digest_init(self.hash).context(builder::OpenSSL)?;

        ctx.digest_update(data).context(builder::OpenSSL)?;

        ctx.digest_final(&mut out).context(builder::OpenSSL)?;

        Ok(out)
    }
}

impl Verify for Rsa<Public> {
    fn name(&self) -> &str {
        &self.name
    }

    fn initialize(&mut self, key: &[u8]) -> Result<()> {
        let mut buffer = Consumer::new(key);

        let key_type = buffer.consume_one()?;
        if key_type != b"ssh-rsa" {
            return Err(super::MismatchKeySnafu.build().into());
        }

        let e = buffer.consume_one()?;

        let n = buffer.consume_one()?;

        // self.e = Some(e);
        // self.n = Some(n);

        let n = BigNum::from_slice(n).context(builder::OpenSSL)?;
        let e = BigNum::from_slice(e).context(builder::OpenSSL)?;
        let key = rsa::Rsa::from_public_components(n, e).context(builder::OpenSSL)?;

        let pkey = PKey::from_rsa(key).context(builder::OpenSSL)?;

        let mut ctx = PkeyCtx::new(&pkey).context(builder::OpenSSL)?;

        ctx.verify_init().context(builder::OpenSSL)?;
        ctx.set_rsa_padding(Padding::PKCS1)
            .context(builder::OpenSSL)?;
        ctx.set_signature_md(self.hash).context(builder::OpenSSL)?;

        self.ctx = Some(ctx);

        Ok(())
    }

    fn verify(&mut self, signature: &[u8], data: &[u8]) -> Result<bool> {
        let hash = self.calculate_hash(data)?;
        match self.ctx {
            Some(ref mut ctx) => {
                let mut signature = Consumer::new(signature);

                let signature_type = signature.consume_one()?;
                if signature_type != self.name.as_bytes() {
                    return Err(MismatchKeySnafu.build().into());
                }

                let signature = signature.consume_one()?;

                Ok(ctx.verify(&hash, signature).unwrap_or(false))
            }

            _ => Err(builder::InvalidOperation {
                detail: "it must be initialized before verify",
            }
            .build()),
        }
    }
}

impl Signature for Rsa<Private> {
    fn name(&self) -> &str {
        &self.name
    }

    fn initialize(&mut self, key: &[u8]) -> Result<()> {
        let mut buffer = Consumer::new(key);

        if buffer.consume_one()? != b"ssh-rsa" {
            return Err(MismatchKeySnafu.build().into());
        }

        // let func = || {
        //     let one = || Some(buffer.take_one()?.1);

        //     Some((n, e, d, iqmp, p, q))
        // };

        // let (n, e, d, iqmp, p, q) = func().ok_or(Error::invalid_format("invalid key format"))?;

        let n = buffer.consume_one()?;
        let e = buffer.consume_one()?;
        let d = buffer.consume_one()?;
        let iqmp = buffer.consume_one()?;
        let p = buffer.consume_one()?;
        let q = buffer.consume_one()?;

        let n = BigNum::from_slice(n).context(builder::OpenSSL)?;
        let e = BigNum::from_slice(e).context(builder::OpenSSL)?;
        let d = BigNum::from_slice(d).context(builder::OpenSSL)?;
        let iqmp = BigNum::from_slice(iqmp).context(builder::OpenSSL)?;
        let p = BigNum::from_slice(p).context(builder::OpenSSL)?;
        let q = BigNum::from_slice(q).context(builder::OpenSSL)?;
        let mut dmp1 = BigNum::new().context(builder::OpenSSL)?;
        let mut dmq1 = BigNum::new().context(builder::OpenSSL)?;

        {
            let one = BigNum::from_u32(1).context(builder::OpenSSL)?;
            let sub = |number: &BigNum| {
                let mut r = BigNum::new().context(builder::OpenSSL)?;
                r.checked_sub(number, &one).context(builder::OpenSSL)?;
                crate::error::ok(r)
            };
            let p = sub(&p)?;

            let q = sub(&q)?;

            let mut ctx = BigNumContext::new().context(builder::OpenSSL)?;

            dmp1.checked_rem(&d, &p, &mut ctx).context(builder::OpenSSL)?;
            dmq1.checked_rem(&d, &q, &mut ctx).context(builder::OpenSSL)?;
        }

        let key = RsaPrivateKeyBuilder::new(n, e, d)
            .context(builder::OpenSSL)?
            .set_crt_params(dmp1, dmq1, iqmp)
            .context(builder::OpenSSL)?
            .set_factors(p, q)
            .context(builder::OpenSSL)?
            .build();
        // let key = rsa::Rsa::from_private_components(n, e, d, p, q, dmp1, dmq1, iqmp)?;

        let pkey = PKey::from_rsa(key).context(builder::OpenSSL)?;

        let mut ctx = PkeyCtx::new(&pkey).context(builder::OpenSSL)?;
        ctx.sign_init().context(builder::OpenSSL)?;
        ctx.set_rsa_padding(Padding::PKCS1)
            .context(builder::OpenSSL)?;
        ctx.set_signature_md(self.hash).context(builder::OpenSSL)?;

        self.ctx = Some(ctx);

        Ok(())
    }

    fn signature(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let hash = self.calculate_hash(data)?;

        let ctx = self.ctx.as_mut().context(builder::InvalidOperation {
            detail: "Uninitialized",
        })?;

        let len = ctx.sign(&hash, None).context(builder::OpenSSL)?;

        let mut vec = vec![0; len];

        ctx.sign(&hash, Some(&mut vec)).context(builder::OpenSSL)?;

        Ok(vec)
    }
}

struct Dsa<T> {
    key: Option<PKey<T>>,
}

impl<T> Dsa<T> {
    fn new() -> Self {
        Self { key: None }
    }

    fn ssh_dss() -> Self {
        Self::new()
    }
}

impl Verify for Dsa<Public> {
    fn name(&self) -> &str {
        todo!()
    }

    fn initialize(&mut self, key: &[u8]) -> Result<()> {
        let mut key = Consumer::new(key);
        // let invalid_key_format = || Error::invalid_format("invalid key format");

        // let take_one = || Result::Ok(key.take_one().ok_or(invalid_key_format!())?.1);

        if key.consume_one()? != b"ssh-dss" {
            return Err(MismatchKeySnafu.build().into());
        }

        let p = key.consume_one()?;
        let q = key.consume_one()?;
        let g = key.consume_one()?;
        let y = key.consume_one()?;

        let p = BigNum::from_slice(p).context(builder::OpenSSL)?;
        let q = BigNum::from_slice(q).context(builder::OpenSSL)?;
        let g = BigNum::from_slice(g).context(builder::OpenSSL)?;
        let y = BigNum::from_slice(y).context(builder::OpenSSL)?;

        let key = dsa::Dsa::from_public_components(p, q, g, y).context(builder::OpenSSL)?;

        let key = PKey::from_dsa(key).context(builder::OpenSSL)?;

        self.key = Some(key);

        Ok(())
    }

    fn verify(&mut self, signature: &[u8], data: &[u8]) -> Result<bool> {
        let mut signature = Consumer::new(signature);

        let signature_type = signature.consume_one()?;
        if signature_type != b"ssh-dss" {
            return Err(MismatchKeySnafu.build().into());
        }

        let signature = signature.consume_one()?;

        if signature.len() != 40 {
            return Err(super::SignatureVerificationFailedSnafu.build().into());
        }

        let r = BigNum::from_slice(&signature[0..20]).context(builder::OpenSSL)?;
        let s = BigNum::from_slice(&signature[20..]).context(builder::OpenSSL)?;

        let signature = DsaSig::from_private_components(r, s).context(builder::OpenSSL)?;

        // Serialize DSA signature to DER
        let signature = signature.to_der().context(builder::OpenSSL)?;

        let key = self.key.as_ref().context(builder::InvalidOperation {
            detail: "Uninitialize",
        })?;
        let mut verifier = Verifier::new(MessageDigest::sha1(), key).context(builder::OpenSSL)?;
        verifier.update(data).context(builder::OpenSSL)?;

        Ok(verifier.verify(&signature[..]).unwrap_or(false))
    }
}

impl Signature for Dsa<Private> {
    fn name(&self) -> &str {
        "ssh-dss"
    }

    fn initialize(&mut self, key: &[u8]) -> Result<()> {
        let mut key = Consumer::new(key);
        // let invalid_key_format = || Error::invalid_format("invalid key format");

        // let take_one = || Result::Ok(key.take_one().ok_or(invalid_key_format!())?.1);

        if key.consume_one()? != b"ssh-dss" {
            return Err(MismatchKeySnafu.build().into());
        }

        let p = key.consume_one()?;
        let q = key.consume_one()?;
        let g = key.consume_one()?;
        let y = key.consume_one()?;
        let x = key.consume_one()?;

        let p = BigNum::from_slice(p).context(builder::OpenSSL)?;
        let q = BigNum::from_slice(q).context(builder::OpenSSL)?;
        let g = BigNum::from_slice(g).context(builder::OpenSSL)?;
        let y = BigNum::from_slice(y).context(builder::OpenSSL)?;
        let x = BigNum::from_slice(x).context(builder::OpenSSL)?;

        let key = dsa::Dsa::from_private_components(p, q, g, x, y).context(builder::OpenSSL)?;

        self.key = Some(PKey::from_dsa(key).context(builder::OpenSSL)?);

        Ok(())
    }

    fn signature(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let key = self.key.as_ref().context(builder::InvalidOperation {
            detail: "Uninitialized",
        })?;

        let mut signer = Signer::new(MessageDigest::sha1(), key).context(builder::OpenSSL)?;

        signer.update(data).context(builder::OpenSSL)?;

        Ok(signer.sign_to_vec().context(builder::OpenSSL)?)
    }
}

// fn invalid_key_format() -> Error {
//     Error::invalid_format("invalid key format")
// }
struct Ecdsa<T> {
    name: String,
    nid: Nid,
    hash: &'static MdRef,
    key: Option<PKey<T>>,
}

impl<T> Ecdsa<T> {
    fn new(name: String, nid: Nid, hash: &'static MdRef) -> Self {
        Self {
            name,
            nid,
            hash,
            key: None,
        }
    }

    fn calculate_hash(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut ctx = MdCtx::new().context(builder::OpenSSL)?;
        ctx.digest_init(self.hash).context(builder::OpenSSL)?;

        ctx.digest_update(data).context(builder::OpenSSL)?;

        let mut out = vec![0; ctx.size()];

        ctx.digest_final(&mut out).context(builder::OpenSSL)?;

        Ok(out)
    }

    fn get_key(&self) -> Result<&PKey<T>> {
        self.key.as_ref().context(builder::InvalidOperation {
            detail: "Uninitialized",
        })
    }
}

impl<T> Ecdsa<T> {
    fn ecdsa_sha2_nistp256() -> Self {
        Self::new(
            "ecdsa-sha2-nistp256".to_string(),
            Nid::X9_62_PRIME256V1,
            Md::sha256(),
        )
    }

    fn ecdsa_sha2_nistp384() -> Self {
        Self::new(
            "ecdsa-sha2-nistp384".to_string(),
            Nid::SECP384R1,
            Md::sha384(),
        )
    }

    fn ecdsa_sha2_nistp521() -> Self {
        Self::new(
            "ecdsa-sha2-nistp521".to_string(),
            Nid::SECP521R1,
            Md::sha512(),
        )
    }
}

impl Signature for Ecdsa<Private> {
    fn name(&self) -> &str {
        &self.name
    }

    fn initialize(&mut self, key: &[u8]) -> Result<()> {
        let mut key = Consumer::new(key);

        let keytype = key.consume_one()?;
        if keytype != self.name.as_bytes() {
            return Err(super::MismatchKeySnafu.build().into());
        }

        let nid = key.consume_one()?;
        if !self.name.ends_with(
            std::str::from_utf8(nid)
                .ok()
                .context(super::MismatchKeySnafu)?,
        ) {
            return Err(super::MismatchKeySnafu.build().into());
        }

        let public_key = key.consume_one()?;
        let e = key.consume_one()?;

        let ec_key = EcKey::from_curve_name(self.nid).context(builder::OpenSSL)?;
        let group = ec_key.group();

        let mut bnctx = BigNumContext::new().context(builder::OpenSSL)?;
        let point = EcPoint::from_bytes(group, public_key, &mut bnctx).context(builder::OpenSSL)?;
        // let point = EcKey::from_public_key(group, &point)?;

        let e = BigNum::from_slice(e).context(builder::OpenSSL)?;
        let private =
            EcKey::from_private_components(group, &e, &point).context(builder::OpenSSL)?;

        let pkey = PKey::from_ec_key(private).context(builder::OpenSSL)?;

        self.key = Some(pkey);

        Ok(())
    }

    fn signature(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let hash = self.calculate_hash(data)?;
        let key = self.get_key()?;

        let ec = key.ec_key().context(builder::OpenSSL)?;

        let sign = EcdsaSig::sign(&hash, &ec).context(builder::OpenSSL)?;

        let r = sign.r().to_integer();
        let s = sign.s().to_integer();

        let out = make_buffer_without_header! {
            one: r,
            one: s,
        };

        Ok(out.into_vec())
    }
}

impl Verify for Ecdsa<Public> {
    fn name(&self) -> &str {
        &self.name
    }

    fn initialize(&mut self, key: &[u8]) -> Result<()> {
        let mut key = Consumer::new(key);

        let key_type = key.consume_one()?;
        if self.name.as_bytes() != key_type {
            return Err(super::MismatchKeySnafu.build().into());
        }

        let id = key.consume_one()?;

        if !self.name.ends_with(
            std::str::from_utf8(id)
                .ok()
                .context(MismatchKeySnafu)?,
        ) {
            return Err(MismatchKeySnafu.build().into());
        }

        let public_key = key.consume_one()?;

        let eckey = EcKey::from_curve_name(self.nid).context(builder::OpenSSL)?;
        let group = eckey.group();

        let mut bnctx = BigNumContext::new().context(builder::OpenSSL)?;
        let point = EcPoint::from_bytes(group, public_key, &mut bnctx).context(builder::OpenSSL)?;
        let point = EcKey::from_public_key(group, &point).context(builder::OpenSSL)?;

        point.check_key().context(builder::OpenSSL)?;

        let pkey = PKey::from_ec_key(point).context(builder::OpenSSL)?;

        self.key = Some(pkey);
        Ok(())
    }

    fn verify(&mut self, signature: &[u8], data: &[u8]) -> Result<bool> {
        let mut signature = Consumer::new(signature);
        if signature.consume_one()? != self.name.as_bytes() {
            return Err(MismatchKeySnafu.build().into());
        }

        let signature = signature.consume_one()?;

        let mut signature = Consumer::new(signature);
        let r = signature.consume_one()?;
        let s = signature.consume_one()?;

        let ecsig = EcdsaSig::from_private_components(
            BigNum::from_slice(r).context(builder::OpenSSL)?,
            BigNum::from_slice(s).context(builder::OpenSSL)?,
        )
        .context(builder::OpenSSL)?;

        let signature = ecsig.to_der().context(builder::OpenSSL)?;

        let hash = self.calculate_hash(data)?;

        let key = self.get_key()?;

        let mut ctx = PkeyCtx::new(key).context(builder::OpenSSL)?;

        ctx.verify_init().context(builder::OpenSSL)?;
        Ok(ctx.verify(&hash, &signature).context(builder::OpenSSL)?)
    }
}
