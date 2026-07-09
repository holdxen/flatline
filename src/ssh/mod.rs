#[macro_use]
pub(crate) mod buffer;
pub mod msg;
pub(crate) mod protocol;
pub(crate) mod stream;

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
