// pub encrypt: Box<dyn Encrypt + Send>,
// pub decrypt: Box<dyn Decrypt + Send>,
// pub decode: Box<dyn Decode + Send>,
// pub encode: Box<dyn Encode + Send>,

use openssl::{
    cipher::{Cipher, CipherRef},
    cipher_ctx::CipherCtx,
    md_ctx::MdCtx,
    pkey::{Id, PKey},
    symm::{self, Crypter},
};
use snafu::{OptionExt, ResultExt};

use super::Factory;
use crate::error::{self, Result, builder};
use indexmap::IndexMap;

algo_list!(
    encrypt_all,
    new_encrypt_all,
    new_encrypt_by_name,
    dyn Encrypt + Send,
    "chacha20-poly1305@openssh.com" => Chacha20Poly1205::new(),
    "aes256-gcm@openssh.com" => GaloisCounterMode::aes256_gcm_openssh(),
    "aes128-gcm@openssh.com" => GaloisCounterMode::aes128_gcm_openssh(),
    "aes256-ctr" => CounterModeOrCipherBlockChaining::aes256_ctr(),
    "aes128-cbc" => CounterModeOrCipherBlockChaining::aes128_cbc(),
    "aes192-cbc" => CounterModeOrCipherBlockChaining::aes192_cbc(),
    "aes256-cbc" => CounterModeOrCipherBlockChaining::aes256_cbc(),
    "aes128-ctr" => CounterModeOrCipherBlockChaining::aes128_ctr(),
    "aes192-ctr" => CounterModeOrCipherBlockChaining::aes192_ctr(),
    "rijndael-cbc@lysator.liu.se" => CounterModeOrCipherBlockChaining::aes256_cbc(),
    "3des-cbc" => CounterModeOrCipherBlockChaining::des_ede3_cbc(),
);

algo_list!(
    decrypt_all,
    new_decrypt_all,
    new_decrypt_by_name,
    dyn Decrypt + Send,
    "chacha20-poly1305@openssh.com" => Chacha20Poly1205::new(),
    "aes256-gcm@openssh.com" => GaloisCounterMode::aes256_gcm_openssh(),
    "aes128-gcm@openssh.com" => GaloisCounterMode::aes128_gcm_openssh(),
    "aes256-ctr" => CounterModeOrCipherBlockChaining::aes256_ctr(),
    "aes128-cbc" => CounterModeOrCipherBlockChaining::aes128_cbc(),
    "aes192-cbc" => CounterModeOrCipherBlockChaining::aes192_cbc(),
    "aes256-cbc" => CounterModeOrCipherBlockChaining::aes256_cbc(),
    "aes128-ctr" => CounterModeOrCipherBlockChaining::aes128_ctr(),
    "aes192-ctr" => CounterModeOrCipherBlockChaining::aes192_ctr(),
    "rijndael-cbc@lysator.liu.se" => CounterModeOrCipherBlockChaining::aes256_cbc(),
    "3des-cbc" => CounterModeOrCipherBlockChaining::des_ede3_cbc(),
);

pub trait Encrypt {
    fn name(&self) -> &str;
    fn iv_len(&self) -> usize;
    fn key_len(&self) -> usize;
    fn block_size(&self) -> usize;
    fn initialize(&mut self, iv: &[u8], key: &[u8]) -> error::Result<()>;
    fn update(&mut self, data: &[u8], buf: &mut Vec<u8>) -> error::Result<usize>;
    fn finalize(&mut self, buf: &mut Vec<u8>) -> error::Result<usize>;

    fn is_galois_counter_mode(&self) -> bool;
    fn tag_len(&self) -> usize;
    fn update_sequence_number(&mut self, number: u32) -> error::Result<()>;
    fn additional_authenticated_data(&mut self, data: &mut [u8]) -> error::Result<()>;
    fn authentication_tag(&mut self, tag: &mut [u8]) -> error::Result<()>;
}
pub trait Decrypt {
    fn name(&self) -> &str;
    fn iv_len(&self) -> usize;
    fn key_len(&self) -> usize;
    fn block_size(&self) -> usize;
    fn initialize(&mut self, iv: &[u8], key: &[u8]) -> error::Result<()>;
    fn update(&mut self, data: &[u8], out: &mut Vec<u8>) -> error::Result<usize>;
    fn finalize(&mut self, buf: &mut Vec<u8>) -> error::Result<usize>;

