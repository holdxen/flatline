use crate::error;
use crate::session::channel::{BufferChannel, Channel};
use crate::ssh::buffer::Consumer;
use crate::ssh::buffer::*;
use crate::ssh::msg;
use crate::ssh::protocol::SFTPExtension;
use crate::ssh::protocol::sftp::*;
use snafu::ResultExt;
use std::collections::HashMap;

mod types;

pub use types::*;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("Unexpected EOF: {}", msg))]
    UnexpectedEof {
        msg: String,
    },
    #[snafu(display("No such file: {}", msg))]
    NoSuchFile {
        msg: String,
    },
    #[snafu(display("Permission denied: {}", msg))]
    PermissionDenied {
        msg: String,
    },
    #[snafu(display("Failure: {}", msg))]
    Failure {
        msg: String,
    },
    #[snafu(display("Bad message: {}", msg))]
    BadMessage {
        msg: String,
    },
    #[snafu(display("No connection: {}", msg))]
    NoConnection {
        msg: String,
    },
    #[snafu(display("Connection lost: {}", msg))]
    ConnectionLost {
        msg: String,
    },
    #[snafu(display("Operation not supported: {}", msg))]
    OpUnsupported {
        msg: String,
    },
    #[snafu(display("Unexpected message code: {}", code))]
    UnexpectedMessage {
        code: u8,
    },
    UnknownFileType {
        source: num_enum::TryFromPrimitiveError<types::FileType>,
    },
    UnexpectedStatus {
        source: num_enum::TryFromPrimitiveError<types::Status>,
        status: u32,
    },
    MismatchResponse {
        expected: u32,
        got: u32,
    },
    UnexpectedResponse {},
}

impl Error {
    pub fn is_broken(&self) -> bool {
        matches!(
            self,
            Error::UnexpectedEof { .. }
                | Error::NoSuchFile { .. }
                | Error::PermissionDenied { .. }
                | Error::Failure { .. }
                | Error::BadMessage { .. }
                | Error::NoConnection { .. }
                | Error::ConnectionLost { .. }
                | Error::OpUnsupported { .. }
        )
    }
}

impl From<Error> for error::Error {
    fn from(value: Error) -> Self {
        let e = super::Error::from(value);
        e.into()
    }
}


#[derive(Debug)]
pub struct Handle {
    channel: BufferChannel,
    extensions: HashMap<String, Vec<u8>>,
    request_id: u32,
}

impl Handle {
    pub(super) async fn handshake(channel: Channel) -> error::Result<Self> {
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

        let code = consumer.consume_u8()?;
        if code != SSH_FXP_VERSION {
            return Err(UnexpectedMessageSnafu { code }.build().into());
        }

        let version = consumer.consume_u32()?;
        if version != VERSION {
            tracing::warn!("SFTP version mismatch: mine={}, received={}", version, VERSION);
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

    pub async fn close(self) -> error::Result<()> {
        self.channel.close().await
    }

    fn next_request_id(&mut self) -> u32 {
        self.request_id = self.request_id.wrapping_add(1);
        self.request_id
    }

    fn supported(&self, extension: SFTPExtension) -> bool {
        matches!(self.extensions.get(extension.key), Some(v) if v == extension.value)
    }

    pub fn is_posix_rename_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_POSIX_RENAME)
    }
    pub async fn posix_rename(&mut self, oldpath: &str, newpath: &str) -> error::Result<()> {
        debug_assert!(
            self.is_posix_rename_supported(),
            "Server doesn't support posix rename"
        );

        self.handle(|request_id| {
            make_buffer! {
                u8: SSH_FXP_EXTENDED,
                u32: request_id,
                one: OPENSSH_SFTP_EXT_POSIX_RENAME.key,
                one: oldpath,
                one: newpath,
            }.into_vec()
        }, |payload| {

            match payload {
                Payload::Status { status, error, .. } => {
                    status.to_result(error)
                },
                _ => Err(UnexpectedResponseSnafu.build().into())
            }

        }).await
    }


