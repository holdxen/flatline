use std::str::Utf8Error;

#[macro_use]
pub(crate) mod buffer;
pub mod msg;
pub(crate) mod protocol;
pub(crate) mod stream;

#[derive(Debug, snafu::Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    #[snafu(display("payload is too long"))]
    PayloadTooLong { maximum: usize, actual: usize },
    #[snafu(display("packet is too long"))]
    PacketTooLong { maximum: usize, actual: usize },
    // #[snafu(display("padding length is incorrect"))]
    // PaddingLengthIncorrect,
    #[snafu(display("Payload is empty"))]
    PayloadIsEmpty,
    #[snafu(display("Unexpected block size: {}", size))]
    UnexpectBlockSize { size: usize },
    #[snafu(display("MAC verification failed"))]
    MacVerificationFailed,
    #[snafu(display("Unexpected padding length"))]
    UnexpectedPaddingLength,

    #[snafu(display("Expected string: {}", source))]
    ExpectString { source: Utf8Error },
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
