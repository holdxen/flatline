//! Symmetric encryption of SSH binary packets (RFC 4253, section 6).
//!
//! [`Encrypt`] protects outgoing packets and [`Decrypt`] protects incoming
//! ones once the session keys are active. In addition to plain
//! encrypt-and-MAC ciphers, the traits cover the AEAD ciphers
//! `aes128-gcm@openssh.com`, `aes256-gcm@openssh.com` and
//! `chacha20-poly1305@openssh.com`, whose framing of the binary packet
//! (length header handled as associated data, trailing authentication tag)
//! follows OpenSSH's conventions.

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
    "cipher",
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
    "cipher",
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

/// Encrypts outgoing SSH binary packets once the session keys are active.
///
/// One instance serves one direction of a connection (client-to-server or
/// server-to-client). The packet layer drives it once per packet:
/// `update_sequence_number`, `additional_authenticated_data`, `update`,
/// `finalize`, `authentication_tag`.
pub trait Encrypt {
    /// Returns the SSH cipher name (`aes256-ctr`,
    /// `chacha20-poly1305@openssh.com`, ...) as it appears in
    /// `SSH_MSG_KEXINIT`.
    fn name(&self) -> &str;
    /// Returns the number of initialization-vector bytes required by
    /// `initialize`.
    ///
    /// Ciphers that take their nonce from the packet sequence number, such as
    /// `chacha20-poly1305@openssh.com`, need no IV and report `0`.
    fn iv_len(&self) -> usize;
    /// Returns the number of key bytes required by `initialize`.
    ///
    /// This is the total key material derived from the key exchange, which is
    /// 64 bytes for `chacha20-poly1305@openssh.com` (two 32-byte keys).
    fn key_len(&self) -> usize;
    /// Returns the cipher block size in bytes, which the packet layer uses to
    /// compute the padding length of every packet (RFC 4253, section 6).
    fn block_size(&self) -> usize;
    /// Initializes the cipher with the `iv` and `key` derived during key
    /// exchange; must be called before any other method.
    fn initialize(&mut self, iv: &[u8], key: &[u8]) -> error::Result<()>;
    /// Encrypts `data` and appends the ciphertext to `buf` (after any
    /// contents it already has), returning the number of bytes written to
    /// `buf`.
    fn update(&mut self, data: &[u8], buf: &mut Vec<u8>) -> error::Result<usize>;
    /// Flushes the cipher's final internal state into `buf`, appending it
    /// after any existing contents and returning the number of bytes written.
    ///
    /// Called once per packet after `update`.
    fn finalize(&mut self, buf: &mut Vec<u8>) -> error::Result<usize>;

    /// Returns whether this is an AEAD cipher (`aes*-gcm@openssh.com` or
    /// `chacha20-poly1305@openssh.com`).
    ///
    /// Despite the method's name, OpenSSH's chacha20-poly1305 variant reports
    /// `true` as well. The packet layer uses this to select OpenSSH's AEAD
    /// framing of the binary packet: the 4-byte packet length header is
    /// passed through `additional_authenticated_data` rather than encrypted
    /// along with the payload, and the packet ends with a `tag_len`-byte
    /// authentication tag.
    fn is_galois_counter_mode(&self) -> bool;
    /// Returns the length in bytes of the per-packet authentication tag
    /// appended to each packet, or `0` for ciphers that provide no
    /// authentication of their own.
    fn tag_len(&self) -> usize;
    /// Supplies the current packet sequence number before a packet is
    /// processed.
    ///
    /// `chacha20-poly1305@openssh.com` uses the sequence number as its nonce,
    /// so this re-derives the per-packet Poly1305 key for every packet;
    /// AES-GCM advances its IV invocation counter internally and the plain
    /// ciphers ignore the value.
    fn update_sequence_number(&mut self, number: u32) -> error::Result<()>;
    /// Handles the packet's 4-byte length header for AEAD ciphers.
    ///
    /// On encryption `data` holds the plaintext header; implementations may
    /// replace it in place (`chacha20-poly1305@openssh.com` encrypts it with
    /// its dedicated header key) and feed it to the AEAD as associated data
    /// together with the packet body. Called before `update`; a no-op for
    /// non-AEAD ciphers.
    fn additional_authenticated_data(&mut self, data: &mut [u8]) -> error::Result<()>;
    /// Writes the authentication tag of the packet just encrypted into `tag`,
    /// which must be `tag_len` bytes long; the packet layer appends it after
    /// the ciphertext. A no-op for non-AEAD ciphers.
    fn authentication_tag(&mut self, tag: &mut [u8]) -> error::Result<()>;
}
/// Decrypts incoming SSH binary packets once the session keys are active.
///
/// One instance serves one direction of a connection. The packet layer drives
/// it once per packet: `update_sequence_number`,
/// `additional_authenticated_data`, `update`, `authentication_tag`,
/// `finalize`.
pub trait Decrypt {
    /// Returns the SSH cipher name (`aes256-ctr`,
    /// `chacha20-poly1305@openssh.com`, ...) as it appears in
    /// `SSH_MSG_KEXINIT`.
    fn name(&self) -> &str;
    /// Returns the number of initialization-vector bytes required by
    /// `initialize`.
    ///
    /// Ciphers that take their nonce from the packet sequence number, such as
    /// `chacha20-poly1305@openssh.com`, need no IV and report `0`.
    fn iv_len(&self) -> usize;
    /// Returns the number of key bytes required by `initialize`.
    ///
    /// This is the total key material derived from the key exchange, which is
    /// 64 bytes for `chacha20-poly1305@openssh.com` (two 32-byte keys).
    fn key_len(&self) -> usize;
    /// Returns the cipher block size in bytes, which the packet layer uses to
    /// validate the padding length of every packet (RFC 4253, section 6).
    fn block_size(&self) -> usize;
    /// Initializes the cipher with the `iv` and `key` derived during key
    /// exchange; must be called before any other method.
    fn initialize(&mut self, iv: &[u8], key: &[u8]) -> error::Result<()>;
    /// Decrypts `data` and appends the plaintext to `out` (after any contents
    /// it already has), returning the number of bytes written to `out`.
    fn update(&mut self, data: &[u8], out: &mut Vec<u8>) -> error::Result<usize>;
    /// Flushes the cipher's final internal state into `buf`, appending it
    /// after any existing contents and returning the number of bytes written.
    ///
    /// For AEAD ciphers this also checks the packet's authentication tag
    /// against the one supplied by `authentication_tag` and fails if they do
    /// not match (for `chacha20-poly1305@openssh.com` with an
    /// `Error::MacVerificationFailed` error), so a successful return means
    /// the packet was authentic.
    fn finalize(&mut self, buf: &mut Vec<u8>) -> error::Result<usize>;

    /// Returns whether this is an AEAD cipher (`aes*-gcm@openssh.com` or
    /// `chacha20-poly1305@openssh.com`).
    ///
    /// Despite the method's name, OpenSSH's chacha20-poly1305 variant reports
    /// `true` as well. The packet layer uses this to select OpenSSH's AEAD
    /// framing of the binary packet: the 4-byte packet length header is
    /// read through `additional_authenticated_data` and the bytes following
    /// the payload are treated as a `tag_len`-byte authentication tag.
    fn is_galois_counter_mode(&self) -> bool;
    /// Returns the length in bytes of the authentication tag that follows the
    /// payload of each packet, or `0` for ciphers that provide no
    /// authentication of their own.
    fn tag_len(&self) -> usize;
    /// Supplies the current packet sequence number before a packet is
    /// processed.
    ///
    /// `chacha20-poly1305@openssh.com` uses the sequence number as its nonce,
    /// so this re-derives the per-packet Poly1305 key for every packet;
    /// AES-GCM advances its IV invocation counter internally and the plain
    /// ciphers ignore the value.
    fn update_sequence_number(&mut self, number: u32) -> error::Result<()>;
    /// Handles the packet's 4-byte length header for AEAD ciphers.
    ///
    /// `data` holds the header bytes read from the wire; implementations
    /// authenticate them as associated data (`chacha20-poly1305@openssh.com`
    /// also decrypts them in place) so the length is covered by the packet's
    /// tag. Called before `update`; a no-op for non-AEAD ciphers.
    fn additional_authenticated_data(&mut self, data: &mut [u8]) -> error::Result<()>;
    /// Supplies the expected authentication tag read from the packet.
    ///
    /// The tag is compared against the computed one when `finalize` runs,
    /// which fails if they differ. A no-op for non-AEAD ciphers.
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

#[cfg(test)]
mod test {
    use super::*;