    fn is_galois_counter_mode(&self) -> bool;
    fn tag_len(&self) -> usize;
    fn update_sequence_number(&mut self, number: u32) -> error::Result<()>;
    fn additional_authenticated_data(&mut self, data: &mut [u8]) -> error::Result<()>;
    fn authentication_tag(&mut self, data: &[u8]) -> error::Result<()>;
}

#[derive(Default)]
struct Chacha20Poly1205 {
    main_ctx: Option<CipherCtx>,
    header_ctx: Option<CipherCtx>,
    mac_ctx: Option<MdCtx>,
    mac: Option<Vec<u8>>,
}

impl Encrypt for Chacha20Poly1205 {
    fn name(&self) -> &str {
        "chacha20-poly1305@openssh.com"
    }

    fn is_galois_counter_mode(&self) -> bool {
        true
    }

    fn block_size(&self) -> usize {
        8
    }

    fn iv_len(&self) -> usize {
        0
    }

    fn key_len(&self) -> usize {
        64
    }

    fn tag_len(&self) -> usize {
        16
    }

    fn initialize(&mut self, _: &[u8], key: &[u8]) -> Result<()> {
        let mut main_ctx = CipherCtx::new().context(builder::OpenSSL)?;

        main_ctx
            .encrypt_init(Some(Cipher::chacha20()), Some(&key[0..32]), None)
            .context(builder::OpenSSL)?;

        self.main_ctx = Some(main_ctx);

        let mut header_ctx = CipherCtx::new().context(builder::OpenSSL)?;

        header_ctx
            .encrypt_init(Some(Cipher::chacha20()), Some(&key[32..]), None)
            .context(builder::OpenSSL)?;

        self.header_ctx = Some(header_ctx);

        Ok(())
    }

    fn update_sequence_number(&mut self, number: u32) -> Result<()> {
        let bytes = u64::from(number).to_be_bytes();

        let mut iv = [0; 16];

        iv[8..].copy_from_slice(&bytes);

        let header_ctx = self.get_header_ctx()?;

        header_ctx
            .encrypt_init(None, None, Some(&iv))
            .context(builder::OpenSSL)?;

        let main_ctx = self.get_main_ctx()?;

        main_ctx
            .encrypt_init(None, None, Some(&iv))
            .context(builder::OpenSSL)?;

        let mut poly_key = [0; 64];

        main_ctx
            .cipher_update(&[0; 64], Some(&mut poly_key))
            .context(builder::OpenSSL)?;

        let pkey = PKey::private_key_from_raw_bytes(&poly_key[..32], Id::POLY1305)
            .context(builder::OpenSSL)?;

        let mut mac_ctx = MdCtx::new().context(builder::OpenSSL)?;

        mac_ctx
            .digest_sign_init(None, &pkey)
            .context(builder::OpenSSL)?;

        self.mac_ctx = Some(mac_ctx);

        Ok(())
    }

    fn additional_authenticated_data(&mut self, aad: &mut [u8]) -> Result<()> {
        let header_ctx = self.get_header_ctx()?;

        let input = aad.to_vec();

        header_ctx
            .cipher_update(&input, Some(aad))
            .context(builder::OpenSSL)?;

        header_ctx.cipher_final(aad).context(builder::OpenSSL)?;

        self.get_mac_ctx()?
            .digest_sign_update(aad)
            .context(builder::OpenSSL)?;

        Ok(())
    }

    fn update(&mut self, data: &[u8], out: &mut Vec<u8>) -> Result<usize> {
        let pos = out.len();
        let len = self
            .get_main_ctx()?
            .cipher_update_vec(data, out)
            .context(builder::OpenSSL)?;

        self.get_mac_ctx()?
            .digest_sign_update(&out[pos..pos + len])
            .context(builder::OpenSSL)?;

        Ok(len)
    }

    fn finalize(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        self.get_main_ctx()?
            .cipher_final_vec(buf)
            .context(builder::OpenSSL)
    }

    fn authentication_tag(&mut self, tag: &mut [u8]) -> error::Result<()> {
        self.get_mac_ctx()?
            .digest_sign_final(Some(tag))
            .context(builder::OpenSSL)?;
        Ok(())
    }
}