    pub fn is_statvfs_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_STATVFS)
    }
    pub async fn statvfs(&mut self, path: &str) -> error::Result<Statvfs> {
        debug_assert!(self.is_statvfs_supported(), "Server doesn't support statvfs");
        self.handle(|request_id| {
            make_buffer! {
                u8: SSH_FXP_EXTENDED,
                u32: request_id,
                one: OPENSSH_SFTP_EXT_STATVFS.key,
                one: path,
            }.into_vec()
        }, |payload| {
            match payload {
                Payload::Status { status, error, .. } => {
                    Err(status.to_error(error))
                },
                Payload::ExtendReply(data) => {
                    Statvfs::parse(&data)
                },
                _ => Err(UnexpectedResponseSnafu.build().into())
            }
        }).await
    }

    pub fn is_fstatvfs_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_FSTATVFS)
    }

    pub async fn fstatvfs(&mut self, file: &File) -> error::Result<Statvfs> {
        debug_assert!(self.is_fstatvfs_supported(), "Server doesn't support fstatvfs");

        self.handle(|request_id| {
            make_buffer! {
                u8: SSH_FXP_EXTENDED,
                u32: request_id,
                one: OPENSSH_SFTP_EXT_FSTATVFS.key,
                one: file.handle(),
            }.into_vec()
        }, |payload| {
            match payload {
                Payload::Status { status, error, .. } => {
                    Err(status.to_error(error))
                },
                Payload::ExtendReply(data) => Statvfs::parse(&data),
                _ => Err(UnexpectedResponseSnafu.build().into())
            }
        }).await
    }

    pub fn is_hardlink_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_HARDLINK)
    }

    pub async fn hardlink(&mut self, oldpath: &str, newpath: &str) -> error::Result<()> {
        debug_assert!(self.is_hardlink_supported(), "Server doesn't support hardlink");

        self.handle(|request_id| {
            make_buffer! {
                u8: SSH_FXP_EXTENDED,
                u32: request_id,
                one: OPENSSH_SFTP_EXT_HARDLINK.key,
                one: oldpath,
                one: newpath,
            }.into_vec()
        },  |payload| {

            match payload {
                Payload::Status { status, error, .. } => {
                    status.to_result(error)
                },
                _ => Err(UnexpectedResponseSnafu.build().into())
            }
        }).await
    }

    pub fn is_fsync_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_FSYNC)
    }

    pub async fn fsync(&mut self, file: &File) -> error::Result<()> {
        debug_assert!(self.is_fsync_supported(), "Server doesn't support fsync");

        self.handle(|request_id| {
            make_buffer! {
                u8: SSH_FXP_EXTENDED,
                u32: request_id,
                one: OPENSSH_SFTP_EXT_FSYNC.key,
                one: file.handle(),
            }.into_vec()
        }, |payload| {
            match payload {
                Payload::Status { status, error, .. } => {
                    status.to_result(error)
                },
                _ => Err(UnexpectedResponseSnafu.build().into())
            }
        }).await
        
    }

    pub fn is_lsetstat_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_LSETSTAT)
    }


    pub async fn lsetstat(&mut self, path: &str, attrs: &Attributes) -> error::Result<()> {
        debug_assert!(self.is_lsetstat_supported(), "Server doesn't lsetstat");

        self.handle(|request_id| {
            make_buffer! {
                u8: SSH_FXP_EXTENDED,
                u32: request_id,
                one: OPENSSH_SFTP_EXT_LSETSTAT.key,
                one: path,
                bytes: attrs.to_bytes(),
            }.into_vec()
        }, |payload| {
            match payload {
                Payload::Status { status, error, .. } =>  {
                    status.to_result(error)
                },
                _ => Err(UnexpectedResponseSnafu.build().into())
            }
        }).await
    }


    pub fn is_limits_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_LIMITS)
    }

    pub async fn limits(&mut self) -> error::Result<Limits>  {
        debug_assert!(self.is_limits_supported(), "Server doesn't support limits");

        self.handle(|request_id| {
            make_buffer! {
                u8: SSH_FXP_EXTENDED,
                u32: request_id,
                one: OPENSSH_SFTP_EXT_LIMITS.key
            }.into_vec()
        }, |payload| {
            match payload {
                Payload::ExtendReply(data) => Limits::parse(&data),
                _ => Err(UnexpectedResponseSnafu.build().into())
            }
        }).await
    }

    pub fn is_expand_path_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_EXPAND_PATH)
    }

    pub async fn expand_path(&mut self, path: &str) -> error::Result<String>  {
        debug_assert!(
            self.is_expand_path_supported(),
            "Server doesn't support expand path"
        );

        self.handle(|request_id| {
            make_buffer! {
                u8: SSH_FXP_EXTENDED,
                u32: request_id,
                one: OPENSSH_SFTP_EXT_EXPAND_PATH.key,
                one: path
            }.into_vec()
        }, |payload| {
            match payload {
                Payload::Status { status, error, .. } => {
                    Err(status.to_error(error))
                },
                Payload::Name(file_infos) => {
                    if file_infos.is_empty() {
                        return Err(UnexpectedResponseSnafu {
                        }.build().into());
                    }
                    Ok(file_infos[0].file_name.clone())
                },
                _ => {
                    Err(UnexpectedResponseSnafu.build().into())
                }
            }
        }).await
    }

    pub fn is_copy_data_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_COPY_DATA)
    }

    pub async fn copy_data(&mut self, read: &mut File, len: u64, write: &mut File) -> error::Result<()>  {
        debug_assert!(self.is_copy_data_supported(), "Server doesn't support copy data");

        self.handle(|request_id| {
            make_buffer! {
            u8: SSH_FXP_EXTENDED,
            u32: request_id,
            one: OPENSSH_SFTP_EXT_COPY_DATA.key,
            one: &read.handle(),
            u64: read.pos(),
            u64: len,
            one: &write.handle(),
            u64: write.pos(),
        }.into_vec()
        }, |payload| {
            match payload {
                Payload::Status { status, error, .. } => {
                    status.to_result(error)
                },
                _ => {
                    Err(UnexpectedResponseSnafu.build().into())
                }
            }
        }).await

    }

    pub fn is_home_directory_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_HOME_DIRECTORY)
    }

    pub async fn home_directory(&mut self, username: &str) -> error::Result<String> {
        debug_assert!(
            self.is_home_directory_supported(),
            "Server doesn't support home directory"
        );

        self.handle(|request_id| {
            make_buffer! {
                u8: SSH_FXP_EXTENDED,
                u32: request_id,
                one: OPENSSH_SFTP_EXT_HOME_DIRECTORY.key,
                one: username
            }.into_vec()
        }, |payload| {
            match payload {
                Payload::Status { status, error, .. } => {
                    Err(status.to_error(error))
                },
                Payload::Name(file_infos) => {
                    if file_infos.is_empty() {
                        return Err(UnexpectedResponseSnafu {
                        }.build().into());
                    }
                    Ok(file_infos[0].file_name.clone())
                },
                _ => {
                    Err(UnexpectedResponseSnafu.build().into())
                }
            }
        }).await
    }

    pub fn is_users_groups_by_id_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_USERS_GROUPS_BY_ID)
    }
    pub async fn users_groups_by_id(&mut self, users: &[u32], groups: &[u32]) -> error::Result<(Vec<String>, Vec<String>)> {
        debug_assert!(self.is_users_groups_by_id_supported(), "Server doesn't support users-groups-by-id");
        
        self.handle(|request_id| {
            make_buffer! {
                u8: SSH_FXP_EXTENDED,
                u32: request_id,
                one: OPENSSH_SFTP_EXT_USERS_GROUPS_BY_ID.key,
                one_list_u32: users,
                one_list_u32: groups
            }.into_vec()
        }, |payload| {

            match payload {
                Payload::Status { status, error, .. } => {
                    Err(status.to_error(error))
                },
                Payload::ExtendReply(data) => {
                    let mut consumer = Consumer::new(&data);
                    let usernames = {
                        let mut consumer = Consumer::new(consumer.consume_one()?);
                        let mut usernames = Vec::with_capacity(users.len());
                        while !consumer.is_empty() {
                            let name = std::str::from_utf8(consumer.consume_one()?).context(msg::ExpectStringSnafu)?;
                            usernames.push(name.to_string());
                        }
                        usernames
                    };

                    let groupnames = {
                        let mut consumer = Consumer::new(consumer.consume_one()?);
                        let mut groupnames = Vec::with_capacity(users.len());
                        while !consumer.is_empty() {
                            let name = std::str::from_utf8(consumer.consume_one()?).context(msg::ExpectStringSnafu)?;
                            groupnames.push(name.to_string());
                        }
                        groupnames
                    };

                    Ok((usernames, groupnames))
                },
                _ => Err(UnexpectedResponseSnafu.build().into())
            }
        }).await

    }

    async fn receive_msg(&mut self, request_id: u32) -> error::Result<Message> {
        let len = self.channel.fill_exact(4).await?;
        let len = u32::from_be_bytes(len.try_into().unwrap());
        if len > 1024 * 1024 * 1024 { // 1G max
            tracing::error!("SFTP packet is too long: {}", len);
            return Err(UnexpectedResponseSnafu.build().into());
        }
        let data = self.channel.fill_exact(len as usize + 4).await?;
        let msg = Message::parse(&data[4..])?;

        self.channel.consumer_read_buffer(len as usize + 4);

        if msg.id != request_id {
            return Err(MismatchResponseSnafu {
                expected: request_id,
                got: msg.id,
            }
            .build()
            .into());
        }

        Ok(msg)
    }

    async fn handle<T>(
        &mut self,
        p: impl FnOnce(u32) -> Vec<u8>,
        m: impl FnOnce(Payload) -> error::Result<T>,
    ) -> error::Result<T> {
        let request_id = self.next_request_id();

        let bytes = p(request_id);

        self.channel.send(&bytes).await?;
        self.channel.flush().await?;

        let response = self.receive_msg(request_id).await?;

        m(response.payload)
    }

    pub async fn symlink(&mut self, target: &str, linkpath: &str) -> error::Result<()> {
        // let request_id = self.next_request_id();
        // let buffer = make_buffer! {
        //     u8: SSH_FXP_READLINK,
        //     u32: request_id,
        //     one: target,
        //     one: linkpath
        // };

        // self.channel.send(&buffer[..]).await?;
        // self.channel.flush().await?;

        // let response = self.receive_msg(request_id).await?;

        // match response.payload {
        //     Payload::Status {
        //         status,
        //         error,
        //         language,
        //     } => {
        //         tracing::debug!("language: {}", language);
        //         status.to_result(error)
        //     }
        //     _ => Err(UnexpectedResponseSnafu.build().into()),
        // }

        self.handle(
            |request_id| {
                make_buffer! {
                    u8: SSH_FXP_READLINK,
                    u32: request_id,
                    one: target,
                    one: linkpath
                }
                .into_vec()
            },
            |payload| match payload {
                Payload::Status {
                    status,
                    error,
                    language,
                } => {
                    tracing::debug!("language: {}", language);
                    status.to_result(error)
                }
                _ => Err(UnexpectedResponseSnafu.build().into()),
            },
        )
        .await
    }

    pub async fn readlink(&mut self, path: &str) -> error::Result<FileInfo> {
        let request_id = self.next_request_id();
        let buffer = make_buffer! {
            u8: SSH_FXP_READLINK,
            u32: request_id,
            one: path
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let response = self.receive_msg(request_id).await?;

        match response.payload {
            Payload::Status { status, error, .. } => Err(status.to_error(error)),
            Payload::Name(mut entries) => {
                if entries.is_empty() {
                    return Err(UnexpectedResponseSnafu.build().into());
                }
                Ok(entries.remove(0))
            }
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    pub async fn rename(&mut self, old_path: &str, new_path: &str) -> error::Result<()> {
        let request_id = self.next_request_id();
        let buffer = make_buffer! {
            u8: SSH_FXP_RENAME,
            u32: request_id,
            one: old_path,
            one: new_path
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let response = self.receive_msg(request_id).await?;

        match response.payload {
            Payload::Status {
                status,
                error,
                language,
            } => {
                tracing::debug!("language: {}", language);
                status.to_result(error)
            }
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    pub async fn realpath(&mut self, path: &str) -> error::Result<FileInfo> {
        let request_id = self.next_request_id();
        let buffer = make_buffer! {
            u8: SSH_FXP_REALPATH,
            u32: request_id,
            one: path
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let response = self.receive_msg(request_id).await?;

        match response.payload {
            Payload::Status { status, error, .. } => Err(status.to_error(error)),
            Payload::Name(mut entries) => {
                if entries.is_empty() {
                    return Err(UnexpectedResponseSnafu.build().into());
                }
                Ok(entries.remove(0))
            }
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    pub async fn rmdir(&mut self, path: &str) -> error::Result<()> {
        let request_id = self.next_request_id();
        let buffer = make_buffer! {
            u8: SSH_FXP_RMDIR,
            u32: request_id,
            one: path
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let response = self.receive_msg(request_id).await?;

        match response.payload {
            Payload::Status {
                status,
                error,
                language,
            } => {
                tracing::debug!("language: {}", language);
                status.to_result(error)
            }
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    pub async fn mkdir(&mut self, path: &str, attrs: &Attributes) -> error::Result<()> {
        let request_id = self.next_request_id();
        let buffer = make_buffer! {
            u8: SSH_FXP_MKDIR,
            u32: request_id,
            one: path,
            bytes: attrs.to_bytes()
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let response = self.receive_msg(request_id).await?;

        match response.payload {
            Payload::Status {
                status,
                error,
                language,
            } => {
                tracing::debug!("language: {}", language);
                status.to_result(error)
            }
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    pub async fn remove_file(&mut self, path: &str) -> error::Result<()> {
        let request_id = self.next_request_id();
        let buffer = make_buffer! {
            u8: SSH_FXP_REMOVE,
            u32: request_id,
            one: path
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let response = self.receive_msg(request_id).await?;

        match response.payload {
            Payload::Status {
                status,
                error,
                language,
            } => {
                tracing::debug!("language: {}", language);
                status.to_result(error)
            }
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    pub async fn close_file(&mut self, file: &File) -> error::Result<()> {
        let request_id = self.next_request_id();
        let buffer = make_buffer! {
            u8: SSH_FXP_CLOSE,
            u32: request_id,
            one: file.handle(),
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let response = self.receive_msg(request_id).await?;

        match response.payload {
            Payload::Status {
                status,
                error,
                language,
            } => {
                tracing::debug!("language: {}", language);
                status.to_result(error)
            }
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    pub async fn open_file(
        &mut self,
        path: &str,
        flags: OpenFlags,
        permission: Option<Permissions>,
    ) -> error::Result<File> {
        let request_id = self.next_request_id();

        let buffer = if let Some(permission) = permission {
            make_buffer! {
                u8: SSH_FXP_OPEN,
                u32: request_id,
                one: path,
                u32: flags.bits(),
                u32: SSH_FILEXFER_ATTR_PERMISSIONS,
                u32: permission.bits(),
            }
        } else {
            make_buffer! {
                u8: SSH_FXP_OPEN,
                u32: request_id,
                one: path,
                u32: flags.bits(),
                u32: 0
            }
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let response = self.receive_msg(request_id).await?;

        match response.payload {
            Payload::Status {
                status,
                error,
                language,
            } if status != Status::OK => {
                tracing::debug!("language: {}", language);
                Err(status.to_error(error))
            }
            Payload::Handle(handle) => Ok(File::new(handle)),
            _ => Err(UnexpectedResponseSnafu {}.build().into()),
        }
    }

    pub async fn read_file(
        &mut self,
        file: &mut File,
        offset: u64,
        length: u32,
    ) -> error::Result<Vec<u8>> {
        let request_id = self.next_request_id();

        let buffer = make_buffer! {
            u8: SSH_FXP_READ,
            u32: request_id,
            one: file.handle(),
            u64: offset,
            u32: length,
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let msg = self.receive_msg(request_id).await?;

        match msg.payload {
            Payload::Status { status, error, .. } => status.to_result(error).map(|_| vec![]),
            Payload::Data(data) => {
                file.forward(data.len() as u64);
                Ok(data)
            }
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    pub async fn write_file(
        &mut self,
        file: &mut File,
        offset: u64,
        data: &[u8],
    ) -> error::Result<()> {
        let request_id = self.next_request_id();

        let buffer = make_buffer! {
            u8: SSH_FXP_WRITE,
            u32: request_id,
            one: file.handle(),
            u64: offset,
            one: data
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let msg = self.receive_msg(request_id).await?;

        match msg.payload {
            Payload::Status { status, error, .. } => {
                file.forward(data.len() as u64);
                status.to_result(error).map(|_| ())
            }
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    pub async fn open_directory(&mut self, path: &str) -> error::Result<Directory> {
        let request_id = self.next_request_id();

        let buffer = make_buffer! {
            u8: SSH_FXP_OPENDIR,
            u32: request_id,
            one: path
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let msg = self.receive_msg(request_id).await?;

        match msg.payload {
            Payload::Status { status, error, .. } => Err(status.to_error(error)),
            Payload::Handle(handle) => Ok(Directory::new(handle)),
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    pub async fn read_directory(&mut self, directory: &Directory) -> error::Result<Vec<FileInfo>> {
        let request_id = self.next_request_id();

        let buffer = make_buffer! {
            u8: SSH_FXP_READDIR,
            u32: request_id,
            one: directory.handle()
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let msg = self.receive_msg(request_id).await?;

        match msg.payload {
            Payload::Status { status, error, .. } => status.to_result(error).map(|_| vec![]),
            Payload::Name(entries) => Ok(entries),
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    async fn set_status(
        &mut self,
        target: &[u8],
        code: u8,
        attrs: &Attributes,
    ) -> error::Result<()> {
        let request_id = self.next_request_id();

        let buffer = make_buffer! {
            u8: code,
            u32: request_id,
            one: target,
            bytes: attrs.to_bytes()
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let msg = self.receive_msg(request_id).await?;

        match msg.payload {
            Payload::Status { status, error, .. } => status.to_result(error).map(|_| ()),
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    pub async fn set_stat(&mut self, path: &str, attrs: &Attributes) -> error::Result<()> {
        self.set_status(path.as_bytes(), SSH_FXP_SETSTAT, attrs)
            .await
    }

    pub async fn set_fstat(&mut self, file: &File, attrs: &Attributes) -> error::Result<()> {
        self.set_status(file.handle(), SSH_FXP_FSETSTAT, attrs)
            .await
    }

    async fn status(&mut self, target: &[u8], code: u8) -> error::Result<Attributes> {
        let request_id = self.next_request_id();

        let buffer = make_buffer! {
            u8: code,
            u32: request_id,
            one: target
        };

        self.channel.send(&buffer[..]).await?;
        self.channel.flush().await?;

        let msg = self.receive_msg(request_id).await?;

        match msg.payload {
            Payload::Status { status, error, .. } => Err(status.to_error(error)),
            Payload::Attributes(attrs) => Ok(attrs),
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    pub async fn stat(&mut self, path: &str) -> error::Result<Attributes> {
        self.status(path.as_bytes(), SSH_FXP_STAT).await
    }

    pub async fn lstat(&mut self, path: &str) -> error::Result<Attributes> {
        self.status(path.as_bytes(), SSH_FXP_LSTAT).await
    }

    pub async fn fstat(&mut self, file: &File) -> error::Result<Attributes> {
        self.status(file.handle(), SSH_FXP_FSTAT).await
    }
}

#[cfg(test)]
mod test {
    use crate::{session::sftp::OpenFlags, test::*};

    async fn open_sftp() -> anyhow::Result<super::Handle> {
        tracing_subscriber::fmt::init();
        let config = Config::load().await?;

        let session = config.open_session().await?;
        session.request_authentication().await?;
        config.authenticate_password(&session).await?;

        let handle = session.sftp_open_default().await?;

        Ok(handle)
    }

    #[tokio::test]
    async fn test_handshake() -> anyhow::Result<()> {
        let handle = open_sftp().await?;

        handle.close().await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_open_file() -> anyhow::Result<()> {
        let mut handle = open_sftp().await?;

        let file = handle
            .open_file("/usr/bin/ls", OpenFlags::READ, None)
            .await?;

        tracing::info!("Opened file: {:?}", file);

        handle.close_file(&file).await?;

        handle.close().await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_read_file() -> anyhow::Result<()> {
        let mut handle = open_sftp().await?;

        let mut file = handle
            .open_file("/usr/bin/ls", OpenFlags::READ, None)
            .await?;

        let data = handle.read_file(&mut file, 0, u32::MAX).await?;

        tracing::info!("Read data: {}", data.len());

        handle.close_file(&file).await?;

        handle.close().await?;

        Ok(())
    }
}