    /// Builds a deterministic key/IV pair of the requested lengths.
    fn material(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| seed.wrapping_add(i as u8)).collect()
    }

    fn encryptor(name: &str) -> Box<dyn Encrypt + Send> {
        new_encrypt_by_name(name).expect("unknown cipher")()
    }

    fn decryptor(name: &str) -> Box<dyn Decrypt + Send> {
        new_decrypt_by_name(name).expect("unknown cipher")()
    }

    /// Encrypts one non-AEAD packet body and returns the wire bytes.
    fn seal(name: &str, seq: u32, plain: &[u8]) -> Vec<u8> {
        let mut enc = encryptor(name);
        enc.initialize(&material(enc.iv_len(), 1), &material(enc.key_len(), 2))
            .expect("init failed");
        enc.update_sequence_number(seq).expect("seq failed");

        let mut out = Vec::new();
        enc.update(plain, &mut out).expect("update failed");
        enc.finalize(&mut out).expect("finalize failed");
        out
    }

    /// Decrypts bytes produced by [`seal`] with the same parameters.
    fn open(name: &str, seq: u32, cipher: &[u8]) -> Vec<u8> {
        let mut dec = decryptor(name);
        dec.initialize(&material(dec.iv_len(), 1), &material(dec.key_len(), 2))
            .expect("init failed");
        dec.update_sequence_number(seq).expect("seq failed");

        let mut out = Vec::new();
        dec.update(cipher, &mut out).expect("update failed");
        dec.finalize(&mut out).expect("finalize failed");
        out
    }

    fn plain() -> Vec<u8> {
        b"attack at dawn".repeat(100)
    }

    /// A payload padded out to a whole number of 16-byte blocks.
    ///
    /// SSH performs padding in the packet layer, so these ciphers run with
    /// OpenSSL padding disabled: CBC/3DES input must be block-aligned or
    /// `finalize` fails.
    fn plain_aligned() -> Vec<u8> {
        let mut data = plain();
        while data.len() % 16 != 0 {
            data.push(0);
        }
        data
    }

    #[test]
    fn ctr_ciphers_round_trip() {
        for name in ["aes128-ctr", "aes192-ctr", "aes256-ctr"] {
            let wire = seal(name, 0, &plain());
            assert_eq!(wire.len(), plain().len(), "CTR is a stream cipher: {name}");
            assert_ne!(&wire[..], &plain()[..], "ciphertext must differ: {name}");
            assert_eq!(open(name, 0, &wire), plain(), "round trip failed: {name}");
        }
    }

    #[test]
    fn cbc_ciphers_round_trip() {
        let payload = plain_aligned();
        for name in ["aes128-cbc", "aes192-cbc", "aes256-cbc", "3des-cbc"] {
            let wire = seal(name, 0, &payload);
            assert_eq!(open(name, 0, &wire), payload, "round trip failed: {name}");
        }
    }

    /// With SSH-style padding disabled in the cipher, a partial block is a
    /// hard error instead of something OpenSSL quietly pads.
    #[test]
    fn cbc_rejects_unaligned_input() {
        let mut enc = encryptor("aes256-cbc");
        enc.initialize(&material(enc.iv_len(), 1), &material(enc.key_len(), 2))
            .expect("init failed");
        let mut out = Vec::new();
        let unaligned = &plain()[..plain().len() - 1];
        enc.update(unaligned, &mut out).expect("update failed");
        assert!(enc.finalize(&mut out).is_err());
    }

