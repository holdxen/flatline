
use std::collections::HashMap;

use snafu::ResultExt;
use num_enum::{IntoPrimitive, TryFromPrimitive};

use crate::{error, ssh::{buffer::{Consumer, Producer}, msg, protocol::sftp::*}};

bitflags::bitflags! {
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

bitflags::bitflags! {
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
    pub(super) fn parse(data: &[u8]) -> error::Result<Self> {
        let mut consumer = Consumer::new(data);

        Ok(Self {
            bsize: consumer.consume_u64()?,
            frsize: consumer.consume_u64()?,
            blocks: consumer.consume_u64()?,
            bfree: consumer.consume_u64()?,
            bavail: consumer.consume_u64()?,
            files: consumer.consume_u64()?,
            ffree: consumer.consume_u64()?,
            favail: consumer.consume_u64()?,
            fsid: consumer.consume_u64()?,
            flag: consumer.consume_u64()?,
            namemax: consumer.consume_u64()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
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

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_packet_len: u64,
    pub max_read_len: u64,
    pub max_write_len: u64,
    pub max_open_handles: u64,
}

impl Limits {
    pub(super) fn parse(data: &[u8]) -> error::Result<Self> {
        let mut consumer = Consumer::new(data);

        Ok(Self {
            max_packet_len: consumer.consume_u64()?,
            max_read_len: consumer.consume_u64()?,
            max_write_len: consumer.consume_u64()?,
            max_open_handles: consumer.consume_u64()?,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash, TryFromPrimitive)]
#[repr(u32)]
pub enum Status {
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
    pub(super) fn to_error(&self, msg: String) -> error::Error {
        match self {
            Status::OK =>super::UnexpectedResponseSnafu.build().into(),
            Status::Eof =>super::UnexpectedEofSnafu { msg }.build().into(),
            Status::NoSuchFile =>super::NoSuchFileSnafu { msg }.build().into(),
            Status::PermissionDenied =>super::PermissionDeniedSnafu { msg }.build().into(),
            Status::Failure =>super::FailureSnafu { msg }.build().into(),
            Status::BadMessage =>super::BadMessageSnafu { msg }.build().into(),
            Status::NoConnection =>super::NoConnectionSnafu { msg }.build().into(),
            Status::ConnectionLost =>super::ConnectionLostSnafu { msg }.build().into(),
            Status::OpUnsupported =>super::OpUnsupportedSnafu { msg }.build().into(),   
        }
    }
    pub(super) fn to_result(&self, msg: String) -> error::Result<()> {
        match self {
            Status::OK => Ok(()),
            Status::Eof => Err(super::UnexpectedEofSnafu { msg }.build().into()),
            Status::NoSuchFile => Err(super::NoSuchFileSnafu { msg }.build().into()),
            Status::PermissionDenied => Err(super::PermissionDeniedSnafu { msg }.build().into()),
            Status::Failure => Err(super::FailureSnafu { msg }.build().into()),
            Status::BadMessage =>Err(super::BadMessageSnafu { msg }.build().into()),
            Status::NoConnection =>Err(super::NoConnectionSnafu { msg }.build().into()),
            Status::ConnectionLost =>Err(super::ConnectionLostSnafu { msg }.build().into()),
            Status::OpUnsupported =>Err(super::OpUnsupportedSnafu { msg }.build().into()),   
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub file_name: String,
    pub long_name: String,
    pub attributes: Attributes,
}

impl FileInfo {
    fn parse(data: &[u8]) -> error::Result<Self> {
        let mut consumer = Consumer::new(data);

        let file_name = consumer.consume_one()?;
        let file_name = std::str::from_utf8(file_name).context(msg::ExpectStringSnafu)?.to_string();

        let long_name = consumer.consume_one()?;
        let long_name = std::str::from_utf8(long_name).context(msg::ExpectStringSnafu)?.to_string();

        let attributes = Attributes::parse(consumer.peek())?;

        Ok(Self {
            file_name,
            long_name,
            attributes,
        })
    }
}

#[derive(derive_more::Debug, Clone)]
pub(super) enum Payload {
    Status {
        status: Status,
        error: String,
        language: String,
    },
    Handle(Vec<u8>),
    Data(#[debug(skip)] Vec<u8>),
    Name(Vec<FileInfo>),
    Attributes(Attributes),
    ExtendReply(Vec<u8>)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub atime: u32,
    pub mtime: u32,
}

impl Timestamp {
    fn new(atime: u32, mtime: u32) -> Self {
        Self { atime, mtime }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
}

impl User {
    fn new(uid: u32, gid: u32) -> Self {
        Self { uid, gid }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionsAndFileType {
    pub permissions: Permissions,
    pub file_type: FileType,
}

impl TryFrom<u32> for PermissionsAndFileType {
    type Error = error::Error;

    fn try_from(value: u32) -> std::result::Result<Self, Self::Error> {
        Ok(Self::new(
            Permissions::from_bits_truncate(value & Permissions::MASK),
            (value & FileType::MASK).try_into().context(super::UnknownFileTypeSnafu)?,
        ))
    }
}

impl PermissionsAndFileType {
    pub fn new(permissions: Permissions, file_type: FileType) -> Self {
        Self {
            permissions,
            file_type,
        }
    }
    pub fn bits(&self) -> u32 {
        self.permissions.bits() | self.file_type as u32
    }
}

#[derive(Debug, Clone)]
pub(super) struct Message {
    pub id: u32,
    pub payload: Payload,
}

impl Message {
    pub fn parse(data: &[u8]) -> error::Result<Message> {

        let mut consumer = Consumer::new(data);
        let r#type = consumer.consume_u8()?;

        let id = consumer.consume_u32()?;

        match r#type {
            SSH_FXP_STATUS => {
                let code = consumer.consume_u32()?;
                let status = Status::try_from(code).context(super::UnexpectedStatusSnafu {
                    status: code,
                })?;

                let error = consumer.consume_one().unwrap_or_default();
                let error = std::str::from_utf8(error).context(msg::ExpectStringSnafu)?.to_string();

                let language = consumer.consume_one().unwrap_or_default();
                let language = std::str::from_utf8(language).context(msg::ExpectStringSnafu)?.to_string();

                Ok(Message {
                    id,
                    payload: Payload::Status {
                        status,
                        error,
                        language,
                    },
                })
            }
            SSH_FXP_HANDLE => {
                let handle = consumer.consume_one()?;
                Ok(Message {
                    id,
                    payload: Payload::Handle(handle.to_vec()),
                })
            }
            SSH_FXP_DATA => {
                let data = consumer.consume_one()?;
                Ok(Message {
                    id,
                    payload: Payload::Data(data.to_vec()),
                })
            }
            SSH_FXP_NAME => {
                let count = consumer.consume_u32()?;
                let mut file_infos = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let file_info = FileInfo::parse(consumer.peek())?;
                    file_infos.push(file_info);
                }

                Ok(Message { id, payload: Payload::Name(file_infos) })
            }
            SSH_FXP_ATTRS => {
                let attributes = Attributes::parse(consumer.peek())?;
                Ok(Message { id, payload: Payload::Attributes(attributes) })
            }
            SSH_FXP_EXTENDED_REPLY => {
                Ok(Message { id, payload: Payload::ExtendReply(consumer.peek().to_vec()) })
            }
            code => {
                Err(super::UnexpectedMessageSnafu {
                    code
                }.build().into())
            }
        }

    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributes {
    pub size: Option<u64>,
    pub user: Option<User>,
    pub property: Option<PermissionsAndFileType>,
    pub time: Option<Timestamp>,
    pub extend: Option<HashMap<String, Vec<u8>>>,
}

impl Attributes {

    fn new(
        size: Option<u64>,
        user: Option<User>,
        property: Option<PermissionsAndFileType>,
        time: Option<Timestamp>,
        extend: Option<HashMap<String, Vec<u8>>>,
    ) -> Self {
        Self {
            size,
            user,
            property,
            time,
            extend,
        }
    }

    pub(super) fn to_bytes(&self) -> Vec<u8> {
        let mut flags = 0;
        let mut producer = Producer::default();

        producer.put_u32(0); // flags

        if let Some(size) = self.size {
            flags |= SSH_FILEXFER_ATTR_SIZE;
            producer.put_u64(size);
        }

        if let Some(user) = self.user {
            flags |= SSH_FILEXFER_ATTR_UIDGID;
            producer.put_u32(user.uid);
            producer.put_u32(user.gid);
        }

        if let Some(permissions) = self.property {
            flags |= SSH_FILEXFER_ATTR_PERMISSIONS;
            producer.put_u32(permissions.bits());
        }

        if let Some(time) = self.time {
            flags |= SSH_FILEXFER_ATTR_ACMODTIME;
            producer.put_u32(time.atime);
            producer.put_u32(time.mtime);
        }

        if let Some(ref extend) = self.extend {
            flags |= SSH_FILEXFER_ATTR_EXTENDED;

            let count =extend.len() as u32;

            producer.put_u32(count);

            for (k, v) in extend {
                producer.put_one(k);
                producer.put_one(v);
            }
        }


        producer[..4].copy_from_slice(&flags.to_be_bytes());

        producer.into_vec()
    }

    fn parse(data: &[u8]) -> error::Result<Self> {
        let mut consumer = Consumer::new(data);
        let flags = consumer.consume_u32()?;

        let mut size = None;
        let mut user = None;
        let mut permissions = None;
        let mut time = None;

        let mut extend = None;

        if flags & SSH_FILEXFER_ATTR_SIZE != 0 {
            size = Some(consumer.consume_u64()?)
        }

        if flags & SSH_FILEXFER_ATTR_UIDGID != 0 {
            let uid = consumer.consume_u32()?;
            let gid = consumer.consume_u32()?;
            user = Some(User::new(uid, gid))
        }

        if flags & SSH_FILEXFER_ATTR_PERMISSIONS != 0 {
            let per = consumer.consume_u32()?;
            permissions = PermissionsAndFileType::try_from(per).ok();
        }

        if flags & SSH_FILEXFER_ATTR_ACMODTIME != 0 {
            let atime = consumer.consume_u32()?;
            let mtime = consumer.consume_u32()?;

            time = Some(Timestamp::new(atime, mtime))
        }

        if flags & SSH_FILEXFER_ATTR_EXTENDED != 0 {
            extend = {
                let mut extend = HashMap::new();
                let ecount = consumer.consume_u32()?;

                for _ in 0..ecount {
                    let key = consumer.consume_one()?;
                    let value = consumer.consume_one()?;

                    extend.insert(std::str::from_utf8(key).context(msg::ExpectStringSnafu)?.to_string(), value.to_vec());
                }
                Some(extend)
            };
        }

        Ok(Self::new(size, user, permissions, time, extend))
    }
}

#[derive(Debug, Clone)]
pub struct File {
    handle: Vec<u8>,
    pos: u64
}

impl File {
    pub(super) fn new(handle: Vec<u8>) -> Self {
        Self { handle, pos: 0 }
    }

    pub(super) fn handle(&self) -> &[u8] {
        &self.handle
    }

    pub fn pos(&self) -> u64 {
        self.pos
    }

    #[inline(always)]
    pub fn forward(&mut self, offset: u64) {
        self.pos += offset;
    }

    #[inline(always)]
    pub fn backward(&mut self, offset: u64) {
        self.pos -= offset;
    }
}


pub struct Directory {
    handle: Vec<u8>,
}

impl Directory {
    pub(super) fn new(handle: Vec<u8>) -> Self {
        Self { handle }
    }

    pub(super) fn handle(&self) -> &[u8] {
        &self.handle
    }
}