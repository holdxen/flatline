//! Client-side SFTP (SSH File Transfer Protocol) operations.
//!
//! This module implements version 3 of the SFTP protocol
//! (draft-ietf-secsh-filexfer-01) on top of a session channel running the
//! `sftp` subsystem, plus the protocol extensions implemented by OpenSSH's
//! server: `posix-rename@openssh.com`, `statvfs@openssh.com`,
//! `fstatvfs@openssh.com`, `hardlink@openssh.com`, `fsync@openssh.com`,
//! `lsetstat@openssh.com`, `limits@openssh.com`, `expand-path@openssh.com`,
//! `copy-data`, `home-directory`, and `users-groups-by-id@openssh.com`.
//!
//! A [`Handle`] is obtained from
//! [`crate::session::Session::sftp_open_default`] (see the crate
//! documentation), which performs the `SSH_FXP_INIT`/`SSH_FXP_VERSION`
//! handshake and records the extensions advertised by the server. The handle
//! then exposes one `async` method per SFTP request — opening, reading and
//! writing files, listing directories, querying attributes — while failures
//! are reported as [`Error`], whose first eight variants mirror the
//! `SSH_FX_*` status codes. The SFTP wire types ([`OpenFlags`],
//! [`Attributes`], [`File`], [`FileInfo`], and friends) are re-exported from
//! this module as well.
//!
//! # Examples
//!
//! Opening a file, reading from it, and closing everything again:
//!
//! ```rust,no_run
//! use flatline::session::sftp::OpenFlags;
//! use flatline::session::{Config, DefaultNotifier, Session};
//! use tokio::net::TcpStream;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let socket = TcpStream::connect("example.com:22").await?;
//! let session = Session::handshake(socket, Config::default(), DefaultNotifier).await?;
//!
//! session.request_authentication().await?;
//! let status = session.authenticate_password("user", "password").await?;
//! assert!(status.success());
//!
//! let mut sftp = session.sftp_open_default().await?;
//!
//! let mut file = sftp.open_file("/etc/hostname", OpenFlags::READ, None).await?;
//! let data = sftp.read_file(&mut file, 0, 8192).await?;
//! println!("read {} bytes", data.len());
//!
//! sftp.close_file(&file).await?;
//! sftp.close().await?;
//! # Ok(())
//! # }
//! ```

use crate::session::channel::{BufferChannel, Channel};
use crate::ssh::buffer::Consumer;
use crate::ssh::buffer::*;
use crate::ssh::protocol::SFTPExtension;
use crate::ssh::protocol::sftp::*;
use crate::{error, ssh};
use snafu::ResultExt;
use std::collections::HashMap;

mod types;

pub use types::*;

