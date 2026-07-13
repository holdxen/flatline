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
