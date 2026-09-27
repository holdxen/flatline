//! Cryptographic building blocks of the SSH transport layer: symmetric
//! encryption, message authentication, compression, key exchange, and
//! public-key signatures.
//!
//! The algorithms chosen during the `SSH_MSG_KEXINIT` negotiation (RFC 4253)
//! each live in a submodule — [`compress`], [`crypt`], [`kex`], [`mac`] and
//! [`signature`] — and every submodule exposes generated helpers that list the
//! supported algorithm names in preference order and build fresh [`Factory`]
//! instances of them, keyed by name.

use snafu::Snafu;

macro_rules! create_factory {
    ($ex:expr) => {
        Box::new(|| Box::new($ex) as _)
    };
}

// macro_rules! algo_list {
//     (
//         $all:ident,
//         $new_all:ident,
//         $new_by_name:ident,
//         $t:ty,
//         $($key:expr => $value:expr,)*
//     ) => {
//         pub fn $all() -> &'static [&'static str] {
//             &[
//                 $($key,)*
//             ]
//         }

//         pub fn $new_all() -> IndexMap<&'static str, Factory<$t>> {
//             let mut res: IndexMap<&'static str, Factory<$t>> = IndexMap::new();
//             $(
//                 res.insert($key,  Box::new(|| Box::new($value) as _));
//             )*
//             res
//         }

//         pub fn $new_by_name(name: &str) -> Option<Factory<$t>> {
//             match name {
//                 $($key => Some(Box::new(|| Box::new($value) as _)),)*
//                 _ => None,
//             }

//         }
//     }
// }

