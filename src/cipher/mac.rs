//! Message authentication for SSH binary packets (RFC 4253, section 6.4).
//!
//! A [`Mac`] instance authenticates one direction of a connection. Besides
//! the plain MACs of RFC 4253, the OpenSSH `*-etm@openssh.com`
//! (encrypt-then-MAC) variants are supported, in which the tag is computed
//! over the encrypted packet instead of the plaintext.

use openssl::{
    md::{Md, MdRef},
    md_ctx::MdCtx,
    pkey::{PKey, Private},
};
use snafu::{OptionExt, ResultExt};

use super::Factory;
use crate::error::Result;
use crate::error::builder;
use indexmap::IndexMap;

algo_list!(
    "message authentication code",
    all,
    new_all,
    new_mac_by_name,
    dyn Mac + Send,
    #[cfg(feature = "umac")]
    "umac-128-etm@openssh.com" => UMac::umac128_etm_openssh(),
    #[cfg(feature = "umac")]
    "umac-128@openssh.com" => UMac::umac128_openssh(),
    #[cfg(feature = "umac")]
    "umac-64@openssh.com" => UMac::umac64_openssh(),
    #[cfg(feature = "umac")]
    "umac-64-etm@openssh.com" => UMac::umac64_etm_openssh(),
    "hmac-sha2-512-etm@openssh.com" => HMac::new(
        "hmac-sha2-512-etm@openssh.com".to_string(),
        64,
        64,
        true,
        Md::sha512(),
    ),
    "hmac-sha2-256-etm@openssh.com" => HMac::new(
        "hmac-sha2-256-etm@openssh.com".to_string(),
        32,
        32,
        true,
        Md::sha256(),
    ),
    "hmac-sha1-etm@openssh.com" => HMac {
        name: "hmac-sha1-etm@openssh.com".to_string(),
        mac_len: 20,
        key_len: 20,
        ctx: None,
        key: None,
        encrypt_then_mac: true,
        digest: Md::sha1(),
    },
    "hmac-sha1" => HMac {
        name: "hmac-sha1".to_string(),
        mac_len: 20,
        key_len: 20,
        ctx: None,
        key: None,
        encrypt_then_mac: false,
        digest: Md::sha1(),
    },
    "hmac-sha1-96" => HMac::new(
        "hmac-sha1-96".to_string(),
        12,
        20,
        false,
        Md::sha1(),
    ),
    "hmac-sha1-96-etm@openssh.com" => HMac::new(
        "hmac-sha1-96-etm@openssh.com".to_string(),
        12,
        20,
        true,
        Md::sha1(),
    ),
    "hmac-md5" => HMac::new(
        "hmac-md5".to_string(),
        16,
        16,
        false,
        Md::md5(),
    ),
    "hmac-md5-etm@openssh.com" => HMac::new(
        "hmac-md5-etm@openssh.com".to_string(),
        16,
        16,
        true,
        Md::md5(),
    ),
    "hmac-md5-96" => HMac::new(
        "hmac-md5-96".to_string(),
        12,
        16,
        false,
        Md::md5(),
    ),
    "hmac-md5-96-etm@openssh.com" => HMac::new(
        "hmac-md5-96-etm@openssh.com".to_string(),
        12,
        16,
        true,
        Md::md5(),
    ),
    "hmac-sha2-512" => HMac::new(
        "hmac-sha2-512".to_string(),
        64,
        64,
        false,
        Md::sha512(),
    ),
    "hmac-sha2-256" => HMac::new(
        "hmac-sha2-256".to_string(),
        32,
        32,
        false,
        Md::sha256(),
    ),
    // "hmac-ripemd160@openssh.com" => HMac::new(
    //     "hmac-sha1-96-etm@openssh.com".to_string(),
    //     20,
    //     20,
    //     false,
    //     Md::ripemd160(),
    // ),
    // "hmac-ripemd160-etm@openssh.com" => HMac::new(
    //     "hmac-sha1-96-etm@openssh.com".to_string(),
    //     20,
    //     20,
    //     true,
    //     Md::ripemd160(),
    // ),
);

