use std::ops::{Index, IndexMut};

use crate::error::{self, builder};

// #[derive(snafu::Snafu, Debug)]
// pub enum Error {
//     #[snafu(display("Unexpected end of buffer: {detail}"))]
//     UnexpectedEndOfBuffer {
//         detail: String,
//     },
//     InvalidUtf8String {
//         source: Utf8Error,
//     },
// }

macro_rules! match_type {
    (u8 $(,$i:expr)?) => {
        1
    };
    (u32 $(,$i:expr)?) => {
        4
    };
    (u64 $(,$i:expr)?) => {
        8
    };
    (one, $i:expr) => {
        (4 + $i.len())
    };
    (bytes, $i:expr) => {
        $i.len()
    };
    (one_list_u32, $i:expr) => {
        (4 + $i.len() * 4)
    };
}

macro_rules! put_type {
    ($buffer:ident, u8, $i:expr) => {
        $buffer.put_u8($i);
    };
    ($buffer:ident, u32, $i:expr) => {
        $buffer.put_u32($i)
    };
    ($buffer:ident, u64, $i:expr) => {
        $buffer.put_u64($i)
    };
    ($buffer:ident, one, $i:expr) => {
        $buffer.put_one($i)
    };
    ($buffer:ident, bytes, $i:expr) => {
        $buffer.put_bytes($i)
    };
    ($buffer:ident, one_list_u32, $i:expr) => {
        {
            $buffer.put_u32((4 + $i.len() * 4) as u32);
            for &item in $i {
                $buffer.put_u32(item);
            }
        }
    };
}

macro_rules! make_buffer {
    ($($ty:ident: $value:expr $(,)?)+) => {
        {
            let len = $( match_type!($ty, $value) + )+ 0;
            let cap = len + 4;
            let mut buffer = Producer::with_capacity(cap);
            buffer.put_u32(len as u32);
            $( put_type!(buffer, $ty, $value); )+
            buffer
        }
    };
}

macro_rules! make_buffer_without_header {
    ($($ty:ident: $value:expr $(,)?)+) => {
        {
            let len = $( match_type!($ty, $value) + )+ 0;
            let mut buffer = Producer::with_capacity(len);
            $( put_type!(buffer, $ty, $value); )+
            buffer
        }
    };
}

pub(crate) use make_buffer_without_header;
pub(crate) use match_type;
pub(crate) use put_type;

pub struct Producer {
    data: Vec<u8>,
}

impl Index<usize> for Producer {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        &self.data[index]
    }
}

impl IndexMut<usize> for Producer {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.data[index]
    }
}

use std::ops::{Range, RangeFrom, RangeFull, RangeInclusive, RangeTo};

impl Index<Range<usize>> for Producer {
    type Output = [u8];

    fn index(&self, index: Range<usize>) -> &Self::Output {
        &self.data[index]
    }
}

impl Index<RangeFrom<usize>> for Producer {
    type Output = [u8];

    fn index(&self, index: RangeFrom<usize>) -> &Self::Output {
        &self.data[index]
    }
}

impl Index<RangeTo<usize>> for Producer {
    type Output = [u8];

    fn index(&self, index: RangeTo<usize>) -> &Self::Output {
        &self.data[index]
    }
}

impl Index<RangeFull> for Producer {
    type Output = [u8];

    fn index(&self, index: RangeFull) -> &Self::Output {
        &self.data[index]
    }
}

impl Index<RangeInclusive<usize>> for Producer {
    type Output = [u8];

    fn index(&self, index: RangeInclusive<usize>) -> &Self::Output {
        &self.data[index]
    }
}

impl IndexMut<Range<usize>> for Producer {
    fn index_mut(&mut self, index: Range<usize>) -> &mut Self::Output {
        &mut self.data[index]
    }
}

impl IndexMut<RangeFrom<usize>> for Producer {
    fn index_mut(&mut self, index: RangeFrom<usize>) -> &mut Self::Output {
        &mut self.data[index]
    }
}