impl Decrypt for Chacha20Poly1205 {
    fn name(&self) -> &str {
        "chacha20-poly1305@openssh.com"
    }

    fn is_galois_counter_mode(&self) -> bool {
        true
    }

    fn block_size(&self) -> usize {
        8
    }

    fn iv_len(&self) -> usize {
        0
    }

    fn key_len(&self) -> usize {
        64
    }

    fn tag_len(&self) -> usize {
        16
    }

    fn initialize(&mut self, _: &[u8], key: &[u8]) -> Result<()> {
        let mut main_ctx = CipherCtx::new().context(builder::OpenSSL)?;

        main_ctx
            .decrypt_init(Some(Cipher::chacha20()), Some(&key[0..32]), None)
            .context(builder::OpenSSL)?;

        self.main_ctx = Some(main_ctx);

        let mut header_ctx = CipherCtx::new().context(builder::OpenSSL)?;

        header_ctx
            .decrypt_init(Some(Cipher::chacha20()), Some(&key[32..]), None)
            .context(builder::OpenSSL)?;

        self.header_ctx = Some(header_ctx);

        Ok(())
    }

    fn update_sequence_number(&mut self, number: u32) -> Result<()> {
        let bytes = u64::from(number).to_be_bytes();

        let mut iv = [0; 16];

        iv[8..].copy_from_slice(&bytes);

        let header_ctx = self.get_header_ctx()?;

        header_ctx
            .decrypt_init(None, None, Some(&iv))
            .context(builder::OpenSSL)?;

        let main_ctx = self.get_main_ctx()?;

        main_ctx
            .decrypt_init(None, None, Some(&iv))
            .context(builder::OpenSSL)?;

        let mut poly_key = [0; 64];

        main_ctx
            .cipher_update(&[0; 64], Some(&mut poly_key))
            .context(builder::OpenSSL)?;

        let pkey = PKey::private_key_from_raw_bytes(&poly_key[..32], Id::POLY1305)
            .context(builder::OpenSSL)?;

        let mut mac_ctx = MdCtx::new().context(builder::OpenSSL)?;

        mac_ctx
            .digest_sign_init(None, &pkey)
            .context(builder::OpenSSL)?;

        self.mac_ctx = Some(mac_ctx);

        Ok(())
    }

    fn additional_authenticated_data(&mut self, aad: &mut [u8]) -> Result<()> {
        let input = aad.to_vec();
        self.get_mac_ctx()?
            .digest_sign_update(&input)
            .context(builder::OpenSSL)?;

        let header_ctx = self.get_header_ctx()?;

        header_ctx
            .cipher_update(&input, Some(aad))
            .context(builder::OpenSSL)?;

        header_ctx.cipher_final(aad).context(builder::OpenSSL)?;

        Ok(())
    }

    fn update(&mut self, data: &[u8], out: &mut Vec<u8>) -> Result<usize> {
        self.get_mac_ctx()?
            .digest_sign_update(data)
            .context(builder::OpenSSL)?;
        let len = self
            .get_main_ctx()?
            .cipher_update_vec(data, out)
            .context(builder::OpenSSL)?;
        Ok(len)
    }

    fn finalize(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let len = self
            .get_main_ctx()?
            .cipher_final_vec(buf)
            .context(builder::OpenSSL)?;

        let mut tag = vec![];
        self.get_mac_ctx()?
            .digest_sign_final_to_vec(&mut tag)
            .context(builder::OpenSSL)?;

        // if self.mac != Some(tag) {
        //     return Err(Error::MacVerificationFailed);
        // }
        // println!("mac={:?}, tag={:?}", self.mac, tag);
        snafu::ensure!(self.mac == Some(tag), super::MacVerificationFailedSnafu);

        self.mac = None;

        Ok(len)
    }

    fn authentication_tag(&mut self, data: &[u8]) -> Result<()> {
        self.mac = Some(data.to_vec());
        Ok(())
    }
}

impl Chacha20Poly1205 {
    fn new() -> Self {
        Self::default()
    }
    fn get_main_ctx(&mut self) -> Result<&mut CipherCtx> {
        self.main_ctx.as_mut().context(builder::InvalidOperation {
            detail: "Uninitialized",
        })
    }