/// Computes the message authentication code that protects outgoing SSH binary
/// packets, or verifies the tag of incoming ones (RFC 4253, section 6.4).
///
/// One instance serves one direction of a connection. `update` is called with
/// the packet sequence number first and then with the packet bytes, and
/// `finalize` produces (or, on the receive side, is compared against) the
/// tag for that packet.
pub trait Mac {
    /// Returns the SSH MAC algorithm name (`hmac-sha2-256`,
    /// `umac-128-etm@openssh.com`, ...) as it appears in `SSH_MSG_KEXINIT`.
    fn name(&self) -> &str;
    /// Returns whether this is an encrypt-then-MAC algorithm (any name ending
    /// in `-etm@openssh.com`).
    ///
    /// In encrypt-then-MAC mode the tag is computed over the encrypted packet
    /// (with the 4-byte length header left in the clear), as an OpenSSH
    /// extension; otherwise the packet is authenticated first and then
    /// encrypted, as specified in RFC 4253.
    fn encrypt_then_mac(&self) -> bool;
    /// Returns the MAC key length in bytes, which determines how many key
    /// bytes are derived for this direction during key exchange.
    fn key_len(&self) -> usize;
    /// Returns the length in bytes of the tag produced by `finalize`.
    ///
    /// Some algorithms truncate their tag, for example `hmac-sha1-96` emits
    /// only 12 bytes.
    fn mac_len(&self) -> usize;
    /// Sets the MAC key; must be called before `update`.
    fn initialize(&mut self, key: &[u8]) -> Result<()>;
    /// Feeds data into the tag computation.
    ///
    /// The first call for a packet carries the 4-byte packet sequence number
    /// (big-endian) and later calls carry the packet bytes. UMAC
    /// implementations consume those leading bytes as the packet nonce and
    /// exclude them from the message, while HMAC algorithms cover the
    /// sequence number and packet together, as required by RFC 4253.
    fn update(&mut self, data: &[u8]) -> Result<()>;
    /// Completes the tag for the data fed since the last call, returns it,
    /// and resets the state so the next packet can be processed.
    ///
    /// The result is [`mac_len`](Mac::mac_len) bytes long. Fails if
    /// `initialize` has not been called or the packet's sequence number was
    /// never supplied.
    fn finalize(&mut self) -> Result<Vec<u8>>;
}

/// The UMAC message authentication code (UMAC-64 and UMAC-128) used by
/// OpenSSH's `umac-64@openssh.com`, `umac-128@openssh.com` and their `-etm`
/// variants.
///
/// Only available with the `umac` cargo feature. `KEY` and `TAG` are the key
/// and tag sizes in bytes and `T` is the underlying `umac` crate
/// implementation.
#[cfg(feature = "umac")]
pub struct UMac<const KEY: usize, const TAG: usize, T: umac::UMac<KEY, TAG>> {
    algorithm: Option<T>,
    name: &'static str,
    encrypt_then_mac: bool,
    sequence_number: Option<u32>,
}

#[cfg(feature = "umac")]
impl UMac<16, 16, umac::UMac128> {
    fn umac128_openssh() -> Self {
        Self::new("umac-128@openssh.com", false)
    }
    fn umac128_etm_openssh() -> Self {
        Self::new("umac-128-etm@openssh.com", true)
    }
}

#[cfg(feature = "umac")]
impl UMac<16, 8, umac::UMac64> {
    fn umac64_openssh() -> Self {
        Self::new("umac-64@openssh.com", false)
    }
    fn umac64_etm_openssh() -> Self {
        Self::new("umac-64-etm@openssh.com", true)
    }
}