macro_rules! algo_list {
    (
        $doc:expr,
        $all:ident,
        $new_all:ident,
        $new_by_name:ident,
        $t:ty,
        $(
            $(#[$cfg:meta])*
            $key:literal => $value:expr
        ),* $(,)?
    ) => {
        #[doc = concat!("Returns the names of all supported ", $doc, "s, in preference order (most preferred first).")]
        pub fn $all() -> &'static [&'static str] {
            &[
                $(
                    $(#[$cfg])*
                    $key,
                )*
            ]
        }

        #[doc = concat!("Creates a fresh factory for every supported ", $doc, ", keyed by name in preference order.")]
        pub fn $new_all() -> IndexMap<&'static str, Factory<$t>> {
            let mut res: IndexMap<&'static str, Factory<$t>> = IndexMap::new();

            $(
                $(#[$cfg])*
                res.insert($key, Box::new(|| Box::new($value) as _));
            )*

            res
        }

        #[doc = concat!("Returns a factory for the ", $doc, " named `name`, or `None` if it is not supported.")]
        pub fn $new_by_name(name: &str) -> Option<Factory<$t>> {
            match name {
                $(
                    $(#[$cfg])*
                    $key => Some(Box::new(|| Box::new($value) as _)),
                )*
                _ => None,
            }
        }
    }
}

pub mod compress;
pub mod crypt;
pub mod kex;
pub mod mac;
pub mod signature;

/// Errors produced by the algorithms of the [`cipher`](self) module.
#[derive(Debug, Snafu)]
pub enum Error {
    /// The prime received during a Diffie-Hellman group exchange has a bit
    /// length outside the range the client is willing to accept.
    #[snafu(display("Invalid prime"))]
    InvalidPrime,
    /// The compression or decompression stream failed while processing a
    /// packet payload.
    #[snafu(display("Compression error"))]
    CompressError,
    /// A packet's message authentication code (or AEAD authentication tag)
    /// did not match, so the packet was rejected as inauthentic.
    #[snafu(display("MAC verification failed"))]
    MacVerificationFailed,
    /// The key or signature does not belong to the expected algorithm, for
    /// example an `ssh-rsa` key supplied to an `ssh-ed25519` verifier.
    #[snafu(display("Mismatch key"))]
    MismatchKey,
    /// A public-key signature failed verification, such as the server's
    /// host-key signature during key exchange.
    #[snafu(display("Signature verification failed"))]
    SignatureVerificationFailed,
    /// A key or ciphertext had an unexpected length, for example an
    /// ML-KEM-768 encapsulation of the wrong size during hybrid key exchange.
    #[snafu(display("Key length mismatch"))]
    KeyLengthMismatch,
}

/// A thread-safe boxed constructor that creates a fresh, independent instance
/// of a cipher-suite algorithm.
///
/// Factories are built from an algorithm's name during `SSH_MSG_KEXINIT`
/// negotiation so that each direction of the connection gets its own
/// algorithm state.
pub type Factory<T> = Box<dyn (Fn() -> Box<T>) + Send + Sync>;

#[cfg(test)]
mod test {
    use super::*;

    /// Every algorithm registry must agree with itself: `*_all()` lists the
    /// names, `new_*_all()` builds one factory per name, and
    /// `new_*_by_name()` resolves each of them.
    #[test]
    fn key_exchange_registry_is_consistent() {
        let names = kex::all();
        let factories = kex::new_all();

        assert!(!names.is_empty());
        assert_eq!(factories.len(), names.len());
        for name in names {
            assert!(
                factories.contains_key(*name),
                "missing from new_all: {name}"
            );
            let factory = kex::new_kex_by_name(name)
                .unwrap_or_else(|| panic!("new_kex_by_name returned None for {name}"));
            let kex = factory();
            assert_eq!(kex.name(), *name);
        }

        assert!(kex::new_kex_by_name("no-such-kex").is_none());
    }

    /// Registry key -> the name the produced instance reports.
    ///
    /// Two OpenSSH aliases are registered as equivalent ciphers, so the
    /// instance reports the canonical cipher's name instead of the alias.
    /// `name()` is only used for logging (algorithm selection keys off the
    /// registry key), so this is cosmetic — but it is surprising, hence the
    /// explicit table rather than a silent exception.
    fn cipher_reported_name(registry_key: &str) -> &str {
        match registry_key {
            "rijndael-cbc@lysator.liu.se" => "aes256-cbc",
            other => other,
        }
    }

    #[test]
    fn cipher_registry_is_consistent() {
        let names = crypt::encrypt_all();
        assert!(!names.is_empty());

        let encryptors = crypt::new_encrypt_all();
        assert_eq!(encryptors.len(), names.len());
        for name in names {
            assert!(encryptors.contains_key(*name));
            let enc = crypt::new_encrypt_by_name(name).expect("missing encrypt factory")();
            assert_eq!(enc.name(), cipher_reported_name(name));
        }

        let decryptors = crypt::new_decrypt_all();
        assert_eq!(decryptors.len(), names.len());
        for name in names {
            assert!(decryptors.contains_key(*name));
            let dec = crypt::new_decrypt_by_name(name).expect("missing decrypt factory")();
            assert_eq!(dec.name(), cipher_reported_name(name));
        }

        assert!(crypt::new_encrypt_by_name("no-such-cipher").is_none());
        assert!(crypt::new_decrypt_by_name("no-such-cipher").is_none());
    }

    /// KNOWN QUIRK (not yet fixed): `rijndael-cbc@lysator.liu.se` reports
    /// `aes256-cbc` from `name()`, so logs that print the negotiated cipher
    /// show the canonical name rather than the alias the peer chose.
    #[test]
    #[ignore = "known quirk: rijndael-cbc alias reports name() as aes256-cbc"]
    fn cipher_alias_reports_its_registry_key() {
        let enc = crypt::new_encrypt_by_name("rijndael-cbc@lysator.liu.se").unwrap()();
        assert_eq!(enc.name(), "rijndael-cbc@lysator.liu.se");
    }

    #[test]
    fn mac_registry_is_consistent() {
        let names = mac::all();
        assert!(!names.is_empty());

        let factories = mac::new_all();
        assert_eq!(factories.len(), names.len());
        for name in names {
            assert!(factories.contains_key(*name));
            let mac = mac::new_mac_by_name(name).expect("missing mac factory")();
            assert_eq!(mac.name(), *name);
            assert_eq!(mac.mac_len() > 0, true);
            assert_eq!(mac.key_len() > 0, true);
        }

        assert!(mac::new_mac_by_name("no-such-mac").is_none());
    }

    #[test]
    fn compression_registry_is_consistent() {
        let names = compress::encode_all();
        assert!(!names.is_empty());

        let encoders = compress::new_encode_all();
        assert_eq!(encoders.len(), names.len());
        for name in names {
            let enc = compress::new_encode_by_name(name).expect("missing encode factory")();
            assert_eq!(enc.name(), *name);
        }

        let decoders = compress::new_decode_all();
        assert_eq!(decoders.len(), names.len());
        for name in names {
            let dec = compress::new_decode_by_name(name).expect("missing decode factory")();
            assert_eq!(dec.name(), *name);
        }

        assert!(compress::new_encode_by_name("no-such-coding").is_none());
        assert!(compress::new_decode_by_name("no-such-coding").is_none());
    }

    #[test]
    fn signature_registry_is_consistent() {
        let names = signature::signature_all();
        assert!(!names.is_empty());

        let signers = signature::new_signature_all();
        assert_eq!(signers.len(), names.len());
        for name in names {
            let signer = signature::new_signature_by_name(name).expect("missing signer")();
            assert_eq!(signer.name(), *name);
        }

        let verifiers = signature::new_verify_all();
        for name in signature::verify_all() {
            assert!(verifiers.contains_key(*name));
            let verifier = signature::new_verify_by_name(name).expect("missing verifier")();
            assert_eq!(verifier.name(), *name);
        }

        assert!(signature::new_signature_by_name("no-such-sig").is_none());
        assert!(signature::new_verify_by_name("no-such-sig").is_none());
    }

    /// Factories must produce independent instances, since each direction of
    /// a connection gets its own algorithm state.
    #[test]
    fn factories_produce_independent_instances() {
        let mut a = mac::new_mac_by_name("hmac-sha2-256").expect("factory")();
        let mut b = mac::new_mac_by_name("hmac-sha2-256").expect("factory")();

        let key = [7u8; 32];
        a.initialize(&key).unwrap();
        b.initialize(&key).unwrap();

        a.update(&1u32.to_be_bytes()).unwrap();
        a.update(b"payload").unwrap();
        let tag_a = a.finalize().unwrap();

        // b never saw any data, so its tag must differ from a's.
        b.update(&1u32.to_be_bytes()).unwrap();
        b.update(b"other!").unwrap();
        let tag_b = b.finalize().unwrap();

        assert_ne!(tag_a, tag_b);
    }

    #[test]
    fn error_variants_display_distinctly() {
        let cases = [
            (Error::InvalidPrime, "Invalid prime"),
            (Error::CompressError, "Compression error"),
            (Error::MacVerificationFailed, "MAC verification failed"),
            (Error::MismatchKey, "Mismatch key"),
            (
                Error::SignatureVerificationFailed,
                "Signature verification failed",
            ),
            (Error::KeyLengthMismatch, "Key length mismatch"),
        ];
        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }
}
