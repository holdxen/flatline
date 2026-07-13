//! Crate-wide error type, [`Result`] alias, and helper function.
//!
//! [`Error`] is the error type returned by fallible operations throughout the
//! crate. Failures of other subsystems are embedded in it as transparent
//! wrappers of [`crate::ssh::Error`] (transport-layer errors),
//! [`crate::cipher::Error`], [`crate::session::HandshakeError`],
//! [`crate::session::Error`] and [`crate::key::Error`]; the remaining variants
//! carry their own `detail` message, `source` error, or catch-all `message`.
//!
//! The enum is derived with `snafu`, and its `module(builder)` configuration
//! generates the public [`builder`] module of context selectors (builders), for
//! example `builder::InvalidFormat`, `builder::InvalidOperation`, `builder::IO`
//! (for [`Error::IOError`]) and `builder::OpenSSL` (for [`Error::OpenSSLError`]);
//! because of `context(suffix(false))`, selectors are named after the variant
//! with any trailing `Error` removed and no extra suffix. The transparent
//! variants get `From` conversions for their wrapped error instead, and
//! [`Error::Whatever`] is typically produced with snafu's `whatever_context` /
//! `with_whatever_context` helpers.

/// A specialized [`Result`] type where the error defaults to [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Wraps a value in [`Ok`] as a crate [`Result`] with the default error type.
pub fn ok<T>(t: T) -> Result<T> {
    Ok(t)
}

use crate::cipher::Error as CipherError;
use crate::session::Error as SessionError;
use crate::session::HandshakeError;
use crate::ssh::Error as TransportError;

/// The error type returned by fallible operations in this crate.
#[derive(Debug, snafu::Snafu)]
#[snafu(module(builder), context(suffix(false)), visibility(pub))]
pub enum Error {
    /// The input did not match the expected format; `detail` says what was wrong.
    #[snafu(display("Invalid format: {}", detail))]
    InvalidFormat {
        /// What was wrong with the input.
        detail: String,
    },
    /// A transport-layer (binary packet protocol) error, transparently wrapping [`crate::ssh::Error`].
    #[snafu(transparent)]
    TransportError {
        /// The underlying transport error.
        source: TransportError,
    },
    /// An I/O error, wrapping [`std::io::Error`].
    #[snafu(display("IO error: {}", source))]
    IOError {
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// An OpenSSL error, wrapping `openssl::error::ErrorStack`.
    #[snafu(display("OpenSSL error: {}", source))]
    OpenSSLError {
        /// The underlying OpenSSL error stack.
        source: openssl::error::ErrorStack,
    },
    /// An operation that is not valid in the current state; `detail` describes it.
    #[snafu(display("Invalid operation: {}", detail))]
    InvalidOperation {
        /// Description of the invalid operation.
        detail: String,
    },
    /// A cipher error, transparently wrapping [`crate::cipher::Error`].
    #[snafu(transparent)]
    CipherError {
        /// The underlying cipher error.
        source: CipherError,
    },
    /// A key-exchange handshake failure, transparently wrapping [`crate::session::HandshakeError`].
    #[snafu(transparent)]
    HandshakeError {
        /// The underlying handshake error.
        source: HandshakeError,
    },
    /// A session-layer error, transparently wrapping [`crate::session::Error`].
    #[snafu(transparent)]
    SessionError {
        /// The underlying session error.
        source: SessionError,
    },
    /// A key parsing error, transparently wrapping [`crate::key::Error`].
    #[snafu(transparent)]
    KeyError {
        /// The underlying key error.
        source: crate::key::Error,
    },
    /// An invalid argument; `detail` says which argument was invalid.
    #[snafu(display("Invalid argument: {}", detail))]
    InvalidArgument {
        /// Which argument was invalid, and why.
        detail: String,
    },

    /// A catch-all variant holding an arbitrary `message` plus an optional boxed `source` error.
    ///
    /// Typically produced with snafu's `whatever_context` /
    /// `with_whatever_context` helpers or the `whatever!` macro.
    #[snafu(whatever, display("{message}"))]
    Whatever {
        /// The arbitrary error message.
        message: String,
        /// The underlying error, if one was attached.
        #[snafu(source(from(Box<dyn std::error::Error + Send + Sync + 'static>, Some)))]
        source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
    },
}