impl IndexMut<RangeTo<usize>> for Producer {
    fn index_mut(&mut self, index: RangeTo<usize>) -> &mut Self::Output {
        &mut self.data[index]
    }
}

impl IndexMut<RangeFull> for Producer {
    fn index_mut(&mut self, index: RangeFull) -> &mut Self::Output {
        &mut self.data[index]
    }
}

impl IndexMut<RangeInclusive<usize>> for Producer {
    fn index_mut(&mut self, index: RangeInclusive<usize>) -> &mut Self::Output {
        &mut self.data[index]
    }
}

impl Producer {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn put_bytes(&mut self, bytes: impl AsRef<[u8]>) {
        self.data.extend(bytes.as_ref());
    }

    pub fn put_u64(&mut self, num: u64) {
        self.data.extend(num.to_be_bytes());
    }

    pub fn put_u32(&mut self, num: u32) {
        self.data.extend(num.to_be_bytes());
    }

    pub fn put_u8(&mut self, num: u8) {
        self.data.push(num);
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }

    pub fn put_one(&mut self, content: impl AsRef<[u8]>) {
        self.put_u32(content.as_ref().len() as u32);

        self.put_bytes(content);
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn resize(&mut self, new_len: usize, value: u8) {
        self.data.resize(new_len, value);
    }
}

impl Default for Producer {
    fn default() -> Self {
        Self::with_capacity(1024)
    }
}

pub struct Consumer<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Consumer<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.data.len() - self.pos
    }
    
    pub fn is_empty(&self) -> bool {
        assert!(self.pos <= self.data.len());
        self.pos == self.data.len()
    }

    pub fn peek(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    pub fn consume_all(&mut self) {
        self.pos = self.data.len();
    }

    pub fn consume(&mut self, size: usize) {
        self.pos += size;
        assert!(self.pos <= self.data.len());
    }

    pub fn consume_u32(&mut self) -> error::Result<u32> {
        let u32_len = size_of::<u32>();
        if self.peek().len() < u32_len {
            return builder::InvalidFormat {
                detail: "Unexpected end of buffer while reading u32",
            }
            .fail();
        }
        let num = u32::from_be_bytes(self.peek()[..u32_len].try_into().unwrap());

        self.consume(u32_len);

        Ok(num)
    }

    pub fn consume_u64(&mut self) -> error::Result<u64> {
        let tmp = self.peek();
        let u64_len = size_of::<u64>();
        if tmp.len() < u64_len {
            return builder::InvalidFormat {
                detail: "Unexpected end of buffer while reading u64",
            }
            .fail();
        }
        let ret = u64::from_be_bytes(tmp[..u64_len].try_into().unwrap());
        self.consume(u64_len);

        Ok(ret)
    }

    pub fn consume_one(&mut self) -> error::Result<&'a [u8]> {
        let len = self.consume_u32()?;

        if len as usize > self.peek().len() {
            return builder::InvalidFormat {
                detail: "Unexpected end of buffer while reading one",
            }
            .fail();
        }

        self.pos += len as usize;

        Ok(&self.data[self.pos - len as usize..self.pos])
    }

    pub fn consume_bytes(&mut self, len: usize) -> error::Result<&'a [u8]> {
        if self.peek().len() < len {
            return builder::InvalidFormat {
                detail: "Unexpected end of buffer while reading bytes",
            }
            .fail();
        }
        let ret = &self.peek()[..len];
        self.consume(len);
        Ok(ret)
    }

    pub fn peek_u8(&mut self) -> error::Result<u8> {
        if self.peek().is_empty() {
            return builder::InvalidFormat {
                detail: "Unexpected end of buffer while reading u8",
            }
            .fail();
        }

        let ret = self.peek()[0];
        Ok(ret)
    }
    pub fn consume_u8(&mut self) -> error::Result<u8> {
        if self.peek().is_empty() {
            return builder::InvalidFormat {
                detail: "Unexpected end of buffer while reading u8",
            }
            .fail();
        }

        let ret = self.peek()[0];
        self.consume(1);
        Ok(ret)
    }
}