    #[test]
    fn ctr_keystream_advances_within_an_instance() {
        // The IV comes from the key exchange and is fixed for the life of
        // the cipher; the sequence number is *not* mixed in for CTR. Two
        // packets under one instance must still differ, because the counter
        // advances.
        let mut enc = encryptor("aes256-ctr");
        enc.initialize(&material(enc.iv_len(), 1), &material(enc.key_len(), 2))
            .expect("init failed");

        let mut first = Vec::new();
        enc.update(&plain(), &mut first).expect("update failed");
        enc.finalize(&mut first).expect("finalize failed");

        let mut second = Vec::new();
        enc.update(&plain(), &mut second).expect("update failed");
        enc.finalize(&mut second).expect("finalize failed");

        assert_eq!(first.len(), second.len());
        assert_ne!(first, second, "keystream must not repeat within one cipher");
    }

    /// A freshly initialized cipher repeats the keystream of another with
    /// the same key/IV — which is exactly why each direction of a connection
    /// derives its own IV during key exchange.
    #[test]
    fn same_key_and_iv_produce_the_same_keystream() {
        assert_eq!(
            seal("aes256-ctr", 0, &plain()),
            seal("aes256-ctr", 0, &plain())
        );
    }

    #[test]
    fn decrypting_with_the_wrong_key_yields_garbage() {
        let wire = seal("aes256-ctr", 0, &plain());

        let mut dec = decryptor("aes256-ctr");
        dec.initialize(&material(dec.iv_len(), 1), &material(dec.key_len(), 99))
            .expect("init failed");
        let mut out = Vec::new();
        dec.update(&wire, &mut out).expect("update failed");
        dec.finalize(&mut out).expect("finalize failed");

        assert_ne!(out, plain());
    }

    #[test]
    fn non_aead_ciphers_report_no_tag() {
        for name in ["aes256-ctr", "aes256-cbc", "3des-cbc"] {
            let enc = encryptor(name);
            assert!(!enc.is_galois_counter_mode(), "{name}");
            assert_eq!(enc.tag_len(), 0, "{name}");
            assert!(enc.block_size() >= 8, "{name}");
            assert!(enc.key_len() > 0 && enc.iv_len() > 0, "{name}");
        }
    }

    #[test]
    fn aead_ciphers_report_a_tag() {
        for name in [
            "chacha20-poly1305@openssh.com",
            "aes128-gcm@openssh.com",
            "aes256-gcm@openssh.com",
        ] {
            let enc = encryptor(name);
            assert!(enc.is_galois_counter_mode(), "{name}");
            assert_eq!(enc.tag_len(), 16, "{name}");
        }
    }

    /// Seals one packet the way `CipherStream::send_payload` does for an
    /// AEAD cipher: the 4-byte length header is authenticated as AAD.
    ///
    /// chacha20-poly1305 encrypts that header in place, so the returned
    /// header is the ciphertext that actually goes on the wire (and what the
    /// receiver must feed back into `additional_authenticated_data`).
    fn seal_aead(name: &str, seq: u32, plain: &[u8]) -> (Vec<u8>, Vec<u8>, [u8; 4]) {
        let mut enc = encryptor(name);
        enc.initialize(&material(enc.iv_len(), 1), &material(enc.key_len(), 2))
            .expect("init failed");
        enc.update_sequence_number(seq).expect("seq failed");

        let mut header = (plain.len() as u32 + 16).to_be_bytes();
        enc.additional_authenticated_data(&mut header)
            .expect("aad failed");

        let mut body = Vec::new();
        enc.update(plain, &mut body).expect("update failed");
        enc.finalize(&mut body).expect("finalize failed");

        let mut tag = vec![0u8; enc.tag_len()];
        enc.authentication_tag(&mut tag).expect("tag failed");
        (body, tag, header)
    }