    fn get_header_ctx(&mut self) -> Result<&mut CipherCtx> {
        self.header_ctx.as_mut().context(builder::InvalidOperation {
            detail: "Uninitialized",
        })
    }

    fn get_mac_ctx(&mut self) -> Result<&mut MdCtx> {
        self.mac_ctx.as_mut().context(builder::InvalidOperation {
            detail: "Uninitialized",
        })
    }
}

struct GaloisCounterMode {
    name: &'static str,
    cipher: symm::Cipher,
    block_size: usize,
    key_len: usize,
    iv_len: usize,

    tag_len: usize,
    iv: Option<Vec<u8>>,
    key: Option<Vec<u8>>,
    ctx: Option<Crypter>,
}

impl GaloisCounterMode {
    fn new(
        name: &'static str,
        cipher: symm::Cipher,
        block_size: usize,
        key_len: usize,
        iv_len: usize,
        tag_len: usize,
    ) -> Self {
        Self {
            name,
            cipher,
            block_size,
            key_len,
            iv_len,
            tag_len,
            iv: None,
            key: None,
            ctx: None,
        }
    }
    fn aes128_gcm_openssh() -> Self {
        Self::new(
            "aes128-gcm@openssh.com",
            symm::Cipher::aes_128_gcm(),
            16,
            16,
            12,
            16,
        )
    }
    fn aes256_gcm_openssh() -> Self {
        Self::new(
            "aes256-gcm@openssh.com",
            symm::Cipher::aes_256_gcm(),
            16,
            32,
            12,
            16,
        )
    }
    fn get_ctx(&mut self) -> Result<&mut Crypter> {
        self.ctx.as_mut().context(builder::InvalidOperation {
            detail: "Uninitiailzed",
        })
    }

    fn reset(&mut self, mode: symm::Mode) -> Result<()> {
        match (&self.key, &mut self.iv) {
            /*
                   With AES-GCM, the 12-octet IV is broken into two fields: a 4-octet
                   fixed field and an 8-octet invocation counter field.  The invocation
                   field is treated as a 64-bit integer and is incremented after each
                   invocation of AES-GCM to process a binary packet.
            */
            (Some(key), Some(iv)) => {
                assert_eq!(iv.len(), 12);
                // let u64 = BigEndian::read_u64(&iv[4..]).wrapping_add(1);
                for i in (4..12).rev() {
                    iv[i] = iv[i].wrapping_add(1);
                    if iv[i] != 0 {
                        break;
                    }
                }
                let ctx =
                    Crypter::new(self.cipher, mode, key, Some(iv)).context(builder::OpenSSL)?;
                self.ctx = Some(ctx);
                Ok(())
            }
            _ => Err(builder::InvalidOperation {
                detail: "Uninitialized",
            }
            .build()),
        }
    }
}

impl Encrypt for GaloisCounterMode {
    fn name(&self) -> &str {
        self.name
    }

    fn is_galois_counter_mode(&self) -> bool {
        true
    }

    fn block_size(&self) -> usize {
        self.block_size
    }

    fn iv_len(&self) -> usize {
        self.iv_len
    }

    fn key_len(&self) -> usize {
        self.key_len
    }

    fn initialize(&mut self, iv: &[u8], key: &[u8]) -> Result<()> {
        let mut ctx = Crypter::new(self.cipher, symm::Mode::Encrypt, key, Some(iv))
            .context(builder::OpenSSL)?;
        ctx.pad(false);
        self.ctx = Some(ctx);
        self.iv = Some(iv.to_vec());
        self.key = Some(key.to_vec());
        Ok(())
    }

    fn update(&mut self, data: &[u8], buf: &mut Vec<u8>) -> Result<usize> {
        let base = buf.len();
        buf.resize(base + data.len() + self.block_size, 0);
        let len = self
            .get_ctx()?
            .update(data, &mut buf[base..])
            .context(builder::OpenSSL)?;
        buf.truncate(base + len);
        Ok(len)
    }

    fn finalize(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let base = buf.len();
        buf.resize(base + self.block_size, 0);
        let len = self
            .get_ctx()?
            .finalize(&mut buf[base..])
            .context(builder::OpenSSL)?;
        buf.truncate(base + len);
        Ok(len)
    }