/// Errors reported by the SFTP subsystem.
///
/// The first eight variants correspond one-to-one to the `SSH_FX_*` status
/// codes that an SFTP server returns in an `SSH_FXP_STATUS` reply and carry
/// the server's error message in `msg`; the remaining variants describe
/// framing, parsing, and reply-matching problems detected locally by the
/// client.
#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("Unexpected EOF: {}", msg))]
    /// The server reported `SSH_FX_EOF` (status 1): no more data will be returned.
    UnexpectedEof {
        /// The error message reported by the server.
        msg: String,
    },
    #[snafu(display("No such file: {}", msg))]
    /// The server reported `SSH_FX_NO_SUCH_FILE` (status 2): the referenced file does not exist.
    NoSuchFile {
        /// The error message reported by the server.
        msg: String,
    },
    #[snafu(display("Permission denied: {}", msg))]
    /// The server reported `SSH_FX_PERMISSION_DENIED` (status 3): access to the file is denied.
    PermissionDenied {
        /// The error message reported by the server.
        msg: String,
    },
    #[snafu(display("Failure: {}", msg))]
    /// The server reported `SSH_FX_FAILURE` (status 4): a nonspecific error with no more precise code.
    Failure {
        /// The error message reported by the server.
        msg: String,
    },
    #[snafu(display("Bad message: {}", msg))]
    /// The server reported `SSH_FX_BAD_MESSAGE` (status 5): the request was malformed or could not be processed.
    BadMessage {
        /// The error message reported by the server.
        msg: String,
    },
    #[snafu(display("No connection: {}", msg))]
    /// The server reported `SSH_FX_NO_CONNECTION` (status 6): there is no connection to the server.
    NoConnection {
        /// The error message reported by the server.
        msg: String,
    },
    #[snafu(display("Connection lost: {}", msg))]
    /// The server reported `SSH_FX_CONNECTION_LOST` (status 7): the connection was lost during the operation.
    ConnectionLost {
        /// The error message reported by the server.
        msg: String,
    },
    #[snafu(display("Operation not supported: {}", msg))]
    /// The server reported `SSH_FX_OP_UNSUPPORTED` (status 8): the operation or extension is not supported.
    OpUnsupported {
        /// The error message reported by the server.
        msg: String,
    },
    #[snafu(display("Unexpected message code: {}", code))]
    /// An SFTP message arrived carrying an unexpected message type code.
    ///
    /// Also raised during the version exchange when the server does not
    /// answer `SSH_FXP_INIT` with `SSH_FXP_VERSION`. `code` holds the
    /// offending message type.
    UnexpectedMessage {
        /// The offending message type code.
        code: u8,
    },
    #[snafu(display("Unknown file type: {}", source))]
    /// An attribute block carried permission bits that encode no known file type.
    ///
    /// The failed conversion is available in `source`.
    UnknownFileType {
        /// The failed conversion from the raw permission bits.
        source: num_enum::TryFromPrimitiveError<types::FileType>,
    },
    #[snafu(display("Unexpected status {}: {}", status, source))]
    /// An `SSH_FXP_STATUS` reply carried a status code outside the SFTP v3 set.
    ///
    /// `status` holds the raw code and `source` the failed conversion.
    UnexpectedStatus {
        /// The failed conversion from the raw status code.
        source: num_enum::TryFromPrimitiveError<types::Status>,
        /// The raw status code that was received.
        status: u32,
    },
    #[snafu(display("Mismatch response: expected {}, got {}", expected, got))]
    /// A reply carried a request id other than that of the outstanding request.
    ///
    /// `expected` is the id that was sent and `got` the id that arrived.
    MismatchResponse {
        /// The request id that was sent.
        expected: u32,
        /// The request id that was received.
        got: u32,
    },
    #[snafu(display("Unexpected response"))]
    /// The server sent a reply that does not fit the request that was sent.
    ///
    /// Raised when the payload has an unexpected kind (for example an empty
    /// `SSH_FXP_NAME` list), when a status of `SSH_FX_OK` appears where data
    /// was expected, or when an SFTP packet exceeds the 1 GiB size limit.
    UnexpectedResponse {},
}

impl Error {
    /// Returns `true` when the error was reported by the server as an `SSH_FX_*` status.
    ///
    /// The status-derived variants (`UnexpectedEof`, `NoSuchFile`,
    /// `PermissionDenied`, `Failure`, `BadMessage`, `NoConnection`,
    /// `ConnectionLost`, `OpUnsupported`) yield `true`, while problems
    /// detected locally while framing, parsing, or matching replies
    /// (`UnexpectedMessage`, `UnknownFileType`, `UnexpectedStatus`,
    /// `MismatchResponse`, `UnexpectedResponse`) yield `false`.
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

/// An SFTP session: a channel running the `sftp` subsystem plus the server's advertised extensions.
///
/// A handle is produced by [`crate::session::Session::sftp_open_default`] or
/// [`crate::session::Session::sftp_open`], which perform the `SSH_FXP_INIT` /
/// `SSH_FXP_VERSION` handshake, record the extension list from the server's
/// `SSH_FXP_VERSION` reply, and start the subsystem. Each method sends one
/// request on the channel and waits for its matching reply, so operations
/// take `&mut self` and are executed sequentially.
///
/// Use the `is_*_supported` methods to discover which OpenSSH protocol
/// extensions the server offers; the matching operation is only valid when
/// its check returns `true`.
#[derive(Debug)]
pub struct Handle {
    channel: BufferChannel,
    extensions: HashMap<String, Vec<u8>>,
    request_id: u32,
}

impl Handle {
    const MAX_PACKET_SIZE: u32 = 1024 * 1024 * 1024;
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

        if len > Self::MAX_PACKET_SIZE {
            tracing::error!("Packet is too long");
            return Err(UnexpectedResponseSnafu.build().into());
        }

        let data = channel.fill_exact(len as usize + 4).await?;

        let mut consumer = Consumer::new(&data[4..]);

        let code = consumer.consume_u8()?;
        if code != SSH_FXP_VERSION {
            return Err(UnexpectedMessageSnafu { code }.build().into());
        }