#[cfg(feature = "umac")]
impl<const KEY: usize, const TAG: usize, T: umac::UMac<KEY, TAG>> UMac<KEY, TAG, T> {
    fn new(name: &'static str, encrypt_then_mac: bool) -> Self {
        Self {
            algorithm: None,
            name,
            encrypt_then_mac,
            sequence_number: None,
        }
    }
}

#[cfg(feature = "umac")]
impl<const KEY: usize, const TAG: usize, T: umac::UMac<KEY, TAG>> Mac for UMac<KEY, TAG, T> {
    fn name(&self) -> &str {
        self.name
    }

    fn encrypt_then_mac(&self) -> bool {
        self.encrypt_then_mac
    }

    fn key_len(&self) -> usize {
        T::KEY_LEN
    }

    fn mac_len(&self) -> usize {
        T::TAG_LEN
    }

    fn initialize(&mut self, key: &[u8]) -> Result<()> {
        self.algorithm = Some(T::new(key.try_into().unwrap()));
        // self.key = Some(key.to_vec());
        Ok(())
    }

    fn update(&mut self, data: &[u8]) -> Result<()> {
        // if self.algorithm.is_none() {
        //     let key = self.key.as_ref().context(builder::InvalidOperation {
        //         detail: "Uninitialized",
        //     })?;

        //     self.algorithm = Some(T::new(key[..].try_into().unwrap()));
        // }

        // let ctx = self.algorithm.as_mut().unwrap();
        //

        let ctx = self.algorithm.as_mut().context(builder::InvalidOperation {
            detail: "Uninitialized",
        })?;

        if self.sequence_number.is_none() {
            self.sequence_number = Some(u32::from_be_bytes(data[..4].try_into().unwrap()));
            if data.len() > 4 {
                ctx.update(&data[4..]);
            }
        } else {
            ctx.update(data);
        }
        Ok(())
    }

    fn finalize(&mut self) -> Result<Vec<u8>> {
        let ctx = self.algorithm.as_mut().context(builder::InvalidOperation {
            detail: "Uninitialized",
        })?;
        let sequence_number = self.sequence_number.context(builder::InvalidOperation {
            detail: "Uninitialized",
        })?;
        let mut nonce = [0u8; 8];
        nonce[4..].copy_from_slice(&sequence_number.to_be_bytes());

        let tag = ctx.finalize(nonce);
        self.sequence_number = None;
        Ok(tag.to_vec())
    }
}

struct HMac {
    name: String,
    mac_len: usize,
    key_len: usize,
    ctx: Option<MdCtx>,
    key: Option<PKey<Private>>,
    encrypt_then_mac: bool,
    digest: &'static MdRef,
}

impl HMac {
    fn new(
        name: String,
        mac_len: usize,
        key_len: usize,
        encrypt_then_mac: bool,
        digest: &'static MdRef,
    ) -> Self {
        Self {
            name,
            mac_len,
            key_len,
            ctx: None,
            key: None,
            encrypt_then_mac,
            digest,
        }
    }

    fn get_ctx_mut(&mut self) -> Result<&mut MdCtx> {
        match self.ctx {
            Some(ref mut ctx) => Ok(ctx),
            None => builder::InvalidOperation {
                detail: "Uninitialized",
            }
            .fail(),
        }
    }
}

impl Mac for HMac {
    fn encrypt_then_mac(&self) -> bool {
        self.encrypt_then_mac
    }
    fn key_len(&self) -> usize {
        self.key_len
    }

    fn mac_len(&self) -> usize {
        self.mac_len
    }

    fn initialize(&mut self, key: &[u8]) -> Result<()> {
        let pkey = PKey::hmac(key).context(builder::OpenSSL)?;

        let mut ctx = MdCtx::new().context(builder::OpenSSL)?;
        ctx.digest_sign_init(Some(self.digest), &pkey)
            .context(builder::OpenSSL)?;

        self.ctx = Some(ctx);
        self.key = Some(pkey);
        Ok(())
    }

