use std::cell::Cell;
use std::cmp::min;
use std::collections::HashMap;
use std::fmt::Debug;
use std::io;
use std::mem::transmute;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use derive_new::new;
use num_enum::TryFromPrimitive;
use snafu::OptionExt;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::channel::{BufferChannel, Channel};
use crate::error::{builder, Result};
use crate::msg::Request;
use crate::ssh::common::*;
use crate::BoxFuture;
use crate::{
    error::Error,
    ssh::{buffer::Buffer, common::code::*},
};

use super::o_channel;
use bitflags::bitflags;

bitflags! {
    // https://datatracker.ietf.org/doc/html/draft-ietf-secsh-filexfer-01#section-7.3
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct OpenFlags: u32 {
        // Open the file for reading
        const READ                        = SSH_FXF_READ;
        // Open the file for writing.  If both this and SSH_FXF_READ are specified,
        // the file is opened for both reading and writing.
        const WRITE                       = SSH_FXF_WRITE;
        // Force all writes to append data at the end of the file.
        const APPEND                      = SSH_FXF_APPEND;
        // If this flag is specified, then a new file will be created if one
        // does not alread exist (if O_TRUNC is specified, the new file will
        // be truncated to zero length if it previously exists)
        const CREAT                       = SSH_FXF_CREAT;
        // Forces an existing file with the same name to be truncated to zero
        // length when creating a file by specifying SSH_FXF_CREAT.
        // SSH_FXF_CREAT MUST also be specified if this flag is used.
        const TRUNC                       = SSH_FXF_TRUNC;
        // Causes the request to fail if the named file already exists.
        // SSH_FXF_CREAT MUST also be specified if this flag is used.
        const EXCL                        = SSH_FXF_EXCL;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Statvfs {
    pub bsize: u64,
    pub frsize: u64,
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub favail: u64,
    pub fsid: u64,
    pub flag: u64,
    pub namemax: u64,
}

impl Statvfs {
    pub const FLAG_RDONLY: u64 = 0x1;
    pub const FLAG_NOSUID: u64 = 0x2;
    fn parse(data: &[u8]) -> Option<Self> {
        let buffer = Buffer::from_slice(data);

        Some(Self {
            bsize: buffer.take_u64()?,
            frsize: buffer.take_u64()?,
            blocks: buffer.take_u64()?,
            bfree: buffer.take_u64()?,
            bavail: buffer.take_u64()?,
            files: buffer.take_u64()?,
            ffree: buffer.take_u64()?,
            favail: buffer.take_u64()?,
            fsid: buffer.take_u64()?,
            flag: buffer.take_u64()?,
            namemax: buffer.take_u64()?,
        })
    }
}

pub struct Stream<'a> {
    sftp: &'a mut SFtp,
    file: &'a mut File,
    read_future: Option<BoxFuture<'a, Result<Vec<u8>>>>,
    write_future: Option<BoxFuture<'a, Result<()>>>,
}

impl<'a> Stream<'a> {
    fn poll_read_no_pin(
        &'a mut self,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self;
        if this.read_future.is_none() {
            let read: BoxFuture<'_, _> = Box::pin(this.sftp.read_file(
                this.file,
                if buf.remaining() > u32::MAX as usize {
                    u32::MAX
                } else {
                    buf.remaining() as u32
                },
            ));
            this.read_future = Some(read);
        }

        let f = this.read_future.as_mut().unwrap().as_mut();

        let res = ready!(f.poll(cx));
        this.read_future = None;
        match res {
            Ok(data) => {
                if data.len() > buf.remaining() {
                    return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, "data too long")));
                }
                buf.put_slice(&data);
                Poll::Ready(Ok(()))
            }
            Err(err) => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, Box::new(err)))),
        }
    }

    fn poll_write_no_pin(
        &'a mut self,
        cx: &mut Context<'_>,
        buf: &'a [u8],
    ) -> Poll<io::Result<usize>> {
        if self.write_future.is_none() {
            let future: BoxFuture<_> = Box::pin(self.sftp.write_file_buf(self.file, buf));

            self.write_future = Some(future);
        }
        let res = ready!(self.write_future.as_mut().unwrap().as_mut().poll(cx));

        self.write_future = None;

        match res {
            Ok(_) => Poll::Ready(Ok(buf.len())),
            Err(err) => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, Box::new(err)))),
        }
    }
}

impl<'a> AsyncWrite for Stream<'a> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this: &mut Stream<'a> = unsafe { transmute(Pin::into_inner(self)) };

        let buf = unsafe { transmute::<&[u8], &[u8]>(buf) };

        this.poll_write_no_pin(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl<'a> AsyncRead for Stream<'a> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this: &mut Stream<'a> = unsafe { transmute(Pin::into_inner(self)) };
        this.poll_read_no_pin(cx, buf)
        // let this = Pin::into_inner(self);
        // let this = &mut *this;
        // if this.read_future.is_none() {
        //     let read: BoxFuture<'_, _> = Box::pin(this.sftp.read_file(
        //         &mut this.file,
        //         if buf.remaining() > u32::MAX as usize {
        //             u32::MAX
        //         } else {
        //             buf.remaining() as u32
        //         },
        //     ));
        //     // this.read_future = unsafe { transmute(Some(read)) };
        //     this.read_future = Some(read);
        // }

        // let f = this.read_future.as_mut().unwrap().as_mut();

        // let res = ready!(f.poll(cx));
        // this.read_future = None;
        // match res {
        //     Ok(data) => {
        //         if data.len() > buf.remaining() {
        //             return Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, "data too long")));
        //         }
        //         buf.put_slice(&data);
        //         Poll::Ready(Ok(()))
        //     }
        //     Err(err) => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, Box::new(err)))),
        // }
    }
}

pub struct SFtp {
    channel: BufferChannel,
    request_id: u32,
    version: u32,
    ext: HashMap<String, Vec<u8>>,
    packets: HashMap<u32, Packet>,
}

impl Debug for SFtp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SFtp")
            .field("request_id", &self.request_id)
            .field("version", &self.version)
            .field("ext", &self.ext)
            .finish()
    }
}

impl SFtp {
    pub(crate) fn new(channel: Channel, version: u32, ext: HashMap<String, Vec<u8>>) -> Self {
        Self {
            channel: BufferChannel::new(channel),
            request_id: 0,
            version,
            ext,
            packets: Default::default(),
        }
    }
}

pub struct File {
    handle: Vec<u8>,
    pos: u64,
}

impl File {
    fn new(handle: Vec<u8>) -> Self {
        Self { handle, pos: 0 }
    }
}

pub struct Dir {
    handle: Vec<u8>,
}

