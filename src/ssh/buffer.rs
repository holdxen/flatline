use std::{
    cell::Cell,
    mem::size_of,
    ops::{Deref, Index, IndexMut},
};

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
}

macro_rules! make_buffer {
    ($($ty:ident: $value:expr $(,)?)+) => {
        {
            let len = $( match_type!($ty, $value) + )+ 0;
            let cap = len + 4;
            let mut buffer = Buffer::with_capacity(cap);
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
            let mut buffer = Buffer::with_capacity(len);
            $( put_type!(buffer, $ty, $value); )+
            buffer
        }
    };
}

// todo: Improve performance
#[derive(Default, Clone, Debug)]
#[repr(transparent)]
pub struct Buffer<T>(T);

impl From<Buffer<Vec<u8>>> for Vec<u8> {
    fn from(value: Buffer<Vec<u8>>) -> Self {
        value.into_vec()
    }
}

impl<T: AsRef<[u8]>> Deref for Buffer<T> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl<T: AsRef<[u8]>> AsRef<[u8]> for Buffer<T> {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}
impl<I, T: Index<I>> Index<I> for Buffer<T> {
    type Output = T::Output;

    #[inline]
    fn index(&self, index: I) -> &Self::Output {
        self.0.index(index)
    }
}

impl<I, T: IndexMut<I>> IndexMut<I> for Buffer<T> {
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        self.0.index_mut(index)
    }
}

impl<'a> Buffer<Cell<&'a [u8]>> {
    pub fn from_slice(slice: &[u8]) -> Buffer<Cell<&[u8]>> {
        Buffer(Cell::new(slice))
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.get().to_vec()
    }

    pub fn take_u8(&self) -> Option<u8> {
        if self.0.get().is_empty() {
            None
        } else {
            let ret = self.0.get()[0];
            self.0.set(&self.0.get()[1..]);
            Some(ret)
        }
    }

    pub fn take_u32(&self) -> Option<u32> {
        let tmp = self.0.get();
        let u32_len = size_of::<u32>();
        if tmp.len() < u32_len {
            return None;
        }
        let ret = u32::from_be_bytes(tmp[..u32_len].try_into().unwrap());
        self.0.set(&tmp[u32_len..]);
        Some(ret)
    }

    pub fn take_u64(&self) -> Option<u64> {
        let tmp = self.0.get();
        let u64_len = size_of::<u64>();
        if tmp.len() < u64_len {
            return None;
        }
        let ret = u64::from_be_bytes(tmp[..u64_len].try_into().unwrap());
        self.0.set(&tmp[u64_len..]);
        Some(ret)
    }

    pub fn take_bytes(&self, len: usize) -> Option<&'a [u8]> {
        if self.0.get().len() < len {
            None
        } else {
            let ret = &self.0.get()[..len];
            self.0.set(&self.0.get()[len..]);
            Some(ret)
        }
    }

    pub fn take_one(&self) -> Option<(u32, &'a [u8])> {
        let tmp = self.0.get();

        let len = match self.take_u32() {
            Some(len) => len,
            None => {
                self.0.set(tmp);
                return None;
            }
        } as usize;

        if self.len() < len {
            self.0.set(tmp);
            return None;
        }

        let ret = &self.0.get()[..len];

        self.0.set(&self.0.get()[len..]);

        Some((len as u32, ret))
    }

    pub fn len(&self) -> usize {
        self.0.get().len()
    }
}

impl Buffer<Vec<u8>> {
    pub fn from_one(content: impl AsRef<[u8]>) -> Self {
        let content = content.as_ref();
        let mut vec = Vec::with_capacity(content.len() + size_of::<u32>());

        vec.extend((content.len() as u32).to_be_bytes());
        vec.extend(content);
        Self(vec)
    }

    pub fn as_slice(&self) -> Buffer<Cell<&[u8]>> {
        Buffer::from_slice(&self.0)
    }

    pub fn new() -> Self {
        Default::default()
    }

    pub fn with_capacity(size: usize) -> Self {
        Self(Vec::with_capacity(size))
    }

    pub fn from_vec(vec: Vec<u8>) -> Self {
        Buffer(vec)
    }

    pub fn put_u64(&mut self, num: u64) {
        self.0.extend(num.to_be_bytes());
    }

    pub fn put_u32(&mut self, num: u32) {
        self.0.extend(num.to_be_bytes());
    }

    pub fn put_u8(&mut self, num: u8) {
        self.0.push(num);
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }

    pub fn take_u32(&mut self) -> Option<u32> {
        let u32_len = size_of::<u32>();
        if self.0.len() < u32_len {
            return None;
        }
        let num = u32::from_be_bytes(self.0[..u32_len].try_into().unwrap());

        self.0.drain(..u32_len);

        Some(num)
    }

    pub fn take_u8(&mut self) -> Option<u8> {
        if self.0.is_empty() {
            None
        } else {
            Some(self.0.remove(0))
        }
        // self.take_bytes(1).map(|v| v[0])
    }

    pub fn take_bytes(&mut self, size: usize) -> Option<Vec<u8>> {
        if self.0.len() < size {
            None
        } else {
            Some(self.0.drain(..size).collect())
        }
    }

    pub fn take_one(&mut self) -> Option<(u32, Vec<u8>)> {
        let u32_len = size_of::<u32>();
        if self.0.len() < u32_len {
            return None;
        }
        let size = u32::from_be_bytes(self.0[..u32_len].try_into().unwrap());

        if self.0.len() < size as usize + u32_len {
            return None;
        }

        self.0.drain(0..u32_len); // remove size 4 bytes

        Some((size, self.0.drain(0..size as usize).collect()))
    }

    pub fn put_bytes(&mut self, bytes: impl AsRef<[u8]>) {
        self.0.extend(bytes.as_ref());
    }

    pub fn put_one(&mut self, content: impl AsRef<[u8]>) {
        self.put_u32(content.as_ref().len() as u32);

        self.put_bytes(content);
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn extend<T: IntoIterator<Item = u8>>(&mut self, other: T) {
        self.0.extend(other);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}