    fn update(&mut self, data: &[u8]) -> Result<()> {
        self.get_ctx_mut()?
            .digest_sign_update(data)
            .context(builder::OpenSSL)?;
        Ok(())
    }

    fn finalize(&mut self) -> Result<Vec<u8>> {
        let ctx = self.ctx.as_mut().context(builder::InvalidOperation {
            detail: "Uninitialized",
        })?;

        let size = ctx.digest_sign_final(None).context(builder::OpenSSL)?;
        let mut buf = vec![0; size];
        ctx.digest_sign_final(Some(&mut buf))
            .context(builder::OpenSSL)?;
        buf.truncate(self.mac_len);

        let mut ctx = MdCtx::new().context(builder::OpenSSL)?;
        if let Some(ref pkey) = self.key {
            ctx.digest_sign_init(Some(self.digest), pkey)
                .context(builder::OpenSSL)?;
        }
        self.ctx = Some(ctx);
        Ok(buf)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// A key sized for the HMAC algorithms under test (32 bytes).
    const KEY: [u8; 32] = [0x42; 32];

    fn mac(name: &str) -> Box<dyn Mac + Send> {
        new_mac_by_name(name).expect("unknown mac")()
    }

    /// Computes a tag over (sequence number, packet) as the packet layer does.
    fn tag_over(m: &mut Box<dyn Mac + Send>, seq: u32, packet: &[u8]) -> Vec<u8> {
        m.update(&seq.to_be_bytes()).unwrap();
        m.update(packet).unwrap();
        m.finalize().unwrap()
    }

    #[test]
    fn hmac_is_deterministic_for_the_same_input() {
        let mut a = mac("hmac-sha2-256");
        let mut b = mac("hmac-sha2-256");
        a.initialize(&KEY).unwrap();
        b.initialize(&KEY).unwrap();

        assert_eq!(
            tag_over(&mut a, 0, b"payload"),
            tag_over(&mut b, 0, b"payload")
        );
    }

    #[test]
    fn hmac_changes_with_key_sequence_and_packet() {
        let base = {
            let mut m = mac("hmac-sha2-256");
            m.initialize(&KEY).unwrap();
            tag_over(&mut m, 0, b"payload")
        };

        let mut other_key = mac("hmac-sha2-256");
        other_key.initialize(&[0x43; 32]).unwrap();
        assert_ne!(tag_over(&mut other_key, 0, b"payload"), base);

        let mut other_seq = mac("hmac-sha2-256");
        other_seq.initialize(&KEY).unwrap();
        assert_ne!(tag_over(&mut other_seq, 1, b"payload"), base);

        let mut other_packet = mac("hmac-sha2-256");
        other_packet.initialize(&KEY).unwrap();
        assert_ne!(tag_over(&mut other_packet, 0, b"payloae"), base);
    }

    #[test]
    fn hmac_state_resets_between_packets() {
        let mut m = mac("hmac-sha2-256");
        m.initialize(&KEY).unwrap();

        let first = tag_over(&mut m, 5, b"packet");
        let second = tag_over(&mut m, 5, b"packet");

        // finalize() must reset the context, so repeating the same packet
        // yields the same tag rather than extending the first one.
        assert_eq!(first, second);
    }

    #[test]
    fn mac_tag_lengths_match_protocol_expectations() {
        let cases = [
            ("hmac-sha2-256", 32, 32),
            ("hmac-sha2-512", 64, 64),
            ("hmac-sha1", 20, 20),
            ("hmac-sha1-96", 12, 20), // truncated tag, full key
        ];
        for (name, mac_len, key_len) in cases {
            let m = mac(name);
            assert_eq!(m.mac_len(), mac_len, "mac_len of {name}");
            assert_eq!(m.key_len(), key_len, "key_len of {name}");
        }
    }

    #[test]
    fn etm_variants_are_flagged_and_plain_ones_are_not() {
        assert!(mac("hmac-sha2-256-etm@openssh.com").encrypt_then_mac());
        assert!(mac("hmac-sha1-etm@openssh.com").encrypt_then_mac());
        assert!(!mac("hmac-sha2-256").encrypt_then_mac());
        assert!(!mac("hmac-sha1").encrypt_then_mac());
    }

    #[test]
    fn truncated_tag_is_a_prefix_of_the_full_tag() {
        // hmac-sha1-96 emits only the first 12 bytes of the SHA-1 tag.
        let mut truncated = mac("hmac-sha1-96");
        let mut full = mac("hmac-sha1");
        truncated.initialize(&KEY).unwrap();
        full.initialize(&KEY).unwrap();

        assert_eq!(
            tag_over(&mut truncated, 0, b"payload"),
            &tag_over(&mut full, 0, b"payload")[..12]
        );
    }

    #[test]
    fn update_without_initialize_fails() {
        let mut m = mac("hmac-sha2-256");
        assert!(m.update(b"data").is_err());
    }

    #[test]
    fn finalize_without_initialize_fails() {
        let mut m = mac("hmac-sha2-256");
        assert!(m.finalize().is_err());
    }

    #[test]
    fn every_registered_mac_reports_its_registered_name() {
        for name in all() {
            let mut m = new_mac_by_name(name).expect("missing factory")();
            assert_eq!(m.name(), *name);

            // Each algorithm wants exactly `key_len()` bytes: HMAC accepts
            // anything, but UMAC slices the key to a fixed size.
            let key = vec![0x42; m.key_len()];
            m.initialize(&key).expect("initialize failed");

            // Drive it the way the packet layer does: sequence number
            // first, then the packet bytes. UMAC needs the sequence number
            // to build its nonce, so a bare finalize is not enough.
            m.update(&7u32.to_be_bytes()).expect("update seq");
            m.update(b"packet bytes").expect("update payload");

            let tag = m.finalize().expect("finalize failed");
            assert_eq!(tag.len(), m.mac_len(), "tag length of {name}");
        }
    }

    #[test]
    fn finalize_without_a_sequence_number_fails_for_umac() {
        // UMAC derives its nonce from the packet sequence number, which
        // only arrives through `update`, so finalize alone is an error
        // (documented behaviour, not a defect).
        #[cfg(feature = "umac")]
        {
            let mut m = mac("umac-128@openssh.com");
            m.initialize(&vec![0x42; m.key_len()]).expect("init");
            assert!(m.finalize().is_err());
        }
    }

    #[test]
    fn hmac_accepts_a_longer_key_than_it_declares() {
        // HMAC keys are hashed down, so an over-long key is fine — unlike
        // UMAC, which needs an exact match (see the panic note below).
        let mut m = mac("hmac-sha2-256");
        assert_eq!(m.key_len(), 32);
        assert!(m.initialize(&[0x42; 64]).is_ok());
    }

    /// KNOWN SHARP EDGE (not yet fixed): `UMac::initialize` does
    /// `key.try_into().unwrap()` with a fixed-size array target, so a key of
    /// the wrong length panics (`TryFromSliceError`) instead of returning an
    /// error — the same shape of problem as the cipher `initialize` path.
    ///
    /// The packet layer is safe: key lengths come from the negotiated
    /// algorithm's `key_len()`. Only a direct caller supplying a mismatched
    /// key can hit it.
    ///
    /// Only meaningful with `--features umac`.
    #[test]
    #[cfg(feature = "umac")]
    #[should_panic(expected = "TryFromSliceError")]
    fn umac_initialize_with_a_wrong_length_key_panics() {
        let mut m = mac("umac-128@openssh.com");
        // umac-128 wants a 16-byte key; give it 32.
        let _ = m.initialize(&[0x42; 32]);
    }
}
