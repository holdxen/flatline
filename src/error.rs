use std::{io, str::Utf8Error};

use super::channel::ChannelOpenFailureReson;
use openssl::error::ErrorStack;
use tokio::sync::oneshot::error::RecvError;

pub type Result<T> = std::result::Result<T, Error>;
#[cfg(feature = "backtrace")]
use snafu::Backtrace;
use snafu::{IntoError, Snafu};

#[derive(Snafu, Debug)]
#[snafu(module(builder), context(suffix(false)), visibility(pub))]
pub enum Error {
    #[snafu(context(false))]
    #[snafu(display("Openssl error"))]
    OpensslError {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        source: ErrorStack,
    },

    #[snafu(display("Standard io error: {source}"))]
    IOError {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        source: io::Error,
    },

    #[snafu(display("UndefinedBehavior: {tip}"))]
    UndefinedBehavior {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("Failed to Decode binary data as utf8"))]
    Utf8Error {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        source: Utf8Error,
    },

    #[snafu(display("The peer does not support ssh2"))]
    Ssh2Unsupport {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
    },

    #[snafu(display("Protocol error: {tip}"))]
    ProtocolError {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("Algorithm negotiation failed"))]
    NegotiationFailed {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
    },

    #[snafu(display("Banner exchange failed: {tip}"))]
    BannerTooLong {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("Server connection lost"))]
    Disconnected {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
    },

    #[snafu(display("Server Message mac verification failed"))]
    MacVerificationFailed {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
    },

    #[snafu(display("Channel open failed"))]
    ChannelOpenFail {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        reson: ChannelOpenFailureReson,
        desc: String,
    },

    #[snafu(display("SSH Channel Failure"))]
    ChannelFailure {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
    },

    // #[snafu(display("internal error: failed to find channel")]
    // ChannelNotFound,
    #[snafu(display("Error code: {code:?} {tip}"))]
    ScpError {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        code: Option<u8>,
        tip: String,
    },

    #[snafu(display("Channel was closed"))]
    ChannelClosed {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
    },

    #[snafu(display("Channel end of file"))]
    ChannelEof {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
    },

    #[snafu(display("Failed to verify hostkey"))]
    HostKeyVerifyFailed {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
    },

    #[snafu(display("Failed to request subsystem from server"))]
    SubsystemFailed {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
    },

    #[snafu(display("Resource is temporarily unavailable"))]
    TemporarilyUnavailable {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
    },

    #[snafu(display("Uncompress or Compress Error"))]
    CompressFailed {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
    },

    #[snafu(display("Failed to parse binary: {tip}"))]
    InvalidFormat {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display(
        "The packet with sequence number {sequence_number} was rejected by the server"
    ))]
    Unimplemented {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        sequence_number: u32,
    },

    #[snafu(display("User reject: {tip}"))]
    RejectByUser {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("Invalid Argument: {tip}"))]
    InvalidArgument {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("SFtp: {tip}"))]
    NoSuchFile {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("SFtp: {tip}"))]
    PermissionDenied {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("SFtp: {tip}"))]
    SFtpFailure {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("SFtp: {tip}"))]
    BadMessage {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("SFtp: {tip}"))]
    NoConnection {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("SFtp: {tip}"))]
    ConnectionLost {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("SFtp: {tip}"))]
    OpUnsupported {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("Failed to Request: {tip}"))]
    RequestFailure {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        tip: String,
    },

    #[snafu(display("Calling recv on a channel with an None receiver"))]
    ChannelReceiverIsNone {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
    },

    #[snafu(whatever, display("{message}:{source:?}"))]
    Whatever {
        message: String,
        #[snafu(source(from(Box<dyn std::error::Error + Send + Sync + 'static>, Some)))]
        source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
    },

    #[snafu(display("Bad operation: {detail}"))]
    BadOperation {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace,
        detail: String,
    },
}

// struct Unexpect {
//     expect: Vec<String>,
//     found: String
// }

impl Error {
    pub fn ub(tip: impl Into<String>) -> Self {
        builder::UndefinedBehavior { tip }.build()
    }

    // pub fn ssh_packet_parse(tip: impl Into<String>) -> Self {
    //     Self::SshPacketParseError(tip.into())
    // }

    pub fn scp_error(code: Option<u8>, tip: impl Into<String>) -> Self {
        builder::Scp { code, tip }.build()
    }

    pub fn invalid_format(tip: impl Into<String>) -> Self {
        builder::InvalidFormat { tip }.build()
    }
}

impl From<RecvError> for Error {
    fn from(_: RecvError) -> Self {
        builder::Disconnected.build()
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        builder::IO.into_error(value)
    }
}

impl From<Utf8Error> for Error {
    fn from(value: Utf8Error) -> Self {
        builder::Utf8.into_error(value)
    }
}
