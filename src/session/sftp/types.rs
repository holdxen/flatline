//! Wire-level types of the SFTP protocol.
//!
//! This module defines the data that travels over the wire in version 3 of
//! the SSH File Transfer Protocol (draft-ietf-secsh-filexfer-01): open flags
//! and permission bits, file attributes, status codes, directory entries,
//! file and directory handles, and the replies of the OpenSSH protocol
//! extensions (`Statvfs`, `Limits`). All public items are re-exported by the
//! parent `session::sftp` module, which is where they should be imported
//! from.

use std::collections::HashMap;

use num_enum::{IntoPrimitive, TryFromPrimitive};
use snafu::ResultExt;

use crate::{
    error,
    ssh::{
        self,
        buffer::{Consumer, Producer},
        protocol::sftp::*,
    },
};

bitflags::bitflags! {
    // https://datatracker.ietf.org/doc/html/draft-ietf-secsh-filexfer-01#section-7.3
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// Portable file-access flags sent with an `SSH_FXP_OPEN` request.
    ///
    /// See section 7.3 of draft-ietf-secsh-filexfer-01 for the wire
    /// definition of these `SSH_FXF_*` bits.
    pub struct OpenFlags: u32 {
        // Open the file for reading
        /// Open the file for reading.
        const READ                        = SSH_FXF_READ;
        // Open the file for writing.  If both this and SSH_FXF_READ are specified,
        // the file is opened for both reading and writing.
        /// Open the file for writing; combined with [`OpenFlags::READ`] the file
        /// is opened for both reading and writing.
        const WRITE                       = SSH_FXF_WRITE;
        // Force all writes to append data at the end of the file.
        /// Force every write to append data at the end of the file.
        const APPEND                      = SSH_FXF_APPEND;
        // If this flag is specified, then a new file will be created if one
        // does not alread exist (if O_TRUNC is specified, the new file will
        // be truncated to zero length if it previously exists)
        /// Create a new file if one does not already exist; with
        /// [`OpenFlags::TRUNC`] an existing file is truncated to zero length.
        const CREAT                       = SSH_FXF_CREAT;
        // Forces an existing file with the same name to be truncated to zero
        // length when creating a file by specifying SSH_FXF_CREAT.
        // SSH_FXF_CREAT MUST also be specified if this flag is used.
        /// Truncate an existing file to zero length when it is created;
        /// [`OpenFlags::CREAT`] must also be specified when using this flag.
        const TRUNC                       = SSH_FXF_TRUNC;
        // Causes the request to fail if the named file already exists.
        // SSH_FXF_CREAT MUST also be specified if this flag is used.
        /// Fail the request if the named file already exists; [`OpenFlags::CREAT`]
        /// must also be specified when using this flag.
        const EXCL                        = SSH_FXF_EXCL;
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    /// POSIX permission bits used in an SFTP attribute block (`SSH_FILEXFER_ATTR_PERMISSIONS`).
    ///
    /// The bits are the traditional Unix mode bits for owner, group, and
    /// others; the file type is not part of this type and is carried
    /// separately by [`FileType`] (both are recombined by
    /// [`PermissionsAndFileType::bits`] before they go on the wire).
    ///
    /// Each octal digit of a `mode_t` is three bits, so the three classes
    /// are shifted by 0, 3 and 6: `OTHER_*` occupies bits 0..=2, `GROUP_*`
    /// bits 3..=5 and `OWNER_*` bits 6..=8. SFTP itself does not define
    /// these values — draft-ietf-secsh-filexfer-01 section 5 says the field
    /// is "a bit mask of file permissions as defined by [POSIX]".
    ///
    /// Set-user-ID (`0o4000`), set-group-ID (`0o2000`) and the sticky bit
    /// (`0o1000`) are deliberately not modelled; see
    /// [`TryFrom<u32> for PermissionsAndFileType`].
    pub struct Permissions: u32 {
        /// Others: execute permission (`0o001`).
        const OTHER_EXEC                        = 1 << 0;
        /// Others: write permission (`0o002`).
        const OTHER_WRITE                       = 1 << 1;
        /// Others: read permission (`0o004`).
        const OTHER_READ                        = 1 << 2;

        /// Group: execute permission (`0o010`).
        const GROUP_EXEC                        = 1 << 0 << 3;
        /// Group: write permission (`0o020`).
        const GROUP_WRITE                       = 1 << 1 << 3;
        /// Group: read permission (`0o040`).
        const GROUP_READ                        = 1 << 2 << 3;

        /// Owner: execute permission (`0o100`).
        const OWNER_EXEC                        = 1 << 0 << 6;
        /// Owner: write permission (`0o200`).
        const OWNER_WRITE                       = 1 << 1 << 6;
        /// Owner: read permission (`0o400`).
        const OWNER_READ                        = 1 << 2 << 6;
    }
}

impl Permissions {
    const MASK: u32 = !FileType::MASK;
    /// Returns the permission bits of the classic `0o755` mode (`rwxr-xr-x`).
    ///
    /// Handy as the [`Attributes::property`] value for a directory or an
    /// executable file passed to [`super::Handle::mkdir`] or
    /// [`super::Handle::open_file`].
    pub fn p0755() -> Self {
        Self::from_bits_retain(0o755)
    }
}

#[derive(Debug, Clone, Copy)]
/// File system statistics returned by the `statvfs@openssh.com` and `fstatvfs@openssh.com` extensions.
///
/// The layout follows POSIX `statvfs(3)`; the eleven `u64` fields are sent
/// on the wire in declaration order.
pub struct Statvfs {
    /// Fundamental file system block size in bytes (`f_bsize`).
    pub bsize: u64,
    /// Fragment size in bytes — the unit used for block accounting (`f_frsize`).
    pub frsize: u64,
    /// Total number of blocks in the file system, in units of `frsize` (`f_blocks`).
    pub blocks: u64,
    /// Total number of free blocks (`f_bfree`).
    pub bfree: u64,
    /// Free blocks available to unprivileged users (`f_bavail`).
    pub bavail: u64,
    /// Total number of file nodes, i.e. inodes (`f_files`).
    pub files: u64,
    /// Free file nodes (`f_ffree`).
    pub ffree: u64,
    /// Free file nodes available to unprivileged users (`f_favail`).
    pub favail: u64,
    /// File system identifier (`f_fsid`).
    pub fsid: u64,
    /// Bit mask of file system flags: a combination of [`Statvfs::FLAG_RDONLY`] and [`Statvfs::FLAG_NOSUID`].
    pub flag: u64,
    /// Maximum length of a file name, in bytes (`f_namemax`).
    pub namemax: u64,
}

impl Statvfs {
    /// [`Statvfs::flag`] bit stating that the file system is mounted read-only (`ST_RDONLY`).
    pub const FLAG_RDONLY: u64 = 0x1;
    /// [`Statvfs::flag`] bit stating that the file system disallows set-user-ID and set-group-ID execution (`ST_NOSUID`).
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
/// The type of a file, as encoded in the high bits of an SFTP permission field.
///
/// The discriminants are the POSIX `S_IFMT` mode bits (e.g. `S_IFDIR` for
/// [`FileType::Directory`]).
pub enum FileType {
    /// A directory (`S_IFDIR`).
    Directory = 0o40000,
    /// A character special device, e.g. a terminal or `/dev/null` (`S_IFCHR`).
    CharacterDevice = 0o20000,
    /// A block special device, e.g. a disk (`S_IFBLK`).
    BlockDevice = 0o60000,
    /// A regular file (`S_IFREG`).
    RegularFile = 0o100000,
    /// A FIFO, i.e. named pipe (`S_IFIFO`).
    FIFO = 0o10000,
    /// A symbolic link (`S_IFLNK`).
    SymbolicLink = 0o120000,
    /// A socket (`S_IFSOCK`).
    Socket = 0o140000,
}

impl FileType {
    const MASK: u32 = 0o170000;
    /// Returns `true` if this is [`FileType::Directory`].
    pub fn is_directory(&self) -> bool {
        matches!(self, Self::Directory)
    }

    /// Returns `true` if this is [`FileType::CharacterDevice`].
    pub fn is_character_device(&self) -> bool {
        matches!(self, Self::CharacterDevice)
    }

    /// Returns `true` if this is [`FileType::BlockDevice`].
    pub fn is_block_device(&self) -> bool {
        matches!(self, Self::BlockDevice)
    }

    /// Returns `true` if this is [`FileType::RegularFile`].
    pub fn is_regular_file(&self) -> bool {
        matches!(self, Self::RegularFile)
    }

    /// Returns `true` if this is [`FileType::FIFO`].
    pub fn is_fifo(&self) -> bool {
        matches!(self, Self::FIFO)
    }

    /// Returns `true` if this is [`FileType::SymbolicLink`].
    pub fn is_symbolic_link(&self) -> bool {
        matches!(self, Self::SymbolicLink)
    }

    /// Returns `true` if this is [`FileType::Socket`].
    pub fn is_socket(&self) -> bool {
        matches!(self, Self::Socket)
    }
}

#[derive(Debug, Clone, Copy)]
/// Protocol limits reported by the `limits@openssh.com` OpenSSH extension.
///
/// Clients must stay within these limits or the server may drop the
/// connection.
pub struct Limits {
    /// Maximum length of a whole SFTP packet accepted by the server, in bytes.
    pub max_packet_len: u64,
    /// Maximum number of bytes a single `SSH_FXP_READ` may request.
    pub max_read_len: u64,
    /// Maximum number of bytes a single `SSH_FXP_WRITE` may carry.
    pub max_write_len: u64,
    /// Maximum number of file and directory handles that may be open at once; `0` means no explicit limit.
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
/// Status codes carried by `SSH_FXP_STATUS` replies (the `SSH_FX_*` values of draft-ietf-secsh-filexfer-01).
///
/// Each variant maps to the corresponding [`super::Error`] variant when a
/// request fails; [`Status::OK`] marks success.
pub enum Status {
    /// `SSH_FX_OK` (0): the request completed successfully.
    OK = SSH_FX_OK,
    /// `SSH_FX_EOF` (1): no more data will be returned (end of file or directory).
    Eof = SSH_FX_EOF,
    /// `SSH_FX_NO_SUCH_FILE` (2): the referenced file does not exist.
    NoSuchFile = SSH_FX_NO_SUCH_FILE,
    /// `SSH_FX_PERMISSION_DENIED` (3): access to the file is denied.
    PermissionDenied = SSH_FX_PERMISSION_DENIED,
    /// `SSH_FX_FAILURE` (4): a nonspecific error with no more precise code.
    Failure = SSH_FX_FAILURE,
    /// `SSH_FX_BAD_MESSAGE` (5): the request was malformed or could not be processed.
    BadMessage = SSH_FX_BAD_MESSAGE,
    /// `SSH_FX_NO_CONNECTION` (6): there is no connection to the server.
    NoConnection = SSH_FX_NO_CONNECTION,
    /// `SSH_FX_CONNECTION_LOST` (7): the connection was lost during the operation.
    ConnectionLost = SSH_FX_CONNECTION_LOST,
    /// `SSH_FX_OP_UNSUPPORTED` (8): the operation or extension is not supported.
    OpUnsupported = SSH_FX_OP_UNSUPPORTED,
}

impl Status {
    pub(super) fn to_error(self, msg: String) -> error::Error {
        match self {
            Status::OK => super::UnexpectedResponseSnafu.build().into(),
            Status::Eof => super::UnexpectedEofSnafu { msg }.build().into(),
            Status::NoSuchFile => super::NoSuchFileSnafu { msg }.build().into(),
            Status::PermissionDenied => super::PermissionDeniedSnafu { msg }.build().into(),
            Status::Failure => super::FailureSnafu { msg }.build().into(),
            Status::BadMessage => super::BadMessageSnafu { msg }.build().into(),
            Status::NoConnection => super::NoConnectionSnafu { msg }.build().into(),
            Status::ConnectionLost => super::ConnectionLostSnafu { msg }.build().into(),
            Status::OpUnsupported => super::OpUnsupportedSnafu { msg }.build().into(),
        }
    }
    pub(super) fn to_result(self, msg: String) -> error::Result<()> {
        match self {
            Status::OK => Ok(()),
            Status::Eof => Err(super::UnexpectedEofSnafu { msg }.build().into()),
            Status::NoSuchFile => Err(super::NoSuchFileSnafu { msg }.build().into()),
            Status::PermissionDenied => Err(super::PermissionDeniedSnafu { msg }.build().into()),
            Status::Failure => Err(super::FailureSnafu { msg }.build().into()),
            Status::BadMessage => Err(super::BadMessageSnafu { msg }.build().into()),
            Status::NoConnection => Err(super::NoConnectionSnafu { msg }.build().into()),
            Status::ConnectionLost => Err(super::ConnectionLostSnafu { msg }.build().into()),
            Status::OpUnsupported => Err(super::OpUnsupportedSnafu { msg }.build().into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One entry of an `SSH_FXP_NAME` reply.
///
/// Depending on the request that produced it, the entry describes a
/// directory-listing row, a symlink target, or a canonicalized path.
pub struct FileInfo {
    /// The file name of this entry.
    ///
    /// For a directory listing this is the entry's name; for
    /// [`super::Handle::readlink`] and [`super::Handle::realpath`] replies
    /// it holds the link target or the canonical path.
    pub file_name: String,
    /// The `ls -l`-style long form of the entry, as produced by the server
    /// (some servers repeat `file_name` here when nothing else is known).
    pub long_name: String,
    /// The attribute block of the entry.
    pub attributes: Attributes,
}

impl FileInfo {
    fn parse_more(data: &[u8], size: usize) -> error::Result<Vec<Self>> {
        let mut result = Vec::with_capacity(size);

        let mut consumer = Consumer::new(data);

        for _ in 0..size {
            let file_name = consumer.consume_one()?;
            let file_name = std::str::from_utf8(file_name)
                .context(ssh::ExpectStringSnafu)?
                .to_string();

            let long_name = consumer.consume_one()?;
            let long_name = std::str::from_utf8(long_name)
                .context(ssh::ExpectStringSnafu)?
                .to_string();

            let attributes = Attributes::parse(&mut consumer)?;

            result.push(Self {
                file_name,
                long_name,
                attributes,
            });
        }

        Ok(result)
    }

    // fn parse(data: &[u8]) -> error::Result<Self> {
    //     let mut consumer = Consumer::new(data);

    //     let file_name = consumer.consume_one()?;
    //     let file_name = std::str::from_utf8(file_name)
    //         .context(msg::ExpectStringSnafu)?
    //         .to_string();

    //     let long_name = consumer.consume_one()?;
    //     let long_name = std::str::from_utf8(long_name)
    //         .context(msg::ExpectStringSnafu)?
    //         .to_string();

    //     let attributes = Attributes::parse(&mut consumer)?;

    //     Ok(Self {
    //         file_name,
    //         long_name,
    //         attributes,
    //     })
    // }
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
    ExtendReply(Vec<u8>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Access and modification times, carried by the `SSH_FILEXFER_ATTR_ACMODTIME` attribute flag.
pub struct Timestamp {
    /// Last access time, in seconds since the Unix epoch.
    pub atime: u32,
    /// Last modification time, in seconds since the Unix epoch.
    pub mtime: u32,
}

impl Timestamp {
    fn new(atime: u32, mtime: u32) -> Self {
        Self { atime, mtime }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Owner and group ids, carried by the `SSH_FILEXFER_ATTR_UIDGID` attribute flag.
pub struct User {
    /// Numeric user (owner) id.
    pub uid: u32,
    /// Numeric group id.
    pub gid: u32,
}

impl User {
    fn new(uid: u32, gid: u32) -> Self {
        Self { uid, gid }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The permission field of an attribute block: POSIX permissions plus the file type packed into one `u32`.
///
/// This is the value carried under the `SSH_FILEXFER_ATTR_PERMISSIONS`
/// attribute flag.
pub struct PermissionsAndFileType {
    /// The POSIX permission bits, without the file-type bits.
    pub permissions: Permissions,
    /// The file type encoded in the same word.
    pub file_type: FileType,
}

/// Conversion from a raw `SSH_FILEXFER_ATTR_PERMISSIONS` value into its parts.
///
/// The non-file-type bits are truncated to the flags defined by
/// [`Permissions`] (any other bits, such as set-user-ID, are dropped), and
/// the `S_IFMT` bits are decoded as the [`FileType`]. If those bits match no
/// known file type, [`super::Error::UnknownFileType`] is returned.
impl TryFrom<u32> for PermissionsAndFileType {
    type Error = error::Error;

    fn try_from(value: u32) -> std::result::Result<Self, Self::Error> {
        Ok(Self::new(
            Permissions::from_bits_truncate(value & Permissions::MASK),
            (value & FileType::MASK)
                .try_into()
                .context(super::UnknownFileTypeSnafu)?,
        ))
    }
}

impl PermissionsAndFileType {
    /// Combines `permissions` and `file_type` into a single attribute field.
    pub fn new(permissions: Permissions, file_type: FileType) -> Self {
        Self {
            permissions,
            file_type,
        }
    }
    /// Reassembles the raw `u32` that goes on the wire: the permission bits OR the file-type bits.
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
                let status = Status::try_from(code)
                    .context(super::UnexpectedStatusSnafu { status: code })?;

                let error = consumer.consume_one().unwrap_or_default();
                let error = std::str::from_utf8(error)
                    .context(ssh::ExpectStringSnafu)?
                    .to_string();

                let language = consumer.consume_one().unwrap_or_default();
                let language = std::str::from_utf8(language)
                    .context(ssh::ExpectStringSnafu)?
                    .to_string();

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
                // let mut file_infos = Vec::with_capacity(count as usize);
                // for _ in 0..count {
                //     let file_info = FileInfo::parse(consumer.peek())?;
                //     file_infos.push(file_info);
                // }

                let file_infos = FileInfo::parse_more(consumer.peek(), count as usize)?;

                Ok(Message {
                    id,
                    payload: Payload::Name(file_infos),
                })
            }
            SSH_FXP_ATTRS => {
                let attributes = Attributes::parse(&mut consumer)?;
                Ok(Message {
                    id,
                    payload: Payload::Attributes(attributes),
                })
            }
            SSH_FXP_EXTENDED_REPLY => Ok(Message {
                id,
                payload: Payload::ExtendReply(consumer.peek().to_vec()),
            }),
            code => Err(super::UnexpectedMessageSnafu { code }.build().into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// The attribute block exchanged with SFTP requests and replies.
///
/// On the wire the block begins with a flag word, and each field corresponds
/// to one `SSH_FILEXFER_ATTR_*` flag; a field is `None` when its flag is not
/// set, meaning the value is absent from the message (and omitted when the
/// block is serialized): `size` for `SSH_FILEXFER_ATTR_SIZE`, `user` for
/// `SSH_FILEXFER_ATTR_UIDGID`, `property` for `SSH_FILEXFER_ATTR_PERMISSIONS`,
/// `time` for `SSH_FILEXFER_ATTR_ACMODTIME`, and `extend` for
/// `SSH_FILEXFER_ATTR_EXTENDED`.
pub struct Attributes {
    /// File size in bytes — present when `SSH_FILEXFER_ATTR_SIZE` is set.
    pub size: Option<u64>,
    /// Owner and group — present when `SSH_FILEXFER_ATTR_UIDGID` is set.
    pub user: Option<User>,
    /// Permissions and file type — present when `SSH_FILEXFER_ATTR_PERMISSIONS` is set.
    pub property: Option<PermissionsAndFileType>,
    /// Access and modification times — present when `SSH_FILEXFER_ATTR_ACMODTIME` is set.
    pub time: Option<Timestamp>,
    /// Extended name/value data — present when `SSH_FILEXFER_ATTR_EXTENDED` is set, typically server-specific attributes.
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

            let count = extend.len() as u32;

            producer.put_u32(count);

            for (k, v) in extend {
                producer.put_one(k);
                producer.put_one(v);
            }
        }

        producer[..4].copy_from_slice(&flags.to_be_bytes());

        producer.into_vec()
    }

    fn parse(consumer: &mut Consumer<'_>) -> error::Result<Self> {
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

                    extend.insert(
                        std::str::from_utf8(key)
                            .context(ssh::ExpectStringSnafu)?
                            .to_string(),
                        value.to_vec(),
                    );
                }
                Some(extend)
            };
        }

        Ok(Self::new(size, user, permissions, time, extend))
    }
}

#[derive(Debug, Clone)]
/// An open file on the server: the handle returned by `SSH_FXP_OPEN` plus a client-side cursor.
///
/// The cursor is bookkeeping inside this process only — the server keeps no
/// position for a handle, so [`super::Handle::read_file`] and
/// [`super::Handle::write_file`] receive an explicit `offset` on every call
/// (commonly [`File::pos`]) and maintain the cursor for convenience.
pub struct File {
    handle: Vec<u8>,
    pos: u64,
}

impl File {
    pub(super) fn new(handle: Vec<u8>) -> Self {
        Self { handle, pos: 0 }
    }

    pub(super) fn handle(&self) -> &[u8] {
        &self.handle
    }

    /// Returns the client-side cursor, in bytes from the start of the file.
    ///
    /// Pass it as the `offset` of [`super::Handle::read_file`] or
    /// [`super::Handle::write_file`] to continue where the previous call
    /// left off.
    pub fn pos(&self) -> u64 {
        self.pos
    }

    #[inline(always)]
    /// Advances the cursor by `offset` bytes without touching the server.
    pub fn forward(&mut self, offset: u64) {
        self.pos += offset;
    }

    #[inline(always)]
    /// Moves the cursor back by `offset` bytes without touching the server.
    ///
    /// `offset` must not exceed the current position: with overflow checks
    /// enabled (the default in debug builds) an underflow panics, otherwise
    /// the cursor wraps around.
    pub fn backward(&mut self, offset: u64) {
        self.pos -= offset;
    }
}

#[derive(Debug, Clone)]
/// An open directory on the server: the handle returned by `SSH_FXP_OPENDIR`.
///
/// Pass it to [`super::Handle::read_directory`] to fetch the entries in
/// batches; the server releases the handle when the [`super::Handle`] itself
/// is closed, since SFTP v3 has no request for closing a directory handle.
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

#[cfg(test)]
mod test {
    use super::*;

    /// Encodes an SSH `string`: 4-byte big-endian length + bytes.
    fn ssh_string(bytes: &[u8]) -> Vec<u8> {
        let mut out = (bytes.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(bytes);
        out
    }

    /// Every bit `Permissions` defines, or-ed together.
    fn all_permission_bits() -> Permissions {
        Permissions::OTHER_EXEC
            | Permissions::OTHER_WRITE
            | Permissions::OTHER_READ
            | Permissions::GROUP_EXEC
            | Permissions::GROUP_WRITE
            | Permissions::GROUP_READ
            | Permissions::OWNER_EXEC
            | Permissions::OWNER_WRITE
            | Permissions::OWNER_READ
    }

    fn all_flags_attributes() -> Attributes {
        let mut extend = HashMap::new();
        extend.insert("vendor@openssh.com".to_string(), b"1".to_vec());

        Attributes {
            size: Some(4096),
            user: Some(User {
                uid: 1000,
                gid: 1000,
            }),
            property: Some(PermissionsAndFileType::new(
                all_permission_bits(),
                FileType::Directory,
            )),
            time: Some(Timestamp {
                atime: 1_700_000_000,
                mtime: 1_700_000_123,
            }),
            extend: Some(extend),
        }
    }

    #[test]
    fn attributes_round_trip_with_all_flags() {
        let attrs = all_flags_attributes();
        let bytes = attrs.to_bytes();

        let mut consumer = Consumer::new(&bytes);
        let parsed = Attributes::parse(&mut consumer).unwrap();

        assert_eq!(parsed, attrs);
        assert!(consumer.is_empty());
    }

    #[test]
    fn attributes_round_trip_with_no_flags() {
        let attrs = Attributes {
            size: None,
            user: None,
            property: None,
            time: None,
            extend: None,
        };
        let bytes = attrs.to_bytes();

        // Only the flags word, all zeros.
        assert_eq!(bytes, [0, 0, 0, 0]);

        let mut consumer = Consumer::new(&bytes);
        let parsed = Attributes::parse(&mut consumer).unwrap();
        assert_eq!(parsed, attrs);
    }

    #[test]
    fn attributes_flags_only_size() {
        let attrs = Attributes {
            size: Some(1234),
            user: None,
            property: None,
            time: None,
            extend: None,
        };
        let bytes = attrs.to_bytes();

        // flags = SSH_FILEXFER_ATTR_SIZE = 0x00000001
        assert_eq!(&bytes[..4], &1u32.to_be_bytes());
        assert_eq!(&bytes[4..], &1234u64.to_be_bytes());
        assert_eq!(bytes.len(), 12);
    }

    #[test]
    fn attributes_partial_fields_round_trip() {
        let attrs = Attributes {
            size: None,
            user: Some(User { uid: 0, gid: 0 }),
            property: None,
            time: Some(Timestamp { atime: 1, mtime: 2 }),
            extend: None,
        };

        let bytes = attrs.to_bytes();
        let mut consumer = Consumer::new(&bytes);
        let parsed = Attributes::parse(&mut consumer).unwrap();

        assert_eq!(parsed, attrs);
        assert_eq!(parsed.user.unwrap().uid, 0);
        assert_eq!(parsed.time.unwrap().mtime, 2);
    }

    #[test]
    fn attributes_truncated_input_fails() {
        // flags say SIZE is present but no size bytes follow
        let bytes = 1u32.to_be_bytes();
        let mut consumer = Consumer::new(&bytes);
        assert!(Attributes::parse(&mut consumer).is_err());
    }

    #[test]
    fn permissions_and_file_type_bits_round_trip() {
        // Use only bits Permissions defines, so the round trip is lossless.
        let combo = PermissionsAndFileType::new(all_permission_bits(), FileType::Directory);
        let bits = combo.bits();

        // The S_IFMT bits are present for the directory type.
        assert_eq!(bits & 0o170000, FileType::Directory as u32);

        let back = PermissionsAndFileType::try_from(bits).unwrap();
        assert_eq!(back.file_type, FileType::Directory);
        assert_eq!(back.permissions, all_permission_bits());
        assert_eq!(back.bits(), bits);
    }

    #[test]
    fn permissions_p0755_sets_the_octal_mode() {
        // p0755 retains the raw 0o755 word, which is what goes on the wire.
        assert_eq!(Permissions::p0755().bits(), 0o755);
    }

    #[test]
    fn permissions_from_raw_value_drops_file_type_and_unknown_bits() {
        // set-user-ID (0o4000) is not part of Permissions and must be dropped,
        // as must the S_IFMT file-type bits.
        let bits = 0o40755 | 0o4000;
        let combo = PermissionsAndFileType::try_from(bits).unwrap();

        assert_eq!(combo.permissions, Permissions::from_bits_truncate(bits));
        assert_eq!(combo.file_type, FileType::Directory);
        // setuid never survives the round trip
        assert_eq!(combo.bits() & 0o4000, 0);
    }

    /// Regression test: every permission constant must equal its POSIX value.
    ///
    /// SFTP does not define these bits itself — draft-ietf-secsh-filexfer-01
    /// section 5 defers to POSIX — so the only way to get them wrong is to
    /// shift by the wrong amount (an octal digit is 3 bits, not 4).

    #[test]
    fn permissions_group_and_owner_bits_match_posix() {
        assert_eq!(Permissions::OTHER_EXEC.bits(), 0o001);
        assert_eq!(Permissions::OTHER_WRITE.bits(), 0o002);
        assert_eq!(Permissions::OTHER_READ.bits(), 0o004);
        assert_eq!(Permissions::GROUP_EXEC.bits(), 0o010);
        assert_eq!(Permissions::GROUP_WRITE.bits(), 0o020);
        assert_eq!(Permissions::GROUP_READ.bits(), 0o040);
        assert_eq!(Permissions::OWNER_EXEC.bits(), 0o100);
        assert_eq!(Permissions::OWNER_WRITE.bits(), 0o200);
        assert_eq!(Permissions::OWNER_READ.bits(), 0o400);

        // The three classes must tile the low nine bits with no overlap.
        let all = Permissions::OTHER_EXEC
            | Permissions::OTHER_WRITE
            | Permissions::OTHER_READ
            | Permissions::GROUP_EXEC
            | Permissions::GROUP_WRITE
            | Permissions::GROUP_READ
            | Permissions::OWNER_EXEC
            | Permissions::OWNER_WRITE
            | Permissions::OWNER_READ;
        assert_eq!(all.bits(), 0o777, "the rwx classes must be exactly 0o777");
    }

    /// Every POSIX mode a server can plausibly send must survive
    /// `try_from` -> `bits()` unchanged.
    ///
    /// This is the read path: `stat`/`readdir` hand the result straight to
    /// callers, who may write it back with `set_stat`. A lossy round trip
    /// silently changes permissions on the server (e.g. `0o644` -> `0o444`).
    #[test]
    fn permissions_round_trip_is_lossless_for_real_modes() {
        // Regular file, directory, and symlink with a spread of rwx modes.
        for type_bits in [0o100000u32, 0o40000, 0o120000] {
            for rwx in [
                0o000, 0o444, 0o644, 0o664, 0o666, 0o755, 0o700, 0o777, 0o500, 0o111, 0o420,
            ] {
                let mode = type_bits | rwx;
                let back = PermissionsAndFileType::try_from(mode).unwrap().bits();
                assert_eq!(back, mode, "round trip of {mode:#07o}");
            }
        }
    }

    /// `p0755()` is the value used by `mkdir`/`open_file` and must stay
    /// POSIX-correct on the wire.
    #[test]
    fn p0755_is_the_posix_directory_mode() {
        let combo = PermissionsAndFileType::new(Permissions::p0755(), FileType::Directory);
        assert_eq!(combo.bits(), 0o40755);
        // And it must survive being parsed back.
        assert_eq!(
            PermissionsAndFileType::try_from(combo.bits())
                .unwrap()
                .bits(),
            0o40755
        );
    }

    /// Composing a mode from the named constants must produce the POSIX
    /// value, since that is how callers build permissions without knowing
    /// octal literals.
    #[test]
    fn mode_built_from_constants_matches_posix() {
        // rw-r--r--
        let rw_r__r__ = Permissions::OWNER_READ
            | Permissions::OWNER_WRITE
            | Permissions::GROUP_READ
            | Permissions::OTHER_READ;
        assert_eq!(rw_r__r__.bits(), 0o644);

        // rwxr-xr-x
        let rwxr_xr_x = Permissions::OWNER_READ
            | Permissions::OWNER_WRITE
            | Permissions::OWNER_EXEC
            | Permissions::GROUP_READ
            | Permissions::GROUP_EXEC
            | Permissions::OTHER_READ
            | Permissions::OTHER_EXEC;
        assert_eq!(rwxr_xr_x.bits(), 0o755);
    }

    /// Set-user-ID / set-group-ID / sticky are not modelled by
    /// [`Permissions`], so `try_from` drops them — this is the documented
    /// behaviour, asserted here so a future change is a deliberate one.
    #[test]
    fn special_bits_are_dropped_by_try_from() {
        let combo = PermissionsAndFileType::try_from(0o104755).unwrap();
        assert_eq!(combo.bits(), 0o104755 & !0o7000, "setuid must be dropped");
        assert_eq!(combo.file_type, FileType::RegularFile);

        // A file with *only* special bits set still round-trips its rwx part.
        let combo = PermissionsAndFileType::try_from(0o40000 | 0o4755).unwrap();
        assert_eq!(combo.permissions.bits(), 0o755);
    }

    #[test]
    fn permissions_and_file_type_unknown_type_fails() {
        // S_IFMT bits set to a value that maps to no file type
        let err = PermissionsAndFileType::try_from(0o110000).unwrap_err();
        assert!(
            err.to_string().contains("file type")
                || err.to_string().contains("Unknown")
                || err.to_string().contains("unknown"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn file_type_predicates() {
        assert!(FileType::Directory.is_directory());
        assert!(!FileType::Directory.is_regular_file());

        assert!(FileType::RegularFile.is_regular_file());
        assert!(!FileType::RegularFile.is_directory());

        assert!(FileType::SymbolicLink.is_symbolic_link());
        assert!(FileType::CharacterDevice.is_character_device());
        assert!(FileType::BlockDevice.is_block_device());
        assert!(FileType::FIFO.is_fifo());
        assert!(FileType::Socket.is_socket());

        assert!(!FileType::RegularFile.is_symbolic_link());
    }

    #[test]
    fn open_flags_bits() {
        let flags = OpenFlags::READ | OpenFlags::CREAT | OpenFlags::TRUNC;
        assert!(flags.contains(OpenFlags::READ));
        assert!(flags.contains(OpenFlags::CREAT));
        assert!(flags.contains(OpenFlags::TRUNC));
        assert!(!flags.contains(OpenFlags::EXCL));

        // Values come straight from the SSH_FXF_* constants.
        assert_eq!(OpenFlags::READ.bits(), SSH_FXF_READ);
        assert_eq!(OpenFlags::WRITE.bits(), SSH_FXF_WRITE);
        assert_eq!(OpenFlags::APPEND.bits(), SSH_FXF_APPEND);
        assert_eq!(OpenFlags::EXCL.bits(), SSH_FXF_EXCL);
    }

    #[test]
    fn status_codes_match_protocol_constants() {
        assert_eq!(Status::OK as u32, SSH_FX_OK);
        assert_eq!(Status::Eof as u32, SSH_FX_EOF);
        assert_eq!(Status::NoSuchFile as u32, SSH_FX_NO_SUCH_FILE);
        assert_eq!(Status::PermissionDenied as u32, SSH_FX_PERMISSION_DENIED);
        assert_eq!(Status::Failure as u32, SSH_FX_FAILURE);
        assert_eq!(Status::BadMessage as u32, SSH_FX_BAD_MESSAGE);
        assert_eq!(Status::NoConnection as u32, SSH_FX_NO_CONNECTION);
        assert_eq!(Status::ConnectionLost as u32, SSH_FX_CONNECTION_LOST);
        assert_eq!(Status::OpUnsupported as u32, SSH_FX_OP_UNSUPPORTED);
    }

    #[test]
    fn status_ok_is_ok_but_others_are_errors() {
        assert!(Status::OK.to_result("fine".into()).is_ok());
        assert!(Status::Eof.to_result("end".into()).is_err());
        assert!(Status::NoSuchFile.to_result("nope".into()).is_err());
        assert!(Status::OpUnsupported.to_result("nope".into()).is_err());
    }

    #[test]
    fn status_to_result_error_messages_carry_server_text() {
        let err = Status::PermissionDenied
            .to_result("denied".into())
            .unwrap_err();
        assert!(err.to_string().contains("denied"), "got: {err}");
    }

    /// Unwraps the two `From` layers an SFTP error passes through:
    /// `sftp::Error` -> `session::Error` -> `error::Error`.
    fn unwrap_sftp(err: crate::error::Error) -> crate::session::sftp::Error {
        let crate::error::Error::SessionError { source } = err else {
            panic!("expected SessionError, got {err:?}");
        };
        let crate::session::Error::SSHFileTransferProtocolError { source } = source else {
            panic!("expected SSHFileTransferProtocolError, got {source:?}");
        };
        source
    }

    #[test]
    fn status_to_error_matches_expected_variants() {
        use crate::session::Error as SessionError;
        use crate::session::sftp::Error as SftpError;

        let err = unwrap_sftp(Status::NoSuchFile.to_error("gone".into()));
        let SftpError::NoSuchFile { msg } = err else {
            panic!("expected NoSuchFile, got {err:?}");
        };
        assert_eq!(msg, "gone");

        let err = unwrap_sftp(Status::Eof.to_error("eof".into()));
        assert!(matches!(err, SftpError::UnexpectedEof { .. }));

        // Status::OK maps to UnexpectedResponse rather than being dropped.
        let err = unwrap_sftp(Status::OK.to_error("ok".into()));
        assert!(matches!(err, SftpError::UnexpectedResponse { .. }));

        // The outermost wrapper stays intact for callers matching on it.
        let outer = Status::Failure.to_error("nope".into());
        assert!(matches!(
            outer,
            crate::error::Error::SessionError {
                source: SessionError::SSHFileTransferProtocolError { .. }
            }
        ));
    }

    #[test]
    fn file_cursor_moves_forward_and_backward() {
        let mut file = File::new(b"h".to_vec());
        assert_eq!(file.pos(), 0);

        file.forward(100);
        assert_eq!(file.pos(), 100);

        file.forward(50);
        assert_eq!(file.pos(), 150);

        file.backward(30);
        assert_eq!(file.pos(), 120);

        file.backward(120);
        assert_eq!(file.pos(), 0);
    }

    #[test]
    fn parse_status_message() {
        let mut data = vec![SSH_FXP_STATUS];
        data.extend_from_slice(&7u32.to_be_bytes()); // id
        data.extend_from_slice(&SSH_FX_NO_SUCH_FILE.to_be_bytes());
        data.extend_from_slice(&ssh_string(b"no such file"));
        data.extend_from_slice(&ssh_string(b"en"));

        let message = Message::parse(&data).unwrap();
        assert_eq!(message.id, 7);
        let Payload::Status {
            status,
            error,
            language,
        } = message.payload
        else {
            panic!("expected Status, got {:?}", message.payload);
        };
        assert_eq!(status, Status::NoSuchFile);
        assert_eq!(error, "no such file");
        assert_eq!(language, "en");
    }

    #[test]
    fn parse_status_with_unknown_code_fails() {
        let mut data = vec![SSH_FXP_STATUS];
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend_from_slice(&9999u32.to_be_bytes());
        data.extend_from_slice(&ssh_string(b"??"));
        data.extend_from_slice(&ssh_string(b""));

        assert!(Message::parse(&data).is_err());
    }

    #[test]
    fn parse_handle_message() {
        let mut data = vec![SSH_FXP_HANDLE];
        data.extend_from_slice(&3u32.to_be_bytes());
        data.extend_from_slice(&ssh_string(b"handle-1"));

        let message = Message::parse(&data).unwrap();
        assert_eq!(message.id, 3);
        let Payload::Handle(handle) = message.payload else {
            panic!("expected Handle, got {:?}", message.payload);
        };
        assert_eq!(handle, b"handle-1");
    }

    #[test]
    fn parse_data_message() {
        let mut data = vec![SSH_FXP_DATA];
        data.extend_from_slice(&9u32.to_be_bytes());
        data.extend_from_slice(&ssh_string(b"contents"));

        let message = Message::parse(&data).unwrap();
        assert_eq!(message.id, 9);
        let Payload::Data(payload) = message.payload else {
            panic!("expected Data, got {:?}", message.payload);
        };
        assert_eq!(payload, b"contents");
    }

    #[test]
    fn parse_name_message_with_two_entries() {
        let attrs = Attributes {
            size: None,
            user: None,
            property: Some(PermissionsAndFileType::new(
                all_permission_bits(),
                FileType::Directory,
            )),
            time: None,
            extend: None,
        };

        let mut data = vec![SSH_FXP_NAME];
        data.extend_from_slice(&5u32.to_be_bytes()); // id
        data.extend_from_slice(&2u32.to_be_bytes()); // count
        for name in ["a", "b"] {
            data.extend_from_slice(&ssh_string(name.as_bytes()));
            data.extend_from_slice(&ssh_string(format!("drwxr-xr-x {}", name).as_bytes()));
            data.extend_from_slice(&attrs.to_bytes());
        }

        let message = Message::parse(&data).unwrap();
        assert_eq!(message.id, 5);
        let Payload::Name(entries) = message.payload else {
            panic!("expected Name, got {:?}", message.payload);
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].file_name, "a");
        assert_eq!(entries[1].file_name, "b");
        assert_eq!(entries[0].long_name, "drwxr-xr-x a");
        assert_eq!(entries[0].attributes, attrs);
        assert_eq!(entries[1].attributes, attrs);
    }

    #[test]
    fn parse_attrs_message() {
        let attrs = Attributes {
            size: Some(77),
            user: None,
            property: None,
            time: None,
            extend: None,
        };

        let mut data = vec![SSH_FXP_ATTRS];
        data.extend_from_slice(&2u32.to_be_bytes());
        data.extend_from_slice(&attrs.to_bytes());

        let message = Message::parse(&data).unwrap();
        assert_eq!(message.id, 2);
        let Payload::Attributes(parsed) = message.payload else {
            panic!("expected Attributes, got {:?}", message.payload);
        };
        assert_eq!(parsed, attrs);
    }

    #[test]
    fn parse_extended_reply_keeps_raw_bytes() {
        let mut data = vec![SSH_FXP_EXTENDED_REPLY];
        data.extend_from_slice(&4u32.to_be_bytes());
        data.extend_from_slice(&42u64.to_be_bytes());

        let message = Message::parse(&data).unwrap();
        assert_eq!(message.id, 4);
        let Payload::ExtendReply(raw) = message.payload else {
            panic!("expected ExtendReply, got {:?}", message.payload);
        };
        assert_eq!(raw, 42u64.to_be_bytes());
    }

    #[test]
    fn parse_unknown_message_type_fails() {
        let mut data = vec![SSH_FXP_INIT]; // a type this parser never handles
        data.extend_from_slice(&1u32.to_be_bytes());

        assert!(Message::parse(&data).is_err());
    }

    #[test]
    fn parse_missing_id_fails() {
        assert!(Message::parse(&[SSH_FXP_STATUS]).is_err());
    }

    #[test]
    fn statvfs_parses_eleven_u64_fields() {
        let mut data = Vec::new();
        for value in 1u64..=11 {
            data.extend_from_slice(&value.to_be_bytes());
        }

        let stat = Statvfs::parse(&data).unwrap();
        assert_eq!(stat.bsize, 1);
        assert_eq!(stat.frsize, 2);
        assert_eq!(stat.blocks, 3);
        assert_eq!(stat.bfree, 4);
        assert_eq!(stat.bavail, 5);
        assert_eq!(stat.files, 6);
        assert_eq!(stat.ffree, 7);
        assert_eq!(stat.favail, 8);
        assert_eq!(stat.fsid, 9);
        assert_eq!(stat.flag, 10);
        assert_eq!(stat.namemax, 11);
    }

    #[test]
    fn statvfs_flag_constants() {
        assert_eq!(Statvfs::FLAG_RDONLY, 0x1);
        assert_eq!(Statvfs::FLAG_NOSUID, 0x2);
    }

    #[test]
    fn limits_parses_four_u64_fields() {
        let mut data = Vec::new();
        for value in [1u64, 2, 3, 4] {
            data.extend_from_slice(&value.to_be_bytes());
        }

        let limits = Limits::parse(&data).unwrap();
        assert_eq!(limits.max_packet_len, 1);
        assert_eq!(limits.max_read_len, 2);
        assert_eq!(limits.max_write_len, 3);
        assert_eq!(limits.max_open_handles, 4);
    }

    #[test]
    fn statvfs_truncated_input_fails() {
        let data = [0u8; 8]; // one u64, needs eleven
        assert!(Statvfs::parse(&data).is_err());
    }
}