impl Dir {
    fn new(handle: Vec<u8>) -> Self {
        Self { handle }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Permissions: u32 {
        const OTHER_EXEC                        = 1 << 0;
        const OTHER_WRITE                       = 1 << 1;
        const OTHER_READ                        = 1 << 2;

        const GROUP_EXEC                        = 1 << 0 << 4;
        const GROUP_WRITE                       = 1 << 1 << 4;
        const GROUP_READ                        = 1 << 2 << 4;

        const OWNER_EXEC                        = 1 << 0 << 8;
        const OWNER_WRITE                       = 1 << 1 << 8;
        const OWNER_READ                        = 1 << 2 << 8;
    }
}

impl Permissions {
    const MASK: u32 = !FileType::MASK;
    pub fn p0755() -> Self {
        Self::from_bits_retain(0o755)
    }
}

#[derive(new, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub atime: u32,
    pub mtime: u32,
}

#[derive(new, Debug, Clone, Copy, PartialEq, Eq)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[num_enum(error_type(name = Error, constructor = Self::invalid_file_type))]
#[repr(u32)]
pub enum FileType {
    Directory = 0o40000,
    CharacterDevice = 0o20000,
    BlockDevice = 0o60000,
    RegularFile = 0o100000,
    FIFO = 0o10000,
    SymbolicLink = 0o120000,
    Socket = 0o140000,
}

impl FileType {
    const MASK: u32 = 0o170000;
    fn invalid_file_type(value: u32) -> Error {
        builder::Protocol {
            tip: format!("Invalid SFtp file type({})", value),
        }
        .build()
    }
}

impl FileType {
    pub fn is_directory(&self) -> bool {
        matches!(self, Self::Directory)
    }

    pub fn is_character_device(&self) -> bool {
        matches!(self, Self::CharacterDevice)
    }

    pub fn is_block_device(&self) -> bool {
        matches!(self, Self::BlockDevice)
    }

    pub fn is_regular_file(&self) -> bool {
        matches!(self, Self::RegularFile)
    }

    pub fn is_fifo(&self) -> bool {
        matches!(self, Self::FIFO)
    }

    pub fn is_symbolic_link(&self) -> bool {
        matches!(self, Self::SymbolicLink)
    }

    pub fn is_socket(&self) -> bool {
        matches!(self, Self::Socket)
    }
}

#[derive(new, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Property {
    pub permissions: Permissions,
    pub file_type: FileType,
}

impl TryFrom<u32> for Property {
    type Error = Error;

    fn try_from(value: u32) -> std::result::Result<Self, Self::Error> {
        Ok(Self::new(
            Permissions::from_bits_truncate(value & Permissions::MASK),
            (value & FileType::MASK).try_into()?,
        ))
    }
}

impl Property {
    pub fn bits(&self) -> u32 {
        self.permissions.bits() | self.file_type as u32
    }
}

#[derive(new, Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub size: Option<u64>,
    pub user: Option<User>,
    pub property: Option<Property>,
    // atime mtime
    pub time: Option<Timestamp>,
    // extended_count: Option<u32>,
    pub extend: HashMap<String, Vec<u8>>,
}

impl Attributes {
    fn to_buffer(&self) -> Buffer<Vec<u8>> {
        let mut buffer = Buffer::new();
        self.to_bytes(&mut buffer);
        buffer
    }

    fn to_bytes(&self, buffer: &mut Buffer<Vec<u8>>) {
        let mut flags = 0;
        let mut tmp = Buffer::new();
        if let Some(size) = self.size {
            flags |= SSH_FILEXFER_ATTR_SIZE;
            tmp.put_u64(size);
        }

        if let Some(user) = self.user {
            flags |= SSH_FILEXFER_ATTR_UIDGID;
            tmp.put_u32(user.uid);
            tmp.put_u32(user.gid);
        }

        if let Some(permissions) = self.property {
            flags |= SSH_FILEXFER_ATTR_PERMISSIONS;
            tmp.put_u32(permissions.bits());
        }

        if let Some(time) = self.time {
            flags |= SSH_FILEXFER_ATTR_ACMODTIME;
            tmp.put_u32(time.atime);
            tmp.put_u32(time.mtime);
        }

        let count = self.extend.len() as u32;

        tmp.put_u32(count);

        for (k, v) in &self.extend {
            tmp.put_one(k);
            tmp.put_one(v);
        }
        buffer.put_u32(flags);
        buffer.put_bytes(tmp);
    }

    fn parse(buffer: &mut Buffer<Cell<&[u8]>>) -> Option<Self> {
        let flags = buffer.take_u32()?;

        let mut size = None;
        let mut user = None;
        let mut permissions = None;
        let mut time = None;

        let mut extend = HashMap::new();

        if flags & SSH_FILEXFER_ATTR_SIZE != 0 {
            size = Some(buffer.take_u64()?)
        }

        if flags & SSH_FILEXFER_ATTR_UIDGID != 0 {
            let uid = buffer.take_u32()?;
            let gid = buffer.take_u32()?;
            user = Some(User::new(uid, gid))
        }

        if flags & SSH_FILEXFER_ATTR_PERMISSIONS != 0 {
            let per = buffer.take_u32()?;
            permissions = Property::try_from(per).ok();
        }

        if flags & SSH_FILEXFER_ATTR_ACMODTIME != 0 {
            let atime = buffer.take_u32()?;
            let mtime = buffer.take_u32()?;

            time = Some(Timestamp::new(atime, mtime))
        }

        if flags & SSH_FILEXFER_ATTR_EXTENDED != 0 {
            let ecount = buffer.take_u32()?;

            for _ in 0..ecount {
                let (_, key) = buffer.take_one()?;
                let (_, value) = buffer.take_one()?;

                extend.insert(std::str::from_utf8(key).ok()?.to_string(), value.to_vec());
            }
        }

        Some(Self::new(size, user, permissions, time, extend))
    }
}

#[derive(new, Debug, Clone)]
pub struct FileInfo {
    pub filename: String,
    pub longname: String,
    pub attrs: Attributes,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_packet_len: u64,
    pub max_read_len: u64,
    pub max_write_len: u64,
    pub max_open_handles: u64,
}

impl Limits {
    fn parse(data: &[u8]) -> Option<Self> {
        let buffer = Buffer::from_slice(data);

        Some(Self {
            max_packet_len: buffer.take_u64()?,
            max_read_len: buffer.take_u64()?,
            max_write_len: buffer.take_u64()?,
            max_open_handles: buffer.take_u64()?,
        })
    }
}

#[derive(custom_debug_derive::Debug)]
struct Packet {
    id: u32,
    msg: Message,
}

