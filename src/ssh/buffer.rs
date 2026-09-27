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
    ($buffer:ident, one_list_u32, $i:expr) => {{
        $buffer.put_u32((4 + $i.len() * 4) as u32);
        for &item in $i {
            $buffer.put_u32(item);
        }
    }};
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
        self.len() == 0
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

    pub fn peek_u8(&self) -> error::Result<u8> {
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn producer_consumer_round_trip() {
        let mut producer = Producer::default();
        producer.put_u8(0x7f);
        producer.put_u32(0xdead_beef);
        producer.put_u64(0x0123_4567_89ab_cdef);
        producer.put_one(b"hello");
        producer.put_bytes([1, 2, 3]);

        assert_eq!(producer.len(), 1 + 4 + 8 + 4 + 5 + 3);

        let data = producer.into_vec();
        let mut consumer = Consumer::new(&data);

        assert_eq!(consumer.len(), data.len());
        assert_eq!(consumer.consume_u8().unwrap(), 0x7f);
        assert_eq!(consumer.consume_u32().unwrap(), 0xdead_beef);
        assert_eq!(consumer.consume_u64().unwrap(), 0x0123_4567_89ab_cdef);
        assert_eq!(consumer.consume_one().unwrap(), b"hello");
        assert_eq!(consumer.consume_bytes(3).unwrap(), [1, 2, 3]);
        assert!(consumer.is_empty());
        assert_eq!(consumer.len(), 0);
    }

    #[test]
    fn producer_put_one_writes_length_prefix() {
        let mut producer = Producer::default();
        producer.put_one(b"abc");

        // 4-byte big-endian length followed by the bytes.
        assert_eq!(producer.as_bytes(), &[0, 0, 0, 3, b'a', b'b', b'c']);
        assert_eq!(producer[0..4], [0, 0, 0, 3]);
    }

    #[test]
    fn producer_index_and_resize() {
        let mut producer = Producer::default();
        producer.put_u32(0);

        producer[0] = 0xff;
        assert_eq!(producer[0], 0xff);
        assert_eq!(producer[..4], [0xff, 0, 0, 0]);

        producer.resize(6, 0xaa);
        assert_eq!(producer.len(), 6);
        assert_eq!(producer[4..], [0xaa, 0xaa]);
    }

    #[test]
    fn producer_into_vec_preserves_bytes() {
        let mut producer = Producer::with_capacity(4);
        producer.put_u8(1);
        producer.put_u8(2);

        assert_eq!(producer.into_vec(), vec![1, 2]);
    }

    #[test]
    fn consumer_peek_does_not_advance() {
        let data = [1, 2, 3, 4];
        let mut consumer = Consumer::new(&data);

        assert_eq!(consumer.peek(), &data[..]);
        assert_eq!(consumer.peek_u8().unwrap(), 1);
        assert_eq!(consumer.peek(), &data[..]);
        assert_eq!(consumer.len(), 4);

        consumer.consume(2);
        assert_eq!(consumer.peek(), &[3, 4]);
        assert_eq!(consumer.consume_u8().unwrap(), 3);
    }

    #[test]
    fn consumer_consume_all_drains_buffer() {
        let data = [9, 8, 7];
        let mut consumer = Consumer::new(&data);

        consumer.consume_all();
        assert!(consumer.is_empty());
        assert_eq!(consumer.len(), 0);
        assert!(consumer.peek().is_empty());
    }

    #[test]
    fn consumer_truncated_u32_fails() {
        let mut consumer = Consumer::new(&[0, 0, 1]);
        let err = consumer.consume_u32().unwrap_err();
        assert!(
            err.to_string().contains("Unexpected end of buffer"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn consumer_truncated_u64_fails() {
        let mut consumer = Consumer::new(&[0; 7]);
        let err = consumer.consume_u64().unwrap_err();
        assert!(
            err.to_string().contains("Unexpected end of buffer"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn consumer_one_longer_than_remaining_fails() {
        // Length prefix says 100 bytes but only 2 follow.
        let data = [0, 0, 0, 100, 1, 2];
        let mut consumer = Consumer::new(&data);
        let err = consumer.consume_one().unwrap_err();
        assert!(
            err.to_string().contains("Unexpected end of buffer"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn consumer_bytes_past_end_fails() {
        let mut consumer = Consumer::new(&[1, 2]);
        let err = consumer.consume_bytes(3).unwrap_err();
        assert!(
            err.to_string().contains("Unexpected end of buffer"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn consumer_u8_on_empty_fails() {
        let mut consumer = Consumer::new(&[]);
        assert!(consumer.consume_u8().is_err());
        assert!(Consumer::new(&[]).peek_u8().is_err());
    }

    #[test]
    fn consumer_empty_is_empty_but_new_is_not() {
        let mut consumer = Consumer::new(&[1]);
        assert!(!consumer.is_empty());

        consumer.consume(1);
        assert!(consumer.is_empty());
    }

    #[test]
    fn make_buffer_prepends_length() {
        let buffer = make_buffer!(u8: 7u8, u32: 9u32);
        // 4-byte total length, then u8, then u32.
        assert_eq!(buffer.as_bytes(), &[0, 0, 0, 5, 7, 0, 0, 0, 9]);
    }

    #[test]
    fn make_buffer_without_header_has_no_length_prefix() {
        let buffer = make_buffer_without_header!(one: b"xy");
        assert_eq!(buffer.as_bytes(), &[0, 0, 0, 2, b'x', b'y']);
    }

    #[test]
    fn match_type_sizes() {
        assert_eq!(match_type!(u8), 1);
        assert_eq!(match_type!(u32), 4);
        assert_eq!(match_type!(u64), 8);
        assert_eq!(match_type!(one, b"abc"), 7);
        assert_eq!(match_type!(bytes, [1, 2]), 2);
    }
}