        let version = consumer.consume_u32()?;
        if version != VERSION {
            tracing::warn!(
                "SFTP version mismatch: mine={}, received={}",
                version,
                VERSION
            );
        }

        let mut extensions = HashMap::new();
        while !consumer.is_empty() {
            let k = consumer.consume_one()?;
            let k = std::str::from_utf8(k).context(ssh::ExpectStringSnafu)?;
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

    /// Closes the underlying SSH channel, ending the `sftp` subsystem, and consumes the handle.
    ///
    /// This does not send an `SSH_FXP_CLOSE` request for files that are still
    /// open; release them first with [`Handle::close_file`]. The server
    /// discards any handle that remains open once the channel is torn down.
    /// Returns any error encountered while closing the channel.
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

    /// Returns whether the server advertised `posix-rename@openssh.com` (version `1`).
    ///
    /// [`Handle::posix_rename`] is only valid when this returns `true`.
    pub fn is_posix_rename_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_POSIX_RENAME)
    }
    /// Renames `oldpath` to `newpath`, replacing `newpath` if it already exists.
    ///
    /// Sends an `SSH_FXP_EXTENDED` request carrying the
    /// `posix-rename@openssh.com` extension, which maps to POSIX `rename(2)`
    /// and therefore overwrites an existing destination, unlike
    /// [`Handle::rename`]. Only call this when
    /// [`Handle::is_posix_rename_supported`] returns `true`; the
    /// `debug_assert!` inside panics otherwise in debug builds. `Ok(())` is
    /// returned for `SSH_FX_OK`, and any other status (for example
    /// [`Error::NoSuchFile`]) comes back as an error.
    pub async fn posix_rename(&mut self, oldpath: &str, newpath: &str) -> error::Result<()> {
        debug_assert!(
            self.is_posix_rename_supported(),
            "Server doesn't support posix rename"
        );

        self.handle(
            |request_id| {
                make_buffer! {
                    u8: SSH_FXP_EXTENDED,
                    u32: request_id,
                    one: OPENSSH_SFTP_EXT_POSIX_RENAME.key,
                    one: oldpath,
                    one: newpath,
                }
                .into_vec()
            },
            |payload| match payload {
                Payload::Status { status, error, .. } => status.to_result(error),
                _ => Err(UnexpectedResponseSnafu.build().into()),
            },
        )
        .await
    }