impl Packet {
    fn parse(data: &[u8]) -> Option<Packet> {
        let data = Buffer::from_slice(data);
        let (_, data) = data.take_one()?;

        let mut data = Buffer::from_slice(data);

        let code = data.take_u8()?;
        let id = data.take_u32()?;

        let msg = match code {
            SSH_FXP_HANDLE => {
                let (_, handle) = data.take_one()?;

                Message::FileHandle(handle.to_vec())
            }
            SSH_FXP_STATUS => {
                let status = data.take_u32()?;

                let (_, msg) = data.take_one()?;

                let (_, tag) = data.take_one()?;

                let msg = std::str::from_utf8(msg).ok()?.to_string();

                let _tag = std::str::from_utf8(tag).ok()?.to_string();

                let status = Status::from_status(status).ok()?;
                Message::Status { status, msg, _tag }
            }
            SSH_FXP_DATA => Message::Data(data.take_one()?.1.to_vec()),
            SSH_FXP_NAME => {
                let count = data.take_u32()?;
                let mut res = Vec::with_capacity(count as usize);

                for _ in 0..count {
                    let (_, filename) = data.take_one()?;
                    let (_, longname) = data.take_one()?;

                    let filename = std::str::from_utf8(filename).ok()?.to_string();

                    let longname = std::str::from_utf8(longname).ok()?.to_string();

                    res.push(FileInfo::new(
                        filename,
                        longname,
                        Attributes::parse(&mut data)?,
                    ));
                }
                Message::Name(res)
            }
            SSH_FXP_ATTRS => Message::Attributes(Attributes::parse(&mut data)?),
            SSH_FXP_EXTENDED_REPLY => Message::ExtendReply(data.to_vec()),
            _ => return None,
        };

        Some(Packet { id, msg })
    }
}

#[derive(Debug, PartialEq)]
#[repr(u32)]
enum Status {
    OK = SSH_FX_OK,
    Eof = SSH_FX_EOF,
    NoSuchFile = SSH_FX_NO_SUCH_FILE,
    PermissionDenied = SSH_FX_PERMISSION_DENIED,
    Failure = SSH_FX_FAILURE,
    BadMessage = SSH_FX_BAD_MESSAGE,
    NoConnection = SSH_FX_NO_CONNECTION,
    ConnectionLost = SSH_FX_CONNECTION_LOST,
    OpUnsupported = SSH_FX_OP_UNSUPPORTED,
}

impl Status {
    fn from_status(code: u32) -> Result<Self> {
        Ok(match code {
            SSH_FX_OK => Self::OK,
            SSH_FX_EOF => Self::Eof,
            SSH_FX_NO_SUCH_FILE => Self::NoSuchFile,
            SSH_FX_PERMISSION_DENIED => Self::PermissionDenied,
            SSH_FX_FAILURE => Self::Failure,
            SSH_FX_BAD_MESSAGE => Self::BadMessage,
            SSH_FX_NO_CONNECTION => Self::NoConnection,
            SSH_FX_CONNECTION_LOST => Self::ConnectionLost,
            SSH_FX_OP_UNSUPPORTED => Self::OpUnsupported,
            _ => return Err(Error::invalid_format("Invalid SFtp status code")),
        })
    }

    fn to_result<T: Default>(&self, msg: String) -> Result<T> {
        match self {
            Status::OK => Ok(Default::default()),
            Status::Eof => Ok(Default::default()),
            Status::NoSuchFile => builder::NoSuchFile { tip: msg }.fail(), //Err(Error::NoSuchFile(msg)),
            Status::PermissionDenied => builder::PermissionDenied { tip: msg }.fail(), //Err(Error::PermissionDenied(msg)),
            Status::Failure => builder::SFtpFailure { tip: msg }.fail(), //Err(Error::SFtpFailure(msg)),
            Status::BadMessage => builder::BadMessage { tip: msg }.fail(), //Err(Error::BadMessage(msg)),
            Status::NoConnection => builder::NoConnection { tip: msg }.fail(), // Err(Error::NoConnection(msg)),
            Status::ConnectionLost => builder::NoConnection { tip: msg }.fail(), // Err(Error::ConnectionLost(msg)),
            Status::OpUnsupported => builder::OpUnsupported { tip: msg }.fail(), // Err(Error::OpUnsupported(msg)),
        }
    }

    fn no_ok_and_eof<T>(&self, msg: String) -> Result<T> {
        match self {
            Status::OK =>
            /*  Err(Error::ProtocolError(
                "Unexpected Ok status received".to_string(),
            )) */
            {
                builder::Protocol {
                    tip: "Unexpected Ok status received",
                }
                .fail()
            }
            Status::Eof =>
            /* Err(Error::ProtocolError(
                "Unexpected EOF status received".to_string(),
            ))*/
            {
                builder::Protocol {
                    tip: "Unexpected EOF status received",
                }
                .fail()
            }
            Status::NoSuchFile => builder::NoSuchFile { tip: msg }.fail(), //   Err(Error::NoSuchFile(msg)),
            Status::PermissionDenied => builder::PermissionDenied { tip: msg }.fail(), // Err(Error::PermissionDenied(msg)),
            Status::Failure => builder::SFtpFailure { tip: msg }.fail(), // Err(Error::SFtpFailure(msg)),
            Status::BadMessage => builder::BadMessage { tip: msg }.fail(), // Err(Error::BadMessage(msg)),
            Status::NoConnection => builder::NoConnection { tip: msg }.fail(), // Err(Error::NoConnection(msg)),
            Status::ConnectionLost => builder::NoConnection { tip: msg }.fail(), // Err(Error::ConnectionLost(msg)),
            Status::OpUnsupported => builder::OpUnsupported { tip: msg }.fail(), // Err(Error::OpUnsupported(msg)),
        }
    }

    fn no_eof<T: Default>(&self, msg: String) -> Result<T> {
        match self {
            Status::OK => Ok(Default::default()),
            Status::Eof =>
            /* Err(Error::ProtocolError(
                "Unexpected EOF status received".to_string(),
            ))*/
            {
                builder::Protocol {
                    tip: "Unexpected EOF status received",
                }
                .fail()
            }
            Status::NoSuchFile => builder::NoSuchFile { tip: msg }.fail(), //   Err(Error::NoSuchFile(msg)),
            Status::PermissionDenied => builder::PermissionDenied { tip: msg }.fail(), // Err(Error::PermissionDenied(msg)),
            Status::Failure => builder::SFtpFailure { tip: msg }.fail(), // Err(Error::SFtpFailure(msg)),
            Status::BadMessage => builder::BadMessage { tip: msg }.fail(), // Err(Error::BadMessage(msg)),
            Status::NoConnection => builder::NoConnection { tip: msg }.fail(), // Err(Error::NoConnection(msg)),
            Status::ConnectionLost => builder::NoConnection { tip: msg }.fail(), // Err(Error::ConnectionLost(msg)),
            Status::OpUnsupported => builder::OpUnsupported { tip: msg }.fail(), // Err(Error::OpUnsupported(msg)),
        }
    }

