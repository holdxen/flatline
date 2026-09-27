#![allow(missing_docs)]
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::session::scp;

    #[test]
    fn ok_wraps_the_value() {
        let value: Result<u32> = ok(7);
        assert_eq!(value.unwrap(), 7);
    }

    #[test]
    fn result_defaults_to_the_crate_error() {
        fn returns_ok() -> Result<&'static str> {
            Ok("fine")
        }
        fn returns_err() -> Result<&'static str> {
            Err(Error::InvalidArgument {
                detail: "nope".into(),
            })
        }

        assert_eq!(returns_ok().unwrap(), "fine");
        assert!(returns_err().is_err());
    }

    #[test]
    fn plain_variants_display_their_detail() {
        let cases: [(Error, &str); 5] = [
            (
                Error::InvalidFormat {
                    detail: "bad magic".into(),
                },
                "Invalid format: bad magic",
            ),
            (
                Error::InvalidOperation {
                    detail: "while closed".into(),
                },
                "Invalid operation: while closed",
            ),
            (
                Error::InvalidArgument {
                    detail: "port out of range".into(),
                },
                "Invalid argument: port out of range",
            ),
            (
                Error::IOError {
                    source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, "x"),
                },
                "IO error: x",
            ),
            (
                Error::Whatever {
                    message: "something odd".into(),
                    source: None,
                },
                "something odd",
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn whatever_carries_the_source_through() {
        let inner = std::io::Error::other("disk on fire");
        let err = Error::Whatever {
            message: "while reading config".into(),
            source: Some(Box::new(inner)),
        };

        assert_eq!(err.to_string(), "while reading config");
        let source = std::error::Error::source(&err).expect("source must be exposed");
        assert!(source.to_string().contains("disk on fire"));
    }

    #[test]
    fn whatever_without_a_source_has_none() {
        let err = Error::Whatever {
            message: "lonely".into(),
            source: None,
        };
        assert!(std::error::Error::source(&err).is_none());
    }

    #[test]
    fn transparent_wrappers_surface_the_inner_error() {
        // scp::Error is wrapped transparently through two layers
        // (scp::Error -> session::Error -> error::Error), so Display shows
        // the innermost message rather than a generic wrapper string.
        let inner = scp::Error::Failure {
            msg: "remote said no".into(),
        };
        let wrapped: Error = inner.into();
        assert!(
            wrapped.to_string().contains("remote said no"),
            "got: {wrapped}"
        );
    }

    #[test]
    fn transparent_wrappers_are_skipped_by_source() {
        // snafu's `transparent` forwards source() to the wrapped error's own
        // source, so the wrapper layers vanish from the chain: with an
        // innermost error that has no source, source() is None end to end.
        // Callers therefore match on the top-level variant rather than
        // walking source() to find which subsystem failed.
        let inner = scp::Error::Failure {
            msg: "remote said no".into(),
        };
        let wrapped: Error = inner.into();

        assert!(std::error::Error::source(&wrapped).is_none());

        // The variant is still reachable directly for structured matching.
        let Error::SessionError { source } = &wrapped else {
            panic!("expected SessionError, got {wrapped:?}");
        };
        assert!(matches!(
            source,
            crate::session::Error::SecureCopyProtocolError { .. }
        ));
    }

    #[test]
    fn scp_error_is_broken_only_for_remote_reports() {
        assert!(scp::Error::Failure { msg: "x".into() }.is_broken());
        assert!(scp::Error::Critical { msg: "x".into() }.is_broken());
        assert!(!scp::Error::UnexpectedResponse { detail: "x".into() }.is_broken());
        assert!(
            !scp::Error::InvalidTargetName {
                source: shlex::QuoteError::Nul,
            }
            .is_broken()
        );
    }

    #[test]
    fn session_error_variants_display_their_fields() {
        use crate::session::Error as SessionError;

        let cases: [(SessionError, &str); 4] = [
            (
                SessionError::UnexpectedBehaviour {
                    detail: "window went negative".into(),
                },
                "Unexpected behaviour: window went negative",
            ),
            (SessionError::ChannelFailure, "Channel failure"),
            (SessionError::ChannelAlreadyOpen, "Channel already open"),
            (
                SessionError::ChannelWindowOverflow,
                "Channel window overflow",
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn session_error_carries_channel_open_failure_codes() {
        use crate::session::Error as SessionError;

        let err = SessionError::ChannelOpenFailure {
            reason_code: 2,
            description: "connect failed".into(),
        };
        let text = err.to_string();
        assert!(text.contains('2'), "code missing from: {text}");
        assert!(
            text.contains("connect failed"),
            "description missing from: {text}"
        );
    }

    #[test]
    fn cipher_errors_display_distinctly() {
        use crate::cipher::Error as CipherError;

        assert_eq!(CipherError::InvalidPrime.to_string(), "Invalid prime");
        assert_eq!(
            CipherError::MacVerificationFailed.to_string(),
            "MAC verification failed"
        );
        assert_eq!(
            CipherError::SignatureVerificationFailed.to_string(),
            "Signature verification failed"
        );
    }

    #[test]
    fn transport_error_displays_packet_limits() {
        use crate::ssh::Error as TransportError;

        let err = TransportError::PayloadTooLong {
            maximum: 32768,
            actual: 40000,
        };
        assert_eq!(err.to_string(), "payload is too long");

        let err = TransportError::PacketTooLong {
            maximum: 262144,
            actual: 300000,
        };
        assert_eq!(err.to_string(), "packet is too long");
    }

    #[test]
    fn handshake_error_displays_version_and_banner() {
        use crate::session::HandshakeError;

        let err = HandshakeError::UnsupportedVersion {
            version: "SSH-1.5-legacy".into(),
        };
        assert_eq!(err.to_string(), "Unsupported SSH version: SSH-1.5-legacy");

        let err = HandshakeError::UnexpectedDisconnectMessage {
            reason: crate::ssh::msg::DisconnectReason(11),
            description: "bye".into(),
        };
        assert!(err.to_string().contains("bye"));

        assert_eq!(
            HandshakeError::NegotiationFailed.to_string(),
            "Negotiation failed"
        );
        assert_eq!(
            HandshakeError::SignatureVerificationFailed.to_string(),
            "Signature verification failed"
        );
        assert_eq!(
            HandshakeError::ServerHostKeyRejectedByUser.to_string(),
            "Server host key rejected by user"
        );
    }

    #[test]
    fn errors_implement_std_error() {
        // Every error must be usable as a boxed std::error::Error, which is
        // what anyhow/Box<dyn Error> users rely on.
        fn assert_std_error<E: std::error::Error + Send + Sync + 'static>(_: &E) {}

        assert_std_error(&Error::InvalidArgument { detail: "x".into() });
        assert_std_error(&Error::Whatever {
            message: "x".into(),
            source: None,
        });
    }

    #[test]
    fn errors_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Error>();
    }
}
