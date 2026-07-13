//! Support code for the SSH transport layer: the SSH-2.0 binary packet protocol
//! (RFC 4253, section 6).
//!
//! The public parts of this module are the [`Error`] enum, reported while
//! framing, encrypting, decrypting and verifying binary packets, and the
//! [`msg`] submodule, which holds SSH protocol constants and message newtypes.
//! The remaining submodules are crate-internal: `buffer` (wire-format
//! producers/consumers and macros for building messages), `protocol` (protocol
//! constants such as message numbers, reason codes and packet-size limits) and
//! `stream` (the framed transport stream that sends payloads and receives
//! packets). This module also contains a crate-internal helper trait for
//! encoding integers in the SSH `mpint` format.

use std::str::Utf8Error;

#[macro_use]
pub(crate) mod buffer;
pub mod msg;
pub(crate) mod protocol;
pub(crate) mod stream;

/// An error raised while encoding or decoding SSH binary packets.
#[derive(Debug, snafu::Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    /// The payload exceeds the maximum payload length: `maximum` bytes are allowed, `actual` were seen.
    #[snafu(display("payload is too long"))]
    PayloadTooLong {
        /// The maximum payload length allowed, in bytes.
        maximum: usize,
        /// The payload length actually seen, in bytes.
        actual: usize,
    },
    /// The packet exceeds the maximum packet length: `maximum` bytes are allowed, `actual` were seen.
    #[snafu(display("packet is too long"))]
    PacketTooLong {
        /// The maximum packet length allowed, in bytes.
        maximum: usize,
        /// The packet length actually seen, in bytes.
        actual: usize,
    },
    // #[snafu(display("padding length is incorrect"))]
    // PaddingLengthIncorrect,
    /// A received packet carried an empty payload.
    #[snafu(display("Payload is empty"))]
    PayloadIsEmpty,
    /// A block size other than the expected cipher block size was observed, in bytes.
    #[snafu(display("Unexpected block size: {}", size))]
    UnexpectBlockSize {
        /// The observed block size, in bytes.
        size: usize,
    },
    /// The message authentication code of a received packet failed to verify.
    #[snafu(display("MAC verification failed"))]
    MacVerificationFailed,
    /// The declared padding length of a packet is inconsistent with the packet length.
    #[snafu(display("Unexpected padding length"))]
    UnexpectedPaddingLength,

    /// A protocol string field was not valid UTF-8; `source` carries the UTF-8 error.
    #[snafu(display("Expected string: {}", source))]
    ExpectString {
        /// The UTF-8 decoding error.
        source: Utf8Error,
    },
}

pub(super) trait MultiplePrecisionInteger {
    fn to_integer(&self) -> Vec<u8>;
    fn into_integer(self) -> Vec<u8>;
}

impl MultiplePrecisionInteger for Vec<u8> {
    fn to_integer(&self) -> Vec<u8> {
        self.to_vec().into_integer()
    }

    fn into_integer(mut self) -> Vec<u8> {
        while !self.is_empty() && self[0] == 0 {
            self.remove(0);
        }

        if self.is_empty() {
            return vec![0; 4];
        }

        if self[0] & 0x80 != 0 {
            self.insert(0, 0);
            self
        } else {
            self
        }
    }
}

impl MultiplePrecisionInteger for openssl::bn::BigNum {
    fn to_integer(&self) -> Vec<u8> {
        self.to_vec().into_integer()
    }

    fn into_integer(self) -> Vec<u8> {
        self.to_vec().into_integer()
    }
}

impl MultiplePrecisionInteger for openssl::bn::BigNumRef {
    fn to_integer(&self) -> Vec<u8> {
        self.to_vec().into_integer()
    }

    fn into_integer(self) -> Vec<u8> {
        self.to_vec().into_integer()
    }
}