    /// Returns the plaintext, or the verification error when the packet was
    /// tampered with (tag or authenticated header).
    fn open_aead(
        name: &str,
        seq: u32,
        body: &[u8],
        tag: &[u8],
        header: &[u8],
    ) -> error::Result<Vec<u8>> {
        let mut dec = decryptor(name);
        dec.initialize(&material(dec.iv_len(), 1), &material(dec.key_len(), 2))?;
        dec.update_sequence_number(seq)?;

        let mut header = header.to_vec();
        dec.additional_authenticated_data(&mut header)?;

        let mut out = Vec::new();
        dec.update(body, &mut out)?;
        dec.authentication_tag(tag)?;
        dec.finalize(&mut out)?;
        Ok(out)
    }

    #[test]
    fn chacha20_poly1305_round_trips_with_tag() {
        let name = "chacha20-poly1305@openssh.com";
        let (body, tag, header) = seal_aead(name, 7, &plain());
        assert_eq!(tag.len(), 16);

        assert_eq!(open_aead(name, 7, &body, &tag, &header).unwrap(), plain());
    }

    #[test]
    fn aead_rejects_a_flipped_tag() {
        let name = "chacha20-poly1305@openssh.com";
        let (body, mut tag, header) = seal_aead(name, 0, &plain());
        tag[0] ^= 0xff;

        assert!(
            open_aead(name, 0, &body, &tag, &header).is_err(),
            "a tampered tag must not verify"
        );
    }

    #[test]
    fn aead_rejects_a_flipped_header() {
        let name = "chacha20-poly1305@openssh.com";
        let (body, tag, mut header) = seal_aead(name, 0, &plain());

        // Same body/tag, but the authenticated length header disagrees.
        header[3] ^= 0x01;
        assert!(
            open_aead(name, 0, &body, &tag, &header).is_err(),
            "a tampered header must not verify"
        );
    }

    #[test]
    fn aead_rejects_a_flipped_body_byte() {
        let name = "chacha20-poly1305@openssh.com";
        let (mut body, tag, header) = seal_aead(name, 0, &plain());
        body[10] ^= 0xff;

        assert!(open_aead(name, 0, &body, &tag, &header).is_err());
    }

    #[test]
    fn initialize_accepts_correctly_sized_material() {
        let mut enc = encryptor("aes256-ctr");
        // aes256-ctr needs a 32-byte key and a 16-byte IV.
        assert!(enc.initialize(&[0u8; 16], &[0u8; 32]).is_ok());
    }

    /// KNOWN SHARP EDGE (not yet fixed): a key shorter than the cipher
    /// requires trips an assertion inside OpenSSL's `CipherCtx`
    /// (`key_len <= key.len()`) and panics rather than returning an error.
    ///
    /// The packet layer never hits this — key lengths come from the
    /// negotiated algorithm — but a caller driving the `Encrypt` trait
    /// directly can abort the thread.
    #[test]
    #[should_panic(expected = "key_len")]
    fn initialize_with_a_short_key_panics() {
        let mut enc = encryptor("aes256-ctr");
        let _ = enc.initialize(&[0u8; 16], &[0u8; 16]);
    }

    #[test]
    fn every_registered_cipher_has_sane_parameters() {
        for name in encrypt_all() {
            let enc = encryptor(name);
            assert_eq!(enc.name(), cipher_name(name));
            assert!(enc.block_size() > 0, "block_size of {name}");
            assert!(enc.key_len() > 0, "key_len of {name}");
            // AEAD ciphers take a nonce from the sequence number instead.
            if enc.is_galois_counter_mode() {
                assert_eq!(enc.tag_len(), 16, "tag_len of {name}");
            }
        }
    }

    /// See the note in `cipher::test`: `rijndael-cbc@lysator.liu.se` reports
    /// the canonical name of the cipher it is an alias for.
    fn cipher_name(registry_key: &str) -> &str {
        match registry_key {
            "rijndael-cbc@lysator.liu.se" => "aes256-cbc",
            other => other,
        }
    }
}
