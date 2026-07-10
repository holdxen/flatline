pub type Result<T, E = Error> = std::result::Result<T, E>;

pub fn ok<T>(t: T) -> Result<T> {
    Ok(t)
}

use crate::cipher::Error as CipherError;
use crate::session::Error as SessionError;
use crate::session::HandshakeError;
use crate::ssh::Error as TransportError;

#[derive(Debug, snafu::Snafu)]
#[snafu(module(builder), context(suffix(false)), visibility(pub))]
pub enum Error {
    #[snafu(display("Invalid format: {}", detail))]
    InvalidFormat { detail: String },
    #[snafu(transparent)]
    TransportError { source: TransportError },
    #[snafu(display("IO error: {}", source))]
    IOError { source: std::io::Error },
    #[snafu(display("OpenSSL error: {}", source))]
    OpenSSLError { source: openssl::error::ErrorStack },
    #[snafu(display("Invalid operation: {}", detail))]
    InvalidOperation { detail: String },
    #[snafu(transparent)]
    CipherError { source: CipherError },
    #[snafu(transparent)]
    HandshakeError { source: HandshakeError },
    #[snafu(transparent)]
    SessionError { source: SessionError },
    #[snafu(transparent)]
    KeyError { source: crate::key::Error },
    #[snafu(display("Invalid argument: {}", detail))]
    InvalidArgument { detail: String },
}
