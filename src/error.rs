pub type Result<T, E = Error> = std::result::Result<T, E>;

pub fn ok<T>(t: T) -> Result<T> {
    Ok(t)
}

use crate::cipher::Error as CipherError;
use crate::session::Error as SessionError;
use crate::session::HandshakeError;
use crate::ssh::stream::Error as TransportError;

#[derive(Debug, snafu::Snafu)]
#[snafu(module(builder), context(suffix(false)), visibility(pub))]
pub enum Error {
    #[snafu(display("Invalid format: {}", detail))]
    InvalidFormat {
        detail: String,
    },
    #[snafu(transparent)]
    TransportError {
        source: TransportError,
    },
    IOError {
        source: std::io::Error,
    },
    OpenSSLError {
        source: openssl::error::ErrorStack,
    },
    InvalidOperation {
        detail: String,
    },
    #[snafu(transparent)]
    CipherError {
        source: CipherError,
    },
    #[snafu(transparent)]
    HandshakeError {
        source: HandshakeError,
    },
    #[snafu(transparent)]
    SessionError {
        source: SessionError,
    },
    #[snafu(transparent)]
    MessageError {
        source: crate::ssh::msg::Error,
    },
    #[snafu(transparent)]
    KeyError {
        source: crate::key::Error,
    },
    InvalidArgument {
        detail: String,
    }
}
