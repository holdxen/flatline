use std::collections::HashMap;
use snafu::{ResultExt, Snafu};
use crate::session::channel::{BufferChannel, Channel};
use crate::error;
use crate::ssh::buffer::Consumer;
use crate::ssh::protocol::sftp::*;
use crate::ssh::msg;
use crate::ssh::buffer::*;
use crate::ssh::protocol::SFTPExtension;

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
    fn parse(data: &[u8]) -> error::Result<Self> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    fn parse(data: &[u8]) -> error::Result<Self> {
        let mut consumer = Consumer::new(data);

        Ok(Self {
            max_packet_len: consumer.consume_u64()?,
            max_read_len: consumer.consume_u64()?,
            max_write_len: consumer.consume_u64()?,
            max_open_handles: consumer.consume_u64()?,
        })
    }
}

#[derive(Debug, PartialEq)]
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


pub enum Payload<'a> {
    Status {
        code: Status,
        error: &'a str,
        language: & 'a str,
    },
    Handle(&'a [u8]),
    Data(&'a [u8]),
    Name
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub atime: u32,
    pub mtime: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Property {
    pub permissions: Permissions,
    pub file_type: FileType,
}


pub struct Message<'a> {
    id: u32,
    payload: Payload<'a>,
}

#[derive(Debug)]
pub struct Handle {
    channel: BufferChannel,
    extensions: HashMap<String, Vec<u8>>,
    request_id: u32,
}

impl Handle {
    pub(super) async fn handshake(channel: Channel) ->  error::Result<Self> {
        let mut channel = BufferChannel::new(channel);


        let buffer = make_buffer! {
            u8: SSH_FXP_INIT,
            u32: VERSION,
        };


        channel.send(&buffer[..]).await?;

        channel.flush().await?;



        let len = channel.fill_exact(4).await?;

        let len = u32::from_be_bytes(len.try_into().unwrap());

        let data = channel.fill_exact(len as usize + 4).await?;



        let mut consumer = Consumer::new(&data[4..]);

        if consumer.consume_u8()? != SSH_FXP_VERSION {
            return Err(super::UnexpectedMessageSnafu {
                detail: "Unexpected sft message"
            }.build().into());
        }

        let version = consumer.consume_u32()?;
        if version != VERSION {
            tracing::warn!("SFTP version mismatch");
        }


        let mut extensions = HashMap::new();
        while !consumer.is_empty() {
            let k = consumer.consume_one()?;
            let k = std::str::from_utf8(k).context(msg::ExpectStringSnafu)?;
            let v = consumer.consume_one()?;

            extensions.insert(k.to_string(), v.to_vec());
        }


        channel.consumer_read_buffer(4 + len as usize);

        Ok(Self {
            channel,
            extensions,
            request_id: 0,
        })
    }

    fn next_request_id(&mut self) -> u32 {
        self.request_id = self.request_id.wrapping_add(1);
        self.request_id
    }

    fn supported(&self, extension: SFTPExtension) -> bool {
        matches!(self.extensions.get(extension.key), Some(v) if v == extension.value)
    }

    fn is_posix_rename_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_POSIX_RENAME)
    }
    fn is_statvfs_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_STATVFS)
    }

    fn is_fstatvfs_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_FSTATVFS)
    }

    fn is_hardlink_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_HARDLINK)
    }

    fn is_fsync_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_FSYNC)
    }

    fn is_lsetstat_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_LSETSTAT)
    }

    fn is_limits_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_LIMITS)
    }

    fn is_expand_path_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_EXPAND_PATH)
    }

    fn is_copy_data_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_COPY_DATA)
    }

    fn is_home_directory_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_HOME_DIRECTORY)
    }

    fn is_users_groups_by_id_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_USERS_GROUPS_BY_ID)
    }




    fn statvfs(&self) {}

}


#[cfg(test)]
mod test {
    use crate::test::*;

    #[tokio::test]
    async fn test_handshake() -> anyhow::Result<()> {
        tracing_subscriber::fmt::init();
        let config = Config::load().await?;

        let session = config.open_session().await?;
        session.request_authentication().await?;
        config.authenticate_password(&session).await?;

        tracing::info!("Authentication complete");

        let handle = session.sftp_open_default().await?;

        tracing::info!("Opened sftp handle: {:?}", handle);

        Ok(())
    }
}