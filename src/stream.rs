use std::fmt::Debug;
use std::io;

use bytes::{Buf, BufMut, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub struct BufferStream<T> {
    socket: T,
    r_buf: BytesMut,
    w_buf: BytesMut,
}

impl<T> Debug for BufferStream<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "BufferStream {{ socket, r_buf_len: {}, w_buf_len: {} }}",
            self.r_buf.len(),
            self.w_buf.len()
        )
    }
}

// impl<T: AsyncRead + Unpin> AsyncRead for BufferStream<T> {
//     fn poll_read(
//         mut self: Pin<&mut Self>,
//         cx: &mut Context<'_>,
//         buf: &mut ReadBuf<'_>,
//     ) -> Poll<io::Result<()>> {
//         if !self.r_buf.is_empty() {
//             let len = min(buf.remaining(), self.r_buf.len());
//             buf.put(self.r_buf.split_to(len));
//             return Poll::Ready(Ok(()));
//         }
//         Pin::new(&mut self.socket).poll_read(cx, buf)
//     }
// }

impl<T> BufferStream<T> {
    pub fn new(socket: T) -> Self {
        Self {
            socket,
            r_buf: BytesMut::with_capacity(1024 * 40),
            w_buf: BytesMut::with_capacity(1024 * 40),
        }
    }

    // pub fn inner_mut(&mut self) -> &mut T {
    //     &mut self.socket
    // }

    // pub fn into_inner(self) -> T {
    //     self.socket
    // }

    // pub fn rbuffer(&self) -> &[u8] {
    //     &self.r_buf
    // }

    pub fn consume_read_buffer(&mut self, size: usize) {
        self.r_buf.advance(size);
    }
    // pub fn inner_mut(&mut self) -> &mut T {
    //     &mut self.socket
    // }

    // pub fn take_read_bytes(&mut self) -> Vec<u8> {
    //     std::mem::take(&mut self.r_buf).to_vec()
    // }

    // pub fn take_write_bytes(&mut self) -> Vec<u8> {
    //     std::mem::take(&mut self.w_buf).to_vec()
    // }
    // pub fn inner(&self) -> &T {
    //     &self.socket
    // }
}

impl<T> BufferStream<T> {
    pub fn put_bytes(&mut self, data: impl AsRef<[u8]>) {
        self.w_buf.put(data.as_ref());
    }
}

impl<T> BufferStream<T>
where
    T: AsyncWrite + Unpin,
{
    pub async fn write(&mut self, data: impl AsRef<[u8]>) -> io::Result<bool> {
        self.w_buf.put(data.as_ref());
        let len = self.w_buf.len();
        self.socket
            .write_buf(&mut self.w_buf)
            .await
            .map(|write| write == len)
    }

    pub async fn flush(&mut self) -> io::Result<()> {
        while !self.w_buf.is_empty() {
            self.socket.write_buf(&mut self.w_buf).await?;
        }
        self.socket.flush().await
    }

    // pub async fn write_all(&mut self, data: impl AsRef<[u8]>) -> io::Result<()> {
    //     self.w_buf.put(data.as_ref());
    //     while !self.w_buf.is_empty() {
    //         self.socket.write_buf(&mut self.w_buf).await?;
    //     }
    //     self.socket.flush().await
    // }
}

impl<T: AsyncRead + Unpin> BufferStream<T> {
    async fn internal_read(&mut self) -> io::Result<usize> {
        let size = self.socket.read_buf(&mut self.r_buf).await?;
        if size == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Connection closed",
            ));
        }
        Ok(size)
    }

    // pub async fn read_line_lf(&mut self) -> io::Result<Vec<u8>> {
    //     loop {
    //         // todo: improve performance
    //         let pos = self.r_buf.iter().position(|&x| x == b'\n');
    //         if let Some(pos) = pos {
    //             return Ok(self.r_buf.split_to(pos + 1).to_vec());
    //         }
    //         self.internal_read().await?;
    //     }
    // }

    pub async fn read_line_crlf(&mut self, max: usize) -> io::Result<Vec<u8>> {
        if max < 2 {
            return Err(io::Error::other("line ending with crlf size must >= 2"));
        }
        loop {
            // todo: improve performance
            for i in 0..self.r_buf.len() {
                if i == max - 1 {
                    return Err(io::Error::other("Read line error"));
                }
                if self.r_buf[i] == b'\r' && i < self.r_buf.len() - 1 && self.r_buf[i + 1] == b'\n'
                {
                    return Ok(self.r_buf.split_to(i + 2).to_vec());
                }
            }

            self.internal_read().await?;
        }
    }

    // pub async fn read_buf_at_least(&mut self) -> io::Result<Vec<u8>> {
    //     self.socket.read_buf(&mut self.r_buf).await?;

    //     Ok(take(&mut self.r_buf).to_vec())
    // }

    // pub async fn read_buf(&mut self) -> io::Result<Vec<u8>> {
    //     if self.r_buf.is_empty() {
    //         self.socket.read_buf(&mut self.r_buf).await?;
    //     }
    //     let ret = self.r_buf.to_vec();
    //     self.r_buf.clear();
    //     Ok(ret)
    // }

    // pub async fn read_exact(&mut self, size: usize) -> io::Result<Vec<u8>> {
    //     while self.r_buf.len() < size {
    //         self.internal_read().await?;
    //     }

    //     Ok(self.r_buf.split_to(size).to_vec())
    // }

    pub async fn fill(&mut self, len: usize) -> io::Result<&[u8]> {
        while self.r_buf.len() < len {
            self.internal_read().await?;
        }

        Ok(&self.r_buf[..len])
    }
}