    fn authentication_tag(&mut self, tag: &mut [u8]) -> Result<()> {
        self.get_ctx()?.get_tag(tag).context(builder::OpenSSL)?;
        self.reset(symm::Mode::Encrypt)?;
        Ok(())
    }

    fn tag_len(&self) -> usize {
        self.tag_len
    }

    fn update_sequence_number(&mut self, _: u32) -> Result<()> {
        Ok(())
    }

    fn additional_authenticated_data(&mut self, aad: &mut [u8]) -> Result<()> {
        self.get_ctx()?.aad_update(aad).context(builder::OpenSSL)
    }
}

impl Decrypt for GaloisCounterMode {
    fn name(&self) -> &str {
        self.name
    }

    fn is_galois_counter_mode(&self) -> bool {
        true
    }

    fn block_size(&self) -> usize {
        self.block_size
    }

    fn iv_len(&self) -> usize {
        self.iv_len
    }

    fn key_len(&self) -> usize {
        self.key_len
    }

    fn initialize(&mut self, iv: &[u8], key: &[u8]) -> Result<()> {
        let mut ctx = Crypter::new(self.cipher, symm::Mode::Decrypt, key, Some(iv))
            .context(builder::OpenSSL)?;
        ctx.pad(false);
        self.ctx = Some(ctx);
        self.key = Some(key.to_vec());
        self.iv = Some(iv.to_vec());
        Ok(())
    }

    fn update(&mut self, data: &[u8], buf: &mut Vec<u8>) -> Result<usize> {
        let base = buf.len();
        buf.resize(base + data.len() + self.block_size, 0);
        let len = self
            .get_ctx()?
            .update(data, &mut buf[base..])
            .context(builder::OpenSSL)?;
        buf.truncate(base + len);
        Ok(len)
    }

    fn finalize(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let base = buf.len();
        buf.resize(base + self.block_size, 0);
        let len = self
            .get_ctx()?
            .finalize(&mut buf[base..])
            .context(builder::OpenSSL)?;
        buf.truncate(base + len);
        self.reset(symm::Mode::Decrypt)?;
        Ok(len)
    }

    fn authentication_tag(&mut self, data: &[u8]) -> Result<()> {
        self.get_ctx()?.set_tag(data).context(builder::OpenSSL)?;
        Ok(())
    }

    fn tag_len(&self) -> usize {
        self.tag_len
    }

    fn update_sequence_number(&mut self, _: u32) -> Result<()> {
        Ok(())
    }

    fn additional_authenticated_data(&mut self, data: &mut [u8]) -> error::Result<()> {
        self.get_ctx()?.aad_update(data).context(builder::OpenSSL)?;
        Ok(())
    }
}

struct CounterModeOrCipherBlockChaining {
    name: &'static str,
    ctx: Option<CipherCtx>,
    cipher: &'static CipherRef,
    block_size: usize,
    key_len: usize,
    iv_len: usize,
}

#[easy_ext::ext]
impl &CipherRef {
    fn ensure(self, block_size: usize, key_len: usize, iv_len: usize) -> Self {
        assert_eq!(self.block_size(), block_size);
        assert_eq!(self.key_length(), key_len);
        assert_eq!(self.iv_length(), iv_len);

        self
    }
}

impl CounterModeOrCipherBlockChaining {
    fn get_ctx_mut(&mut self) -> Result<&mut CipherCtx> {
        self.ctx.as_mut().context(builder::InvalidOperation {
            detail: "Uninitialized context",
        })
    }

    fn aes256_ctr() -> Self {
        Self {
            name: "aes256-ctr",
            ctx: None,
            cipher: Cipher::aes_256_ctr(),
            block_size: 16,
            key_len: 32,
            iv_len: 16,
        }
    }

    fn aes128_cbc() -> Self {
        Self {
            name: "aes128-cbc",
            ctx: None,
            cipher: Cipher::aes_128_cbc(),
            block_size: 16,
            key_len: 16,
            iv_len: 16,
        }
    }

    fn aes192_cbc() -> Self {
        Self {
            name: "aes192-cbc",
            ctx: None,
            cipher: Cipher::aes_192_cbc(),
            block_size: 16,
            key_len: 24,
            iv_len: 16,
        }
    }