    /// Returns whether the server advertised `statvfs@openssh.com` (version `2`).
    ///
    /// [`Handle::statvfs`] is only valid when this returns `true`.
    pub fn is_statvfs_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_STATVFS)
    }
    /// Returns file system statistics for the file system containing `path`.
    ///
    /// Sends an `SSH_FXP_EXTENDED` request with the `statvfs@openssh.com`
    /// extension and parses the `SSH_FXP_EXTENDED_REPLY` into [`Statvfs`].
    /// Only call this when [`Handle::is_statvfs_supported`] returns `true`
    /// (the `debug_assert!` inside panics otherwise in debug builds); any
    /// `SSH_FXP_STATUS` reply comes back as an error.
    pub async fn statvfs(&mut self, path: &str) -> error::Result<Statvfs> {
        debug_assert!(
            self.is_statvfs_supported(),
            "Server doesn't support statvfs"
        );
        self.handle(
            |request_id| {
                make_buffer! {
                    u8: SSH_FXP_EXTENDED,
                    u32: request_id,
                    one: OPENSSH_SFTP_EXT_STATVFS.key,
                    one: path,
                }
                .into_vec()
            },
            |payload| match payload {
                Payload::Status { status, error, .. } => Err(status.to_error(error)),
                Payload::ExtendReply(data) => Statvfs::parse(&data),
                _ => Err(UnexpectedResponseSnafu.build().into()),
            },
        )
        .await
    }

    /// Returns whether the server advertised `fstatvfs@openssh.com` (version `2`).
    ///
    /// [`Handle::fstatvfs`] is only valid when this returns `true`.
    pub fn is_fstatvfs_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_FSTATVFS)
    }

    /// Returns file system statistics for the file system containing the open `file`.
    ///
    /// Sends an `SSH_FXP_EXTENDED` request with the `fstatvfs@openssh.com`
    /// extension, identified by `file`'s handle, and parses the
    /// `SSH_FXP_EXTENDED_REPLY` into [`Statvfs`]. Only call this when
    /// [`Handle::is_fstatvfs_supported`] returns `true` (the `debug_assert!`
    /// inside panics otherwise in debug builds); any `SSH_FXP_STATUS` reply
    /// comes back as an error.
    pub async fn fstatvfs(&mut self, file: &File) -> error::Result<Statvfs> {
        debug_assert!(
            self.is_fstatvfs_supported(),
            "Server doesn't support fstatvfs"
        );

        self.handle(
            |request_id| {
                make_buffer! {
                    u8: SSH_FXP_EXTENDED,
                    u32: request_id,
                    one: OPENSSH_SFTP_EXT_FSTATVFS.key,
                    one: file.handle(),
                }
                .into_vec()
            },
            |payload| match payload {
                Payload::Status { status, error, .. } => Err(status.to_error(error)),
                Payload::ExtendReply(data) => Statvfs::parse(&data),
                _ => Err(UnexpectedResponseSnafu.build().into()),
            },
        )
        .await
    }

    /// Returns whether the server advertised `hardlink@openssh.com` (version `1`).
    ///
    /// [`Handle::hardlink`] is only valid when this returns `true`.
    pub fn is_hardlink_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_HARDLINK)
    }

    /// Creates `newpath` as a hard link to the existing file `oldpath`.
    ///
    /// Sends an `SSH_FXP_EXTENDED` request with the `hardlink@openssh.com`
    /// extension, mapping to `link(2)` on the server. `Ok(())` is returned
    /// for `SSH_FX_OK`; any other status comes back as an error. Only call
    /// this when [`Handle::is_hardlink_supported`] returns `true` (the
    /// `debug_assert!` inside panics otherwise in debug builds).
    pub async fn hardlink(&mut self, oldpath: &str, newpath: &str) -> error::Result<()> {
        debug_assert!(
            self.is_hardlink_supported(),
            "Server doesn't support hardlink"
        );

        self.handle(
            |request_id| {
                make_buffer! {
                    u8: SSH_FXP_EXTENDED,
                    u32: request_id,
                    one: OPENSSH_SFTP_EXT_HARDLINK.key,
                    one: oldpath,
                    one: newpath,
                }
                .into_vec()
            },
            |payload| match payload {
                Payload::Status { status, error, .. } => status.to_result(error),
                _ => Err(UnexpectedResponseSnafu.build().into()),
            },
        )
        .await
    }

    /// Returns whether the server advertised `fsync@openssh.com` (version `1`).
    ///
    /// [`Handle::fsync`] is only valid when this returns `true`.
    pub fn is_fsync_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_FSYNC)
    }

    /// Flushes the server-side buffers of the open `file` to stable storage.
    ///
    /// Sends an `SSH_FXP_EXTENDED` request with the `fsync@openssh.com`
    /// extension, mapping to `fsync(2)` on the server. `Ok(())` is returned
    /// for `SSH_FX_OK`; any other status comes back as an error. Only call
    /// this when [`Handle::is_fsync_supported`] returns `true` (the
    /// `debug_assert!` inside panics otherwise in debug builds).
    pub async fn fsync(&mut self, file: &File) -> error::Result<()> {
        debug_assert!(self.is_fsync_supported(), "Server doesn't support fsync");

        self.handle(
            |request_id| {
                make_buffer! {
                    u8: SSH_FXP_EXTENDED,
                    u32: request_id,
                    one: OPENSSH_SFTP_EXT_FSYNC.key,
                    one: file.handle(),
                }
                .into_vec()
            },
            |payload| match payload {
                Payload::Status { status, error, .. } => status.to_result(error),
                _ => Err(UnexpectedResponseSnafu.build().into()),
            },
        )
        .await
    }

    /// Returns whether the server advertised `lsetstat@openssh.com` (version `1`).
    ///
    /// [`Handle::lsetstat`] is only valid when this returns `true`.
    pub fn is_lsetstat_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_LSETSTAT)
    }

    /// Sets the attributes of `path` without following symbolic links.
    ///
    /// Sends an `SSH_FXP_EXTENDED` request with the `lsetstat@openssh.com`
    /// extension — like [`Handle::set_stat`] applied with
    /// `AT_SYMLINK_NOFOLLOW`, so the owner, mode, or timestamps of a
    /// symbolic link itself can be changed. Only the fields that are `Some`
    /// in `attrs` are transmitted; note that OpenSSH's server rejects a
    /// `size` attribute with `SSH_FX_BAD_MESSAGE`. Only call this when
    /// [`Handle::is_lsetstat_supported`] returns `true` (the `debug_assert!`
    /// inside panics otherwise in debug builds).
    pub async fn lsetstat(&mut self, path: &str, attrs: &Attributes) -> error::Result<()> {
        debug_assert!(self.is_lsetstat_supported(), "Server doesn't lsetstat");

        self.handle(
            |request_id| {
                make_buffer! {
                    u8: SSH_FXP_EXTENDED,
                    u32: request_id,
                    one: OPENSSH_SFTP_EXT_LSETSTAT.key,
                    one: path,
                    bytes: attrs.to_bytes(),
                }
                .into_vec()
            },
            |payload| match payload {
                Payload::Status { status, error, .. } => status.to_result(error),
                _ => Err(UnexpectedResponseSnafu.build().into()),
            },
        )
        .await
    }

    /// Returns whether the server advertised `limits@openssh.com` (version `1`).
    ///
    /// [`Handle::limits`] is only valid when this returns `true`.
    pub fn is_limits_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_LIMITS)
    }

    /// Queries the protocol limits that the server enforces.
    ///
    /// Sends an `SSH_FXP_EXTENDED` request with the `limits@openssh.com`
    /// extension and parses the `SSH_FXP_EXTENDED_REPLY` into [`Limits`]
    /// (maximum packet length, maximum read and write lengths, and the
    /// maximum number of open handles). Only call this when
    /// [`Handle::is_limits_supported`] returns `true` (the `debug_assert!`
    /// inside panics otherwise in debug builds).
    pub async fn limits(&mut self) -> error::Result<Limits> {
        debug_assert!(self.is_limits_supported(), "Server doesn't support limits");

        self.handle(
            |request_id| {
                make_buffer! {
                    u8: SSH_FXP_EXTENDED,
                    u32: request_id,
                    one: OPENSSH_SFTP_EXT_LIMITS.key
                }
                .into_vec()
            },
            |payload| match payload {
                Payload::ExtendReply(data) => Limits::parse(&data),
                _ => Err(UnexpectedResponseSnafu.build().into()),
            },
        )
        .await
    }

    /// Returns whether the server advertised `expand-path@openssh.com` (version `1`).
    ///
    /// [`Handle::expand_path`] is only valid when this returns `true`.
    pub fn is_expand_path_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_EXPAND_PATH)
    }

    /// Expands `path` on the server and returns the canonical absolute path.
    ///
    /// Sends an `SSH_FXP_EXTENDED` request with the
    /// `expand-path@openssh.com` extension. The server resolves `~` and
    /// `~user` prefixes as well as relative components (OpenSSH also
    /// canonicalizes the result); the returned `String` is the file name of
    /// the first entry of the `SSH_FXP_NAME` reply. Only call this when
    /// [`Handle::is_expand_path_supported`] returns `true` (the
    /// `debug_assert!` inside panics otherwise in debug builds), and
    /// consider [`Handle::realpath`] for plain canonicalization.
    pub async fn expand_path(&mut self, path: &str) -> error::Result<String> {
        debug_assert!(
            self.is_expand_path_supported(),
            "Server doesn't support expand path"
        );

        self.handle(
            |request_id| {
                make_buffer! {
                    u8: SSH_FXP_EXTENDED,
                    u32: request_id,
                    one: OPENSSH_SFTP_EXT_EXPAND_PATH.key,
                    one: path
                }
                .into_vec()
            },
            |payload| match payload {
                Payload::Status { status, error, .. } => Err(status.to_error(error)),
                Payload::Name(file_infos) => {
                    if file_infos.is_empty() {
                        return Err(UnexpectedResponseSnafu {}.build().into());
                    }
                    Ok(file_infos[0].file_name.clone())
                }
                _ => Err(UnexpectedResponseSnafu.build().into()),
            },
        )
        .await
    }

    /// Returns whether the server advertised the `copy-data` extension (version `1`).
    ///
    /// [`Handle::copy_data`] is only valid when this returns `true`.
    pub fn is_copy_data_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_COPY_DATA)
    }

    /// Copies `len` bytes from `read` to `write` entirely on the server.
    ///
    /// Sends an `SSH_FXP_EXTENDED` request with the `copy-data` extension,
    /// using `read.pos()` as the source offset and `write.pos()` as the
    /// destination offset — the two [`File`] values contribute only their
    /// handles and client-side cursors. Neither cursor is advanced by this
    /// call; move them with [`File::forward`] as needed. A `len` of `0`
    /// asks an OpenSSH server to copy until the end of the source file, and
    /// the server refuses to copy a file onto itself (`SSH_FX_FAILURE`).
    /// Only call this when [`Handle::is_copy_data_supported`] returns `true`
    /// (the `debug_assert!` inside panics otherwise in debug builds).
    pub async fn copy_data(
        &mut self,
        read: &mut File,
        len: u64,
        write: &mut File,
    ) -> error::Result<()> {
        debug_assert!(
            self.is_copy_data_supported(),
            "Server doesn't support copy data"
        );

        self.handle(
            |request_id| {
                make_buffer! {
                    u8: SSH_FXP_EXTENDED,
                    u32: request_id,
                    one: OPENSSH_SFTP_EXT_COPY_DATA.key,
                    one: &read.handle(),
                    u64: read.pos(),
                    u64: len,
                    one: &write.handle(),
                    u64: write.pos(),
                }
                .into_vec()
            },
            |payload| match payload {
                Payload::Status { status, error, .. } => status.to_result(error),
                _ => Err(UnexpectedResponseSnafu.build().into()),
            },
        )
        .await
    }

    /// Returns whether the server advertised the `home-directory` extension (version `1`).
    ///
    /// [`Handle::home_directory`] is only valid when this returns `true`.
    pub fn is_home_directory_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_HOME_DIRECTORY)
    }

    /// Returns the home directory path of `username`.
    ///
    /// Sends an `SSH_FXP_EXTENDED` request with the `home-directory`
    /// extension; the path is the file name of the first entry of the
    /// `SSH_FXP_NAME` reply. An empty `username` asks for the account the
    /// server runs as (OpenSSH); unknown users produce a status error. Only
    /// call this when [`Handle::is_home_directory_supported`] returns `true`
    /// (the `debug_assert!` inside panics otherwise in debug builds).
    pub async fn home_directory(&mut self, username: &str) -> error::Result<String> {
        debug_assert!(
            self.is_home_directory_supported(),
            "Server doesn't support home directory"
        );

        self.handle(
            |request_id| {
                make_buffer! {
                    u8: SSH_FXP_EXTENDED,
                    u32: request_id,
                    one: OPENSSH_SFTP_EXT_HOME_DIRECTORY.key,
                    one: username
                }
                .into_vec()
            },
            |payload| match payload {
                Payload::Status { status, error, .. } => Err(status.to_error(error)),
                Payload::Name(file_infos) => {
                    if file_infos.is_empty() {
                        return Err(UnexpectedResponseSnafu {}.build().into());
                    }
                    Ok(file_infos[0].file_name.clone())
                }
                _ => Err(UnexpectedResponseSnafu.build().into()),
            },
        )
        .await
    }

    /// Returns whether the server advertised `users-groups-by-id@openssh.com` (version `1`).
    ///
    /// [`Handle::users_groups_by_id`] is only valid when this returns `true`.
    pub fn is_users_groups_by_id_supported(&self) -> bool {
        self.supported(OPENSSH_SFTP_EXT_USERS_GROUPS_BY_ID)
    }
    /// Resolves numeric user and group ids to their names.
    ///
    /// Sends an `SSH_FXP_EXTENDED` request with the
    /// `users-groups-by-id@openssh.com` extension and parses the
    /// `SSH_FXP_EXTENDED_REPLY` into `(usernames, groupnames)`, whose
    /// entries correspond one-to-one, in order, with the `users` and
    /// `groups` slices. Only call this when
    /// [`Handle::is_users_groups_by_id_supported`] returns `true` (the
    /// `debug_assert!` inside panics otherwise in debug builds); a
    /// `SSH_FXP_STATUS` reply comes back as an error.
    pub async fn users_groups_by_id(
        &mut self,
        users: &[u32],
        groups: &[u32],
    ) -> error::Result<(Vec<String>, Vec<String>)> {
        debug_assert!(
            self.is_users_groups_by_id_supported(),
            "Server doesn't support users-groups-by-id"
        );

        self.handle(
            |request_id| {
                make_buffer! {
                    u8: SSH_FXP_EXTENDED,
                    u32: request_id,
                    one: OPENSSH_SFTP_EXT_USERS_GROUPS_BY_ID.key,
                    one_list_u32: users,
                    one_list_u32: groups
                }
                .into_vec()
            },
            |payload| match payload {
                Payload::Status { status, error, .. } => Err(status.to_error(error)),
                Payload::ExtendReply(data) => {
                    let mut consumer = Consumer::new(&data);
                    let usernames = {
                        let mut consumer = Consumer::new(consumer.consume_one()?);
                        let mut usernames = Vec::with_capacity(users.len());
                        while !consumer.is_empty() {
                            let name = std::str::from_utf8(consumer.consume_one()?)
                                .context(ssh::ExpectStringSnafu)?;
                            usernames.push(name.to_string());
                        }
                        usernames
                    };

                    let groupnames = {
                        let mut consumer = Consumer::new(consumer.consume_one()?);
                        let mut groupnames = Vec::with_capacity(groups.len());
                        while !consumer.is_empty() {
                            let name = std::str::from_utf8(consumer.consume_one()?)
                                .context(ssh::ExpectStringSnafu)?;
                            groupnames.push(name.to_string());
                        }
                        groupnames
                    };

                    Ok((usernames, groupnames))
                }
                _ => Err(UnexpectedResponseSnafu.build().into()),
            },
        )
        .await
    }

    async fn receive_msg(&mut self, request_id: u32) -> error::Result<Message> {
        let len = self.channel.fill_exact(4).await?;
        let len = u32::from_be_bytes(len.try_into().unwrap());
        if len > 1024 * 1024 * 1024 {
            // 1G max
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

    /// Creates a symbolic link at `linkpath` whose contents point to `target`.
    ///
    /// Sends an `SSH_FXP_SYMLINK` request. Note the argument order: `target`
    /// comes first and `linkpath` second — the server creates the link at
    /// `linkpath` pointing at `target`, so `symlink("/etc/hosts",
    /// "hosts.link")` creates `hosts.link`. The request fails if `linkpath`
    /// already exists; `Ok(())` is returned for `SSH_FX_OK` and any other
    /// status comes back as an error.
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
                    u8: SSH_FXP_SYMLINK,
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

    /// Reads the target of the symbolic link at `path`.
    ///
    /// Sends an `SSH_FXP_READLINK` request and returns the first entry of
    /// the `SSH_FXP_NAME` reply, whose `file_name` holds the path the link
    /// points to. A status reply (for example [`Error::NoSuchFile`]) or an
    /// empty name list comes back as an error.
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

    /// Renames `old_path` to `new_path`.
    ///
    /// Sends an `SSH_FXP_RENAME` request. Under SFTP v3 the request fails if
    /// `new_path` already exists; use [`Handle::posix_rename`] to replace an
    /// existing destination atomically. `Ok(())` is returned for `SSH_FX_OK`
    /// and any other status comes back as an error.
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

    /// Resolves `path` to a canonical absolute path.
    ///
    /// Sends an `SSH_FXP_REALPATH` request and returns the first entry of
    /// the `SSH_FXP_NAME` reply, whose `file_name` is the canonical path. A
    /// status reply or an empty name list comes back as an error.
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

    /// Removes the empty directory at `path`.
    ///
    /// Sends an `SSH_FXP_RMDIR` request. `Ok(())` is returned for
    /// `SSH_FX_OK`; a missing directory or one that is not empty comes back
    /// as an error.
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

    /// Creates a directory at `path` with the initial attributes `attrs`.
    ///
    /// Sends an `SSH_FXP_MKDIR` request. Only the fields that are `Some` in
    /// `attrs` are transmitted — typically [`Attributes::property`], for
    /// example [`Permissions::p0755`] — while unset fields fall back to the
    /// server's defaults. `Ok(())` is returned for `SSH_FX_OK`; any other
    /// status comes back as an error.
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

    /// Deletes the file at `path`.
    ///
    /// Sends an `SSH_FXP_REMOVE` request (directories are removed with
    /// [`Handle::rmdir`]). `Ok(())` is returned for `SSH_FX_OK`; a missing
    /// file or a directory comes back as an error.
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

    /// Releases the server-side handle that was opened for `file`.
    ///
    /// Sends an `SSH_FXP_CLOSE` request. The [`File`] value is not consumed,
    /// but it must not be used for further requests once it has been closed.
    /// `Ok(())` is returned for `SSH_FX_OK`; any other status comes back as
    /// an error.
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

    /// Opens `path` and returns a [`File`] handle for it.
    ///
    /// Sends an `SSH_FXP_OPEN` request carrying the portable `flags` (for
    /// example [`OpenFlags::READ`] or [`OpenFlags::WRITE`]). `permission`,
    /// when `Some`, is sent as the `SSH_FILEXFER_ATTR_PERMISSIONS` attribute
    /// and is used by the server when it creates the file; when `None` no
    /// attributes are transmitted and the server applies its own default
    /// creation mode. Any status other than success (such as
    /// [`Error::NoSuchFile`] or [`Error::PermissionDenied`]) comes back as
    /// an error.
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

    /// Reads up to `length` bytes from `file`, starting at `offset`.
    ///
    /// Sends an `SSH_FXP_READ` request. SFTP reads are not seek-based, so
    /// the `offset` is supplied explicitly on every call — commonly
    /// [`File::pos`], the client-side cursor; the server keeps no position
    /// for a handle. On a `SSH_FXP_DATA` reply the bytes are returned and
    /// `file`'s cursor advances by the number of bytes read.
    ///
    /// Status replies become errors: in particular, the end of the file is
    /// reported as `SSH_FX_EOF` and surfaces as [`Error::UnexpectedEof`].
    /// Only an `SSH_FX_OK` status — which servers do not send for a read —
    /// would produce an empty `Vec`.
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

    /// Writes `data` to `file`, starting at `offset`.
    ///
    /// Sends an `SSH_FXP_WRITE` request. As with [`Handle::read_file`], the
    /// `offset` is explicit (commonly [`File::pos`]); on success
    /// (`SSH_FX_OK`) `file`'s cursor advances by `data.len()`. On any other
    /// status the cursor is left untouched and the status comes back as an
    /// error.
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
                status.to_result(error)?;
                file.forward(data.len() as u64);
                Ok(())
            }
            _ => Err(UnexpectedResponseSnafu.build().into()),
        }
    }

    /// Opens the directory at `path` for listing.
    ///
    /// Sends an `SSH_FXP_OPENDIR` request and returns a [`Directory`] handle
    /// to pass to [`Handle::read_directory`]. Any `SSH_FXP_STATUS` reply (for
    /// example when `path` is missing or is not a directory) comes back as
    /// an error.
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

    /// Fetches the next batch of entries from `directory`.
    ///
    /// Sends an `SSH_FXP_READDIR` request and returns the entries as
    /// [`FileInfo`] values. Repeat until the server has returned everything;
    /// it then replies `SSH_FX_EOF`, which is surfaced as
    /// [`Error::UnexpectedEof`] — treat that error as the end of the
    /// listing.
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

    /// Updates the attributes of `path`.
    ///
    /// Sends an `SSH_FXP_SETSTAT` request; only the fields that are `Some`
    /// in `attrs` are transmitted. Symbolic links are followed — the
    /// [`Handle::lsetstat`] extension operates on the link itself. `Ok(())`
    /// is returned for `SSH_FX_OK`, and any other status comes back as an
    /// error.
    pub async fn set_stat(&mut self, path: &str, attrs: &Attributes) -> error::Result<()> {
        self.set_status(path.as_bytes(), SSH_FXP_SETSTAT, attrs)
            .await
    }

    /// Updates the attributes of the open `file`.
    ///
    /// Sends an `SSH_FXP_FSETSTAT` request with `file`'s handle; only the
    /// fields that are `Some` in `attrs` are transmitted. `Ok(())` is
    /// returned for `SSH_FX_OK`, and any other status comes back as an
    /// error.
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

    /// Returns the attributes of `path`, following symbolic links.
    ///
    /// Sends an `SSH_FXP_STAT` request. A status reply (for example
    /// [`Error::NoSuchFile`]) or an unexpected payload comes back as an
    /// error.
    pub async fn stat(&mut self, path: &str) -> error::Result<Attributes> {
        self.status(path.as_bytes(), SSH_FXP_STAT).await
    }

    /// Returns the attributes of `path` without following a final symbolic link.
    ///
    /// Sends an `SSH_FXP_LSTAT` request; otherwise identical to
    /// [`Handle::stat`].
    pub async fn lstat(&mut self, path: &str) -> error::Result<Attributes> {
        self.status(path.as_bytes(), SSH_FXP_LSTAT).await
    }

    /// Returns the attributes of the open `file`.
    ///
    /// Sends an `SSH_FXP_FSTAT` request with `file`'s handle. A status reply
    /// or an unexpected payload comes back as an error.
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