    fn no_ok<T: Default>(&self, msg: String) -> Result<T> {
        match self {
            Status::OK =>
            /* Err(Error::ProtocolError(
                "Unexpected Ok status received".to_string(),
            )) */
            {
                builder::Protocol {
                    tip: "Unexpected Ok status received",
                }
                .fail()
            }
            Status::Eof => Ok(Default::default()),
            Status::NoSuchFile => builder::NoSuchFile { tip: msg }.fail(), //   Err(Error::NoSuchFile(msg)),
            Status::PermissionDenied => builder::PermissionDenied { tip: msg }.fail(), // Err(Error::PermissionDenied(msg)),
            Status::Failure => builder::SFtpFailure { tip: msg }.fail(), // Err(Error::SFtpFailure(msg)),
            Status::BadMessage => builder::BadMessage { tip: msg }.fail(), // Err(Error::BadMessage(msg)),
            Status::NoConnection => builder::NoConnection { tip: msg }.fail(), // Err(Error::NoConnection(msg)),
            Status::ConnectionLost => builder::NoConnection { tip: msg }.fail(), // Err(Error::ConnectionLost(msg)),
            Status::OpUnsupported => builder::OpUnsupported { tip: msg }.fail(), // Err(Error::OpUnsupported(msg)),
        }
    }
}

#[derive(custom_debug_derive::Debug)]
enum Message {
    FileHandle(Vec<u8>),
    Status {
        status: Status,
        msg: String,
        _tag: String,
    },
    Data(#[debug(skip)] Vec<u8>),
    Name(Vec<FileInfo>),
    Attributes(Attributes),
    ExtendReply(#[debug(skip)] Vec<u8>),
}

// struct Stream<'a> {
//     sftp: &'a mut SFtp,
// }

impl SFtp {
    const MAX_SFTP_PACKET: usize = 32000;

    pub fn extend(&self, key: &str) -> Option<&[u8]> {
        self.ext.get(key).map(|v| v.as_ref())
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub async fn from_channel(channel: Channel) -> Result<Self> {
        let (sender, recver) = o_channel();

        let session = channel.session()?;
        let request = Request::SFtpFromChannel { channel, sender };

        session
            .send(request)
            .map_err(|_| builder::Disconnected.build())?;

        recver.await?
    }

    pub async fn close(self) -> Result<()> {
        self.channel.into_inner().close().await
    }

    // pub async fn flush(&mut self) -> Result<()> {
    //     self.channel.flush().await?;
    //     Ok(())
    // }

    // pub async fn flush(&self) -> Result<()> {
    //     self.channel.inner().inner().flush().await
    //}

    pub fn support_posix_rename(&self) -> bool {
        self.support(OPENSSH_SFTP_EXT_POSIX_RENAME)
    }

    pub async fn posix_rename(&mut self, oldpath: &str, newpath: &str) -> Result<()> {
        debug_assert!(
            self.support_posix_rename(),
            "Server doesn't support posix rename"
        );
        // let mut buffer = Buffer::new();

        let request_id = self.genarate_request_id();

        // buffer.put_u32(request_id);
        // buffer.put_one(OPENSSH_SFTP_EXT_POSIX_RENAME.0);
        // buffer.put_one(oldpath);
        // buffer.put_one(newpath);

        // self.send(SSH_FXP_EXTENDED, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_EXTENDED,
            u32: request_id,
            one: OPENSSH_SFTP_EXT_POSIX_RENAME.0,
            one: oldpath,
            one: newpath,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_eof).await
    }

    pub fn support_fstatvfs(&self) -> bool {
        self.support(OPENSSH_SFTP_EXT_FSTATVFS)
    }

    pub async fn fstatvfs(&mut self, file: &File) -> Result<Statvfs> {
        debug_assert!(self.support_fstatvfs(), "Server doesn't support fstatvfs");

        let request_id = self.genarate_request_id();
        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(OPENSSH_SFTP_EXT_FSTATVFS.0);
        // buffer.put_one(&file.handle);

        // self.send(SSH_FXP_EXTENDED, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_EXTENDED,
            u32: request_id,
            one: OPENSSH_SFTP_EXT_FSTATVFS.0,
            one: &file.handle,
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::Status { status, msg, _tag } => status.no_ok_and_eof(msg),
            Message::ExtendReply(data) => Statvfs::parse(&data).context(builder::Protocol {
                tip: "Invalid Statvfs Message",
            }),
            _ => builder::Protocol {
                tip: "Unexpected SFtp Message",
            }
            .fail(), // _ => Err(Error::ProtocolError("Unexpected SFtp Message".to_string())),
        }
    }

    pub fn support_statvfs(&self) -> bool {
        self.support(OPENSSH_SFTP_EXT_STATVFS)
    }

    pub async fn statvfs(&mut self, path: &str) -> Result<Statvfs> {
        debug_assert!(self.support_fstatvfs(), "Server doesn't support statvfs");

        let request_id = self.genarate_request_id();
        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(OPENSSH_SFTP_EXT_STATVFS.0);
        // buffer.put_one(path);

        // self.send(SSH_FXP_EXTENDED, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_EXTENDED,
            u32: request_id,
            one: OPENSSH_SFTP_EXT_STATVFS.0,
            one: path,
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::Status { status, msg, _tag } => status.no_ok_and_eof(msg),
            Message::ExtendReply(data) => Statvfs::parse(&data).context(builder::Protocol {
                tip: "Invalid Statvfs Message",
            }),
            _ => builder::Protocol {
                tip: "Unexpected SFtp Message",
            }
            .fail(), // _ => Err(Error::ProtocolError("Unexpected SFtp Message".to_string())),
        }
    }

    pub fn support_hardlink(&self) -> bool {
        self.support(OPENSSH_SFTP_EXT_HARDLINK)
    }