    fn aes256_cbc() -> Self {
        Self {
            name: "aes256-cbc",
            ctx: None,
            cipher: Cipher::aes_256_cbc(),
            block_size: 16,
            key_len: 32,
            iv_len: 16,
        }
    }

    fn aes128_ctr() -> Self {
        Self {
            name: "aes128-ctr",
            ctx: None,
            cipher: Cipher::aes_128_ctr(),
            block_size: 16,
            key_len: 16,
            iv_len: 16,
        }
    }

    fn aes192_ctr() -> Self {
        Self {
            name: "aes192-ctr",
            ctx: None,
            cipher: Cipher::aes_192_ctr(),
            block_size: 16,
            key_len: 24,
            iv_len: 16,
        }
    }

    fn des_ede3_cbc() -> Self {
        Self {
            name: "3des-cbc",
            ctx: None,
            cipher: Cipher::des_ede3_cbc(),
            block_size: 8,
            key_len: 24,
            iv_len: 8,
        }
    }
}

impl Decrypt for CounterModeOrCipherBlockChaining {
    fn is_galois_counter_mode(&self) -> bool {
        false
    }

    fn block_size(&self) -> usize {
        self.block_size
    }

    fn iv_len(&self) -> usize {
        self.iv_len
    }

    fn key_len(&self) -> usize {
        self.key_len
    }

    fn initialize(&mut self, iv: &[u8], key: &[u8]) -> Result<()> {
        let mut cipher = CipherCtx::new().context(builder::OpenSSL)?;
        cipher
            .decrypt_init(Some(self.cipher), Some(key), Some(iv))
            .context(builder::OpenSSL)?;
        cipher.set_padding(false);
        self.ctx = Some(cipher);
        Ok(())
    }

    fn update(&mut self, data: &[u8], buf: &mut Vec<u8>) -> Result<usize> {
        let len = self
            .get_ctx_mut()?
            .cipher_update_vec(data, buf)
            .context(builder::OpenSSL)?;
        Ok(len)
    }

    fn finalize(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let size = self
            .get_ctx_mut()?
            .cipher_final_vec(buf)
            .context(builder::OpenSSL)?;

        Ok(size)
    }

    fn authentication_tag(&mut self, tag: &[u8]) -> Result<()> {
        self.get_ctx_mut()?.set_tag(tag).context(builder::OpenSSL)
    }

    fn name(&self) -> &str {
        self.name
    }

    fn tag_len(&self) -> usize {
        0
    }

    fn update_sequence_number(&mut self, _: u32) -> Result<()> {
        Ok(())
    }

    fn additional_authenticated_data(&mut self, _: &mut [u8]) -> error::Result<()> {
        Ok(())
    }
}

impl Encrypt for CounterModeOrCipherBlockChaining {
    fn is_galois_counter_mode(&self) -> bool {
        false
    }

    fn block_size(&self) -> usize {
        self.block_size
    }

    fn iv_len(&self) -> usize {
        self.iv_len
    }

    fn key_len(&self) -> usize {
        self.key_len
    }

    fn initialize(&mut self, iv: &[u8], key: &[u8]) -> Result<()> {
        let mut cipher = CipherCtx::new().context(builder::OpenSSL)?;
        cipher
            .encrypt_init(Some(self.cipher), Some(key), Some(iv))
            .context(builder::OpenSSL)?;
        cipher.set_padding(false);
        self.ctx = Some(cipher);
        // self.iv = Some(iv.to_vec());
        // self.key = Some(key.to_vec());
        Ok(())
    }

    fn update(&mut self, data: &[u8], buf: &mut Vec<u8>) -> Result<usize> {
        self.get_ctx_mut()?
            .cipher_update_vec(data, buf)
            .context(builder::OpenSSL)
    }

    fn finalize(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let size = self
            .get_ctx_mut()?
            .cipher_final_vec(buf)
            .context(builder::OpenSSL)?;

        Ok(size)
    }

    fn authentication_tag(&mut self, _: &mut [u8]) -> Result<()> {
        Ok(())
    }

    fn name(&self) -> &str {
        self.name
    }

    fn tag_len(&self) -> usize {
        0
    }

    fn update_sequence_number(&mut self, _: u32) -> Result<()> {
        Ok(())
    }

    fn additional_authenticated_data(&mut self, _: &mut [u8]) -> error::Result<()> {
        Ok(())
    }
}