    pub async fn hardlink(&mut self, oldpath: &str, newpath: &str) -> Result<()> {
        debug_assert!(self.support_hardlink(), "Server doesn't support hardlink");

        let request_id = self.genarate_request_id();
        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(OPENSSH_SFTP_EXT_HARDLINK.0);
        // buffer.put_one(oldpath);
        // buffer.put_one(newpath);

        // self.send(SSH_FXP_EXTENDED, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_EXTENDED,
            u32: request_id,
            one: OPENSSH_SFTP_EXT_HARDLINK.0,
            one: oldpath,
            one: newpath,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_eof).await
    }

    pub fn support_fsync(&self) -> bool {
        self.support(OPENSSH_SFTP_EXT_FSYNC)
    }

    pub async fn fsync(&mut self, file: &File) -> Result<()> {
        debug_assert!(self.support_fsync(), "Server doesn't support fsync");

        let request_id = self.genarate_request_id();
        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(OPENSSH_SFTP_EXT_FSYNC.0);
        // buffer.put_one(&file.handle);

        // self.send(SSH_FXP_EXTENDED, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_EXTENDED,
            u32: request_id,
            one: OPENSSH_SFTP_EXT_FSYNC.0,
            one: &file.handle,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_eof).await
    }

    pub fn support_lsetstat(&self) -> bool {
        self.support(OPENSSH_SFTP_EXT_LSETSTAT)
    }

    pub async fn lsetstat(&mut self, path: &str, attrs: &Attributes) -> Result<()> {
        debug_assert!(self.support_lsetstat(), "Server doesn't lsetstat");

        let request_id = self.genarate_request_id();
        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(OPENSSH_SFTP_EXT_LSETSTAT.0);
        // buffer.put_one(path);
        // attrs.to_bytes(&mut buffer);

        // self.send(SSH_FXP_EXTENDED, buffer.as_ref()).await?;

        let attrs = attrs.to_buffer();

        let buffer = make_buffer! {
            u8: SSH_FXP_EXTENDED,
            u32: request_id,
            one: OPENSSH_SFTP_EXT_LSETSTAT.0,
            one: path,
            bytes: attrs,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_eof).await
    }

    pub fn support_limits(&self) -> bool {
        self.support(OPENSSH_SFTP_EXT_LIMITS)
    }

    pub async fn limits(&mut self) -> Result<Limits> {
        debug_assert!(self.support_limits(), "Server doesn't support limits");
        let request_id = self.genarate_request_id();
        // let mut buffer = Buffer::new();
        // buffer.put_one(OPENSSH_SFTP_EXT_LIMITS.0);

        // self.send(SSH_FXP_EXTENDED, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_EXTENDED,
            u32: request_id,
            one: OPENSSH_SFTP_EXT_LIMITS.0
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::ExtendReply(data) => Limits::parse(&data).context(builder::Protocol {
                tip: "Invalid packet format",
            }),
            _ => builder::Protocol {
                tip: "Unexpected SFtp Message",
            }
            .fail(),
        }
    }

    pub fn support_expand_path(&self) -> bool {
        self.support(OPENSSH_SFTP_EXT_EXPAND_PATH)
    }

    pub async fn expand_path(&mut self, path: &str) -> Result<String> {
        debug_assert!(
            self.support_expand_path(),
            "Server doesn't support expand path"
        );

        let request_id = self.genarate_request_id();
        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(OPENSSH_SFTP_EXT_EXPAND_PATH.0);
        // buffer.put_one(path);

        // self.send(SSH_FXP_EXTENDED, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_EXTENDED,
            u32: request_id,
            one: OPENSSH_SFTP_EXT_EXPAND_PATH.0,
            one: path
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::Status { status, msg, .. } => status.no_ok_and_eof(msg),
            Message::Name(infos) if infos.len() == 1 => Ok(infos[0].filename.clone()),
            // _ => Err(Error::ProtocolError("Unknown msg".to_string())),
            _ => builder::Protocol { tip: "Unknown msg" }.fail(),
        }
    }

    pub fn support_copy_data(&self) -> bool {
        self.support(OPENSSH_SFTP_EXT_COPY_DATA)
    }

    pub async fn copy_data(&mut self, read: &mut File, len: u64, write: &mut File) -> Result<()> {
        debug_assert!(self.support_copy_data(), "Server doesn't support copy data");

        let request_id = self.genarate_request_id();
        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(OPENSSH_SFTP_EXT_COPY_DATA.0);
        // buffer.put_one(&read.handle);
        // buffer.put_u64(read.pos);
        // buffer.put_u64(len);
        // buffer.put_one(&write.handle);
        // buffer.put_u64(write.pos);

        // self.send(SSH_FXP_EXTENDED, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_EXTENDED,
            u32: request_id,
            one: OPENSSH_SFTP_EXT_COPY_DATA.0,
            one: &read.handle,
            u64: read.pos,
            u64: len,
            one: &write.handle,
            u64: write.pos,
        };

        self.channel.write_all(buffer).await?;

        let status = self.wait_for_status(request_id, Status::to_result).await;

        if status.is_ok() {
            read.pos += len;
            write.pos += len;
        }

        status
    }

    pub fn support_home_directory(&self) -> bool {
        self.support(OPENSSH_SFTP_EXT_HOME_DIRECTORY)
    }

    pub async fn home_directory(&mut self, username: &str) -> Result<String> {
        debug_assert!(
            self.support_home_directory(),
            "Server doesn't support home directory"
        );

        let request_id = self.genarate_request_id();
        // cap: 4 + 1 + 4 + 4 + xx.len() + 4 + username.len()
        // let mut buffer = Buffer::with_capacity(
        //     4 + 1 + 4 + 4 + OPENSSH_SFTP_EXT_HOME_DIRECTORY.0.len() + 4 + username.len(),
        // );
        // buffer.put_u32(
        //     (4 + 1 + 4 + 4 + OPENSSH_SFTP_EXT_HOME_DIRECTORY.0.len() + 4 + username.len()) as u32,
        // );
        // buffer.put_u8(SSH_FXP_EXTENDED);
        // buffer.put_u32(request_id);
        // buffer.put_one(OPENSSH_SFTP_EXT_HOME_DIRECTORY.0);
        // buffer.put_one(username);

        // self.write(buffer).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_EXTENDED,
            u32: request_id,
            one: OPENSSH_SFTP_EXT_HOME_DIRECTORY.0,
            one: username
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::Status { status, msg, _tag } => status.no_ok_and_eof(msg),
            Message::Name(infos) if infos.len() == 1 => Ok(infos[0].filename.clone()),
            // _ => Err(Error::ProtocolError("Unexpected message".to_string())),
            _ => builder::Protocol {
                tip: "Unexpected message",
            }
            .fail(),
        }
    }

    pub fn support_users_groups_by_id(&self) -> bool {
        self.support(OPENSSH_SFTP_EXT_USERS_GROUPS_BY_ID)
    }

    pub async fn users_groups_by_id(
        &mut self,
        users: &[u32],
        groups: &[u32],
    ) -> Result<(Vec<String>, Vec<String>)> {
        let request_id = self.genarate_request_id();
        let cap = 4 // buffer len
            + 1 // ssh code
            + 4 // request id
            + 4 // extend string len
            + OPENSSH_SFTP_EXT_USERS_GROUPS_BY_ID.0.len() // extend string content
            + 4 // users id block len
            + users.len() * 4 // users id
            + 4 // groups id block len
            + groups.len() * 4; // groups id
        let mut buffer = Buffer::with_capacity(cap);
        buffer.put_u32((cap - 4) as u32);
        buffer.put_u8(SSH_FXP_EXTENDED);
        buffer.put_u32(request_id);
        buffer.put_one(OPENSSH_SFTP_EXT_USERS_GROUPS_BY_ID.0);

        buffer.put_u32((users.len() * 4) as u32);

        users.iter().for_each(|v| {
            buffer.put_u32(*v);
        });

        buffer.put_u32((groups.len() * 4) as u32);

        groups.iter().for_each(|v| {
            buffer.put_u32(*v);
        });

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::ExtendReply(data) => {
                let buffer = Buffer::from_slice(&data);
                let usernames = buffer
                    .take_one()
                    .context(builder::Protocol {
                        tip: "Invalid sftp packet format",
                    })?
                    .1;
                let usernames = Buffer::from_slice(usernames);

                let groupnames = buffer
                    .take_one()
                    .context(builder::Protocol {
                        tip: "Invalid sftp packet format",
                    })?
                    .1;
                let groupnames = Buffer::from_slice(groupnames);

                let mut unames = vec![];
                while let Some(user) = usernames.take_one() {
                    // unames.push(String::from_utf8(user.1).map_err(|e| e.utf8_error())?);
                    unames.push(std::str::from_utf8(user.1)?.to_string())
                }

                let mut gnames = vec![];

                while let Some(group) = groupnames.take_one() {
                    // gnames.push(String::from_utf8(group.1).map_err(|e| e.utf8_error())?)
                    gnames.push(std::str::from_utf8(group.1)?.to_string());
                }

                Ok((unames, gnames))
            }
            _ => builder::Protocol {
                tip: "Unexpected message",
            }
            .fail(), //Err(Error::ProtocolError("Unexpected message".to_string())),
        }
    }

    fn support(&self, (e, v): (&str, &[u8])) -> bool {
        self.ext.get(e).map(|v| v.as_ref()) == Some(v)
    }

    // pub async fn close(mut self) -> Result<()> {
    //     self.closed = true;
    //     let (sender, recver) = async_channel::bounded(1);
    //     self.session
    //         .send(Request::ChannelDrop {
    //             id: self.id,
    //             sender: Some(sender),
    //         })
    //         .await
    //         .map_err(|_| Error::Disconnect)?;
    //     recver.recv().await.map_err(|_| Error::Disconnect)?
    // }

    pub fn seek_file(&self, file: &mut File, pos: u64) {
        file.pos = pos;
    }

    pub async fn close_file(&mut self, file: File) -> Result<()> {
        let request_id = self.genarate_request_id();

        // let mut buffer = Buffer::new();

        // buffer.put_u32(request_id);
        // buffer.put_one(file.handle);

        // self.send(SSH_FXP_CLOSE, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_CLOSE,
            u32: request_id,
            one: file.handle,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_eof).await
    }

    pub async fn read_file_buf(&mut self, file: &mut File, max: u32) -> Result<Vec<Vec<u8>>> {
        let base = 255 * 1024;

        let mut times = max / base;
        if times == 0 {
            times = 1;
        }

        let mut requests = Vec::with_capacity(times as usize);

        let mut datas = Vec::with_capacity(times as usize);

        let mut all =
            Vec::with_capacity(times as usize * (4 + 1 + 4 + 4 + file.handle.len() + 8 + 4));

        let mut pos = file.pos;
        for _ in 0..times {
            let request_id = self.genarate_request_id();

            let buffer = make_buffer! {
                u8: SSH_FXP_READ,
                u32: request_id,
                one: &file.handle,
                u64: pos,
                u32: base
            };

            all.extend(buffer.into_vec());

            requests.push(request_id);

            pos += base as u64;
        }
        // let first = std::time::Instant::now();
        self.channel.write_all(all).await?;

        // println!("spent1: {}", first.elapsed().as_millis());
        // let mut first = std::time::Instant::now();

        for i in requests {
            let packet = self.wait_for_packet(i).await?;
            // println!("spent2: {}", first.elapsed().as_millis());
            // first = std::time::Instant::now();

            match packet.msg {
                Message::Data(data) => {
                    file.pos += data.len() as u64;
                    datas.push(data);
                }
                Message::Status {
                    status: Status::Eof,
                    ..
                } => {
                    datas.push(vec![]);
                }
                Message::Status { status, msg, .. } => return status.no_ok(msg),
                _ => return builder::Protocol { tip: "Unknown msg" }.fail(), // return Err(Error::ProtocolError("Unknown msg".to_string())),
            }
        }

        Ok(datas)
    }

    pub async fn read_file(&mut self, file: &mut File, max: u32) -> Result<Vec<u8>> {
        let request_id = self.genarate_request_id();

        // // cap: 4 + 1 + 4 + (4 + file.handle.len()) + 8 + 4
        // let len = 1 + 4 + 4 + file.handle.len() + 8 + 4;
        // let mut buffer = Buffer::with_capacity(4 + len);

        // buffer.put_u32(len as u32);
        // buffer.put_u8(SSH_FXP_READ);

        // buffer.put_u32(request_id);

        // buffer.put_one(&file.handle);
        // buffer.put_u64(file.pos);

        // buffer.put_u32(max);

        // self.write(buffer).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_READ,
            u32: request_id,
            one: &file.handle,
            u64: file.pos,
            u32: max
        };

        self.channel.write_all(buffer).await?;

        // self.send(SSH_FXP_READ, buffer.as_ref()).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::Data(data) => {
                file.pos += data.len() as u64;
                Ok(data)
            }
            Message::Status { status, msg, .. } => status.no_ok(msg),
            _ => builder::Protocol { tip: "Unknown msg" }.fail(), //Err(Error::ProtocolError("Unknown msg".to_string())),
        }
    }

    // async fn send(&mut self, code: u8, bytes: &[u8]) -> Result<()> {
    //     // let mut packet = Buffer::with_capacity(4 + 1 + bytes.len());

    //     // packet.put_u32((bytes.len() + 1) as u32);
    //     // packet.put_u8(code);
    //     // packet.put_bytes(bytes);

    //     let packet = make_buffer! {
    //         u32: (bytes.len() + 1) as u32,
    //         u8: code,
    //         bytes: bytes,
    //     };

    //     self.write(packet).await?;
    //     Ok(())
    // }

    pub async fn write_file_buf(&mut self, file: &mut File, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let max = Self::MAX_SFTP_PACKET;
        let mut requests = vec![];
        // cap: 4 + 1 + 4 + 4 + file.handle.len() + 8 + 4 + data
        let mut buffer =
            Buffer::with_capacity(4 + 1 + 4 + 4 + file.handle.len() + 8 + 4 + min(max, data.len()));
        for i in (0..data.len()).step_by(max) {
            let left = data.len() - i;

            let min = min(left, max);

            let request_id = self.genarate_request_id();

            buffer.put_u32((1 + 4 + 4 + file.handle.len() + 8 + 4 + min) as u32);
            buffer.put_u8(SSH_FXP_WRITE);
            buffer.put_u32(request_id);
            buffer.put_one(&file.handle);
            buffer.put_u64(file.pos);
            buffer.put_one(&data[i..i + min]);

            // self.channel.write_all(&buffer).await?;

            self.channel.write(&buffer).await?;

            requests.push(request_id);
            file.pos += min as u64;
            buffer.clear();
        }

        self.channel.flush().await?;

        for id in requests {
            // suppress warning in edition 2021, maybe error in edition 2024
            let _: () = self.wait_for_status(id, Status::no_eof).await?;
        }

        Ok(())
    }

    pub async fn write_file(&mut self, file: &mut File, data: &[u8]) -> Result<()> {
        let max = Self::MAX_SFTP_PACKET;
        for i in (0..data.len()).step_by(max) {
            let left = data.len() - i;

            let min = min(left, max);

            self.write_file_unchecked(file, &data[i..i + min]).await?;
        }

        Ok(())
    }

    async fn write_file_unchecked(&mut self, file: &mut File, data: &[u8]) -> Result<()> {
        // ssh最大数据包检查
        // cap: 4 + 1 + 4 + file.handle.len() + 8 + 4 + data.len()

        let request_id = self.genarate_request_id();

        let buffer = make_buffer! {
            u8: SSH_FXP_WRITE,
            u32: request_id,
            one: &file.handle,
            u64: file.pos,
            one: data
        };

        self.channel.write_all(buffer).await?;

        let res = self.wait_for_status(request_id, Status::no_eof).await;
        if res.is_ok() {
            file.pos += data.len() as u64;
        }
        res
    }

    async fn wait_for_status<T, B>(&mut self, id: u32, f: T) -> Result<B>
    where
        T: FnOnce(&Status, String) -> Result<B>,
    {
        let packet = self.wait_for_packet(id).await?;

        match packet.msg {
            // Message::Status { status, .. } if status == Status::OK => Ok(()),
            Message::Status { status, msg, .. } => f(&status, msg),
            _ => builder::Protocol { tip: "Unknown msg" }.fail(), //Err(Error::ProtocolError("Unknown msg".to_string())),
                                                                  // _ => Err(Error::ProtocolError("Unknown msg received".to_string())),
        }
    }

    pub async fn mkdir(&mut self, path: &str, permissions: Permissions) -> Result<()> {
        let request_id = self.genarate_request_id();
        let flags = SSH_FILEXFER_ATTR_PERMISSIONS;
        let permissions_bits = permissions.bits();

        let buffer = make_buffer! {
            u8: SSH_FXP_MKDIR,
            u32: request_id,
            one: path,
            u32: flags,
            u32: permissions_bits,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_eof).await
    }

    pub async fn rmdir(&mut self, path: &str) -> Result<()> {
        let request_id = self.genarate_request_id();

        let buffer = make_buffer! {
            u8: SSH_FXP_RMDIR,
            u32: request_id,
            one: path,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_ok).await
    }

    pub async fn open_dir(&mut self, path: &str) -> Result<Dir> {
        let request_id = self.genarate_request_id();

        let buffer = make_buffer! {
            u8: SSH_FXP_OPENDIR,
            u32: request_id,
            one: path,
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::FileHandle(handle) => Ok(Dir::new(handle)),
            Message::Status { status, msg, .. } => status.no_ok_and_eof(msg),

            _ => builder::Protocol { tip: "Unknown msg" }.fail(), //Err(Error::ProtocolError("Unknown msg".to_string())),
        }
    }

    pub async fn close_dir(&mut self, dir: Dir) -> Result<()> {
        let request_id = self.genarate_request_id();

        // let mut buffer = Buffer::new();

        // buffer.put_u32(request_id);
        // buffer.put_one(dir.handle);

        // self.send(SSH_FXP_CLOSE, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_CLOSE,
            u32: request_id,
            one: dir.handle,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_eof).await
    }

    pub async fn read_dir(&mut self, dir: &Dir) -> Result<Vec<FileInfo>> {
        let request_id = self.genarate_request_id();

        let buffer = make_buffer! {
            u8: SSH_FXP_READDIR,
            u32: request_id,
            one: &dir.handle,
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::Status { status, msg, .. } => status.no_ok(msg),
            Message::Name(infos) => Ok(infos),
            _ => builder::Protocol { tip: "Unknown msg" }.fail(), //Err(Error::ProtocolError("Unknown msg".to_string())),
                                                                  // _ => Err(Error::ProtocolError("Unknown msg".to_string())),
        }
    }

    pub async fn stat(&mut self, path: &str) -> Result<Attributes> {
        let request_id = self.genarate_request_id();

        let buffer = make_buffer! {
            u8: SSH_FXP_STAT,
            u32: request_id,
            one: path,
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::Status { status, msg, .. } => status.no_ok_and_eof(msg),
            Message::Attributes(attrs) => Ok(attrs),
            _ => builder::Protocol { tip: "Unknown msg" }.fail(), //Err(Error::ProtocolError("Unknown msg".to_string())),
                                                                  // _ => Err(Error::ProtocolError("Unknown msg".to_string())),
        }
    }

    pub async fn lstat(&mut self, path: &str) -> Result<Attributes> {
        let request_id = self.genarate_request_id();

        let buffer = make_buffer! {
            u8: SSH_FXP_LSTAT,
            u32: request_id,
            one: path,
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::Status { status, msg, .. } => status.no_ok_and_eof(msg),
            Message::Attributes(attrs) => Ok(attrs),
            _ => builder::Protocol { tip: "Unknown msg" }.fail(), //Err(Error::ProtocolError("Unknown msg".to_string())),
                                                                  // _ => Err(Error::ProtocolError("Unknown msg".to_string())),
        }
    }

    pub async fn fstat(&mut self, file: &File) -> Result<Attributes> {
        let request_id = self.genarate_request_id();
        let buffer = make_buffer! {
            u8: SSH_FXP_FSTAT,
            u32: request_id,
            one: &file.handle,
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::Status { status, msg, .. } => status.no_ok_and_eof(msg),
            Message::Attributes(attrs) => Ok(attrs),
            _ => builder::Protocol { tip: "Unknown msg" }.fail(), //Err(Error::ProtocolError("Unknown msg".to_string())),
                                                                  // _ => Err(Error::ProtocolError("Unknown msg".to_string())),
        }
    }

    pub async fn setstat(&mut self, path: &str, attrs: &Attributes) -> Result<()> {
        let request_id = self.genarate_request_id();

        let attrs = attrs.to_buffer();

        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(path);

        // attrs.to_bytes(&mut buffer);

        let buffer = make_buffer! {
            u8: SSH_FXP_SETSTAT,
            u32: request_id,
            one: path,
            bytes: attrs,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_eof).await
    }

    pub async fn setfstat(&mut self, file: &File, attrs: &Attributes) -> Result<()> {
        let request_id = self.genarate_request_id();

        let attrs = attrs.to_buffer();
        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(&file.handle);

        // attrs.to_bytes(&mut buffer);

        // self.send(SSH_FXP_FSETSTAT, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_FSETSTAT,
            u32: request_id,
            one: &file.handle,
            bytes: attrs,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_eof).await
    }

    pub async fn readlink(&mut self, path: &str) -> Result<FileInfo> {
        let request_id = self.genarate_request_id();

        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(path);

        // self.send(SSH_FXP_READLINK, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_READLINK,
            u32: request_id,
            one: path,
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::Status { status, msg, .. } => status.no_ok_and_eof(msg),
            Message::Name(mut infos) if infos.len() == 1 => Ok(infos.remove(0)),
            _ => builder::Protocol { tip: "Unknown msg" }.fail(), //Err(Error::ProtocolError("Unknown msg".to_string())),
                                                                  // _ => Err(Error::ProtocolError("Unknown msg".to_string())),
        }
    }

    pub async fn symlink(&mut self, linkpath: &str, targetpath: &str) -> Result<()> {
        let request_id = self.genarate_request_id();

        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(linkpath);
        // buffer.put_one(targetpath);

        // self.send(SSH_FXP_SYMLINK, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_SYMLINK,
            u32: request_id,
            one: linkpath,
            one: targetpath,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_eof).await
    }

    pub async fn realpath(&mut self, path: &str) -> Result<String> {
        let request_id = self.genarate_request_id();

        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(path);

        // self.send(SSH_FXP_REALPATH, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_REALPATH,
            u32: request_id,
            one: path,
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::Status { status, msg, .. } => status.no_ok_and_eof(msg),
            Message::Name(infos) if infos.len() == 1 => Ok(infos[0].filename.clone()),
            _ => builder::Protocol { tip: "Unknown msg" }.fail(), //Err(Error::ProtocolError("Unknown msg".to_string())),
                                                                  // _ => Err(Error::ProtocolError("Unknown msg".to_string())),
        }
    }

    pub async fn rename_file_or_dir(&mut self, old: &str, new: &str) -> Result<()> {
        let request_id = self.genarate_request_id();

        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(old);
        // buffer.put_one(new);

        // self.send(SSH_FXP_RENAME, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_RENAME,
            u32: request_id,
            one: old,
            one: new,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_eof).await
    }

    pub async fn remove_file(&mut self, file: &str) -> Result<()> {
        let request_id = self.genarate_request_id();

        // let mut buffer = Buffer::new();
        // buffer.put_u32(request_id);
        // buffer.put_one(file);

        // self.send(SSH_FXP_REMOVE, buffer.as_ref()).await?;

        let buffer = make_buffer! {
            u8: SSH_FXP_REMOVE,
            u32: request_id,
            one: file,
        };

        self.channel.write_all(buffer).await?;

        self.wait_for_status(request_id, Status::no_eof).await
    }

    pub async fn open_file(
        &mut self,
        filename: &str,
        flags: OpenFlags,
        permissions: Option<Permissions>,
    ) -> Result<File> {
        let request_id = self.genarate_request_id();
        let mut flag = 0;

        let mut tmp = Buffer::new();
        if let Some(permissions) = permissions {
            flag |= SSH_FILEXFER_ATTR_PERMISSIONS;
            tmp.put_u32(permissions.bits());
        }

        // let mut buffer = Buffer::new();

        // buffer.put_u32(request_id);
        // buffer.put_one(filename);
        // buffer.put_u32(flags.bits());

        // buffer.put_u32(flag);
        // buffer.put_bytes(tmp);

        // self.send(SSH_FXP_OPEN, buffer.as_ref()).await?;

        let openflags = flags.bits();
        let buffer = make_buffer! {
            u8: SSH_FXP_OPEN,
            u32: request_id,
            one: filename,
            u32: openflags,
            u32: flag,
            bytes: tmp,
        };

        self.channel.write_all(buffer).await?;

        let packet = self.wait_for_packet(request_id).await?;

        match packet.msg {
            Message::FileHandle(handle) => Ok(File::new(handle)),
            Message::Status { status, msg, .. } => status.no_ok_and_eof(msg),

            _ => builder::Protocol { tip: "Unknown msg" }.fail(), //Err(Error::ProtocolError("Unknown msg".to_string())),
                                                                  // _ => Err(Error::ProtocolError("Unknown msg".to_string())),
        }
    }

    pub fn file_streamer<'a>(&'a mut self, file: &'a mut File) -> Stream<'a> {
        Stream {
            sftp: self,
            file,
            read_future: None,
            write_future: None,
        }
    }

    async fn wait_for_packet(&mut self, id: u32) -> Result<Packet> {
        let packet = self.packets.remove(&id);
        if let Some(packet) = packet {
            return Ok(packet);
        }
        loop {
            let packet = self.recv().await?;
            if packet.id == id {
                return Ok(packet);
            }
            self.packets.insert(packet.id, packet);
        }
    }

    async fn recv(&mut self) -> Result<Packet> {
        let data = self.channel.fill(4).await?;

        let len = u32::from_be_bytes(data.try_into().unwrap());

        let data = self.channel.fill(4 + len as usize).await?;

        let res = Packet::parse(data).context(builder::InvalidArgument {
            tip: "Unable to parse sftp packet",
        });
        self.channel.consume(4 + len as usize);
        res
    }

    fn genarate_request_id(&mut self) -> u32 {
        self.request_id = self.request_id.wrapping_add(1);
        self.request_id
    }

    // async fn write(&mut self, data: impl AsRef<[u8]>) -> Result<()> {
    //     if !self.channel.write(data.as_ref()).await? {
    //         self.channel.flush().await?;
    //     }
    //     Ok(())
    // }
}
