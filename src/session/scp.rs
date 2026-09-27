//! The legacy SCP file transfer protocol.
//!
//! SCP runs the remote side's `scp` binary over an exec channel
//! (`scp -f` to fetch, `scp -t` to send) and speaks the small control
//! protocol built from single-byte acknowledgements and `C`/`D`/`E`/`T`
//! records. Wrap a channel in a [`Handle`], then drive the exchange with
//! [`Handle::start_receiving`] or [`Handle::start_sending`].
//!
//! ```rust,no_run
//! # async fn run(session: &flatline::session::Session) -> anyhow::Result<()> {
//! use flatline::scp::Handle;
//!
//! let channel = session.channel_open_default().await?;
//! let mut scp = Handle::new(channel);
//!
//! scp.start_sending("/tmp/", false).await?;
//! let mut file = scp.start_sending_file(0o644, 5, "hello.txt").await?;
//! file.send(b"hello").await?;
//! file.finish().await?;
//! scp.close().await?;
//! # Ok(())
//! # }
//! ```

use super::channel::{self, BufferChannel};
use crate::error;
use regex::Regex;
use snafu::{OptionExt, ResultExt};
use std::str::Utf8Error;

/// Errors reported by the SCP control protocol.
#[derive(Debug, snafu::Snafu)]
pub enum Error {
    /// The remote side sent a non-fatal warning (response code `1`).
    #[snafu(display("secure copy protocol failure: {}", msg))]
    Failure {
        /// The message the remote side reported.
        msg: String,
    },
    /// The remote side sent a fatal error (response code `2`).
    #[snafu(display("secure copy protocol critical: {}", msg))]
    Critical {
        /// The message the remote side reported.
        msg: String,
    },
    /// A response did not match what the protocol requires at this point.
    #[snafu(display("secure copy protocol: {}", detail))]
    UnexpectedResponse {
        /// Description of what was received instead.
        detail: String,
    },
    /// An error message from the peer was not valid UTF-8.
    #[snafu(display("Unexpected error message: {}", source))]
    UnexpectedErrorMessage {
        /// The UTF-8 decoding error.
        source: Utf8Error,
    },
    /// A path could not be shell-quoted, so it was not sent to the server.
    #[snafu(display("Invalid target name: {}", source))]
    InvalidTargetName {
        /// The shell-quoting error.
        source: shlex::QuoteError,
    },
}

impl Error {
    /// Returns `true` for errors the remote side reported ([`Error::Failure`]
    /// and [`Error::Critical`]), i.e. a failure the server knows about, as
    /// opposed to a locally detected protocol violation.
    pub fn is_broken(&self) -> bool {
        matches!(self, Error::Failure { .. } | Error::Critical { .. })
    }
}

impl From<Error> for error::Error {
    fn from(value: Error) -> Self {
        let e = super::Error::from(value);
        e.into()
    }
}

/// Receives a single file announced by the remote `scp -f` process.
///
/// Created by [`Handle::start_receiving`] after the peer's `C` record has
/// been parsed. Read chunks with [`receive`](Self::receive) until
/// [`is_finished`](Self::is_finished) returns `true`; the final chunk has the
/// trailing NUL byte stripped and an acknowledgement is sent back to the peer.
#[derive(Debug)]
pub struct FileReceiver<'a> {
    stream: &'a mut Handle,
    mode: u16,
    size: u64,
    file_name: String,
    received: u64,
}

impl<'a> FileReceiver<'a> {
    fn new(stream: &'a mut Handle, mode: u16, size: u64, file_name: String) -> Self {
        Self {
            stream,
            mode,
            size,
            file_name,
            received: 0,
        }
    }

    /// Returns the file name as announced in the peer's `C` record.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// Returns the file mode (permission bits) from the `C` record,
    /// e.g. `0o644`.
    pub fn mode(&self) -> u16 {
        self.mode
    }

    /// Returns `true` once the whole file (plus its trailing NUL byte) has
    /// been read.
    pub fn is_finished(&self) -> bool {
        debug_assert!(self.received <= self.size + 1);
        self.received == self.size + 1
    }

    /// Reads the next chunk of file data.
    ///
    /// Returns an empty vector once the file is finished. On the final chunk
    /// this also sends the completion acknowledgement to the peer, so the
    /// caller should stop as soon as [`is_finished`](Self::is_finished) is
    /// `true`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnexpectedResponse`] if the
    /// peer sends more data than the announced file size, or if the trailing
    /// NUL byte is missing.
    pub async fn receive(&mut self) -> error::Result<Vec<u8>> {
        let mut data = self.stream.receive().await?;
        self.received += data.len() as u64;

        if self.received == self.size + 1 {
            if data[data.len() - 1] != 0 {
                return Err(UnexpectedResponseSnafu {
                    detail: "Unexpected file data ending",
                }
                .build()
                .into());
            }
            data.remove(data.len() - 1);
            self.stream.send(&[0]).await?;
            self.stream.flush().await?;
        } else if self.received > self.size + 1 {
            return Err(UnexpectedResponseSnafu {
                detail: format!(
                    "Unexpected file data: received={}, size={}",
                    self.received, self.size
                ),
            }
            .build()
            .into());
        }

        Ok(data)
    }
}

/// Sends a single file to the remote `scp -t` process.
///
/// Created by [`Handle::start_sending_file`]. Write the file contents with
/// [`send`](Self::send), then call [`finish`](Self::finish) exactly once to
/// write the terminating NUL byte and wait for the peer's acknowledgement.
#[derive(Debug)]
pub struct FileSender<'a> {
    stream: &'a mut Handle,
}

impl<'a> FileSender<'a> {
    fn new(stream: &'a mut Handle) -> Self {
        Self { stream }
    }

    /// Writes the next chunk of file data to the peer.
    pub async fn send(&mut self, data: &[u8]) -> error::Result<()> {
        self.stream.send(data).await
    }

    /// Completes the transfer: sends the terminating NUL byte and waits for
    /// the peer to acknowledge the file.
    ///
    /// The file size announced in [`Handle::start_sending_file`] must match
    /// the total number of bytes passed to [`send`](Self::send).
    pub async fn finish(&mut self) -> error::Result<()> {
        self.stream.send(&[0]).await?;
        self.stream.flush().await?;

        self.stream.wait_for_response().await?;

        Ok(())
    }
}

/// A channel speaking the legacy SCP control protocol.
///
/// One `Handle` drives one `scp` invocation: call [`start_receiving`] to fetch
/// a file or [`start_sending`] to upload, then [`close`] the channel when
/// done.
///
/// [`start_receiving`]: Self::start_receiving
/// [`start_sending`]: Self::start_sending
/// [`close`]: Self::close
#[derive(derive_more::Debug)]
pub struct Handle {
    #[debug(skip)]
    channel: BufferChannel,
}

impl Handle {
    async fn flush(&mut self) -> error::Result<()> {
        self.channel.flush().await
    }

    async fn send(&mut self, data: &[u8]) -> error::Result<()> {
        self.channel.send(data).await
    }

    /// Sends EOF and closes the underlying channel.
    pub async fn close(self) -> error::Result<()> {
        self.channel.close().await
    }

    async fn receive(&mut self) -> error::Result<Vec<u8>> {
        let data = self.channel.fill().await?.to_vec();

        self.channel.consumer_read_buffer(data.len());

        Ok(data)
    }

    /// Starts fetching `target` from the remote host.
    ///
    /// Executes `scp -f <target>` on the remote side, performs the initial
    /// handshake and parses the peer's `C` record, returning a
    /// [`FileReceiver`] for reading the announced file. `target` is
    /// shell-quoted before being embedded in the command.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnexpectedResponse`] if the peer's `C` record does
    /// not match the expected `C<mode> <size> <name>` format, or
    /// [`Error::InvalidTargetName`] if `target` cannot be shell-quoted.
    pub async fn start_receiving(&mut self, target: &str) -> error::Result<FileReceiver<'_>> {
        let target = shlex::try_quote(target)
            .context(InvalidTargetNameSnafu)?
            .to_string();
        let command = format!("scp -f {}", target);

        self.channel
            .channel_mut()
            .request_exec(true, command)
            .await?;

        self.send(&[0]).await?;

        let line = self.channel.read_line_lf().await?;

        let len = line.len();

        let line =
            std::str::from_utf8(&line[..line.len() - 1]).context(UnexpectedErrorMessageSnafu)?;

        let re = Regex::new(r"^C(\d{4})\s+(\d+)\s+(.+)$").expect("Failed to compile regex");

        let caps = re.captures(line).context(UnexpectedResponseSnafu {
            detail: format!("Unexpected response line: {}", line),
        })?;

        let mode = u16::from_str_radix(&caps[1], 8)
            .ok()
            .context(UnexpectedResponseSnafu {
                detail: format!("Unexpected response file mode: {}", &caps[1]),
            })?;
        let size = caps[2]
            .parse::<u64>()
            .ok()
            .context(UnexpectedResponseSnafu {
                detail: format!("Unexpected response size: {}", &caps[2]),
            })?;
        let filename = caps[3].to_string();

        self.send(&[0]).await?;
        self.flush().await?;

        self.channel.consumer_read_buffer(len);

        Ok(FileReceiver::new(self, mode, size, filename))
    }

    /// Starts uploading to `target` on the remote host.
    ///
    /// Executes `scp -t <target>` (with `-r` when `recursive` is set) and
    /// waits for the peer's initial acknowledgement. Follow up with
    /// [`enter`], [`set_timestamp`] and [`start_sending_file`] to build the
    /// transfer, then [`close`].
    ///
    /// [`enter`]: Self::enter
    /// [`set_timestamp`]: Self::set_timestamp
    /// [`start_sending_file`]: Self::start_sending_file
    /// [`close`]: Self::close
    pub async fn start_sending(&mut self, target: &str, recursive: bool) -> error::Result<()> {
        let target = shlex::try_quote(target)
            .context(InvalidTargetNameSnafu)?
            .to_string();
        let command = if recursive {
            format!("scp -t -r {}", target)
        } else {
            format!("scp -t {}", target)
        };

        self.channel
            .channel_mut()
            .request_exec(true, command)
            .await?;

        self.wait_for_response().await
    }

    async fn wait_for_response(&mut self) -> error::Result<()> {
        let result = self.channel.fill_exact(1).await?;

        let code = result[0];

        self.channel.consumer_read_buffer(1);
        if code == 0 {
            Ok(())
        } else if code == 1 {
            let line = self.channel.read_line_lf().await?;
            let msg = std::str::from_utf8(&line[..line.len() - 1])
                .context(UnexpectedErrorMessageSnafu)?
                .to_string();

            let len = line.len();

            self.channel.consumer_read_buffer(len);

            Err(FailureSnafu { msg }.build().into())
        } else if code == 2 {
            let line = self.channel.read_line_lf().await?;
            let msg = std::str::from_utf8(&line[..line.len() - 1])
                .context(UnexpectedErrorMessageSnafu)?
                .to_string();

            let len = line.len();

            self.channel.consumer_read_buffer(len);

            Err(CriticalSnafu { msg }.build().into())
        } else {
            Err(UnexpectedResponseSnafu {
                detail: format!("Unexpected response code {}", code),
            }
            .build()
            .into())
        }
    }

    /// Sends a `T` record setting the modification and access timestamps for
    /// the next file, and waits for the peer's acknowledgement.
    ///
    /// Times are split into whole seconds and microseconds; a zero
    /// `*usec` field is sent as-is.
    pub async fn set_timestamp(
        &mut self,
        mtime_sec: u64,
        mtime_usec: u64,
        atime_sec: u64,
        atime_usec: u64,
    ) -> error::Result<()> {
        let line = format!(
            "T{} {} {} {}\n",
            mtime_sec, mtime_usec, atime_sec, atime_usec
        );
        self.channel.send(line.as_bytes()).await?;
        self.channel.flush().await?;
        self.wait_for_response().await?;

        Ok(())
    }

    /// Announces a file with a `C` record and waits for the peer's
    /// acknowledgement.
    ///
    /// `permission` is the file mode (e.g. `0o644`), `size` the exact number
    /// of bytes that will follow, and `file_name` is shell-quoted before use.
    /// The returned [`FileSender`] borrows the handle mutably, so
    /// [`finish`](FileSender::finish) it before issuing further commands.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidTargetName`] if `file_name` cannot be
    /// shell-quoted, or [`Error::Failure`]/[`Error::Critical`] if the peer
    /// rejects the file.
    pub async fn start_sending_file(
        &mut self,
        permission: u16,
        size: u64,
        file_name: &str,
    ) -> error::Result<FileSender<'_>> {
        let file_name = shlex::try_quote(file_name)
            .context(InvalidTargetNameSnafu)?
            .to_string();
        let line = format!("C{:04o} {} {}\n", permission, size, file_name);

        self.channel.send(line.as_bytes()).await?;
        self.channel.flush().await?;
        self.wait_for_response().await?;

        Ok(FileSender::new(self))
    }

    /// Sends a `D` record to enter a directory (create it on the receiving
    /// side) and waits for the peer's acknowledgement.
    ///
    /// Match every `enter` with a corresponding [`exit`] before closing.
    ///
    /// [`exit`]: Self::exit
    pub async fn enter(&mut self, permission: u16, target: &str) -> error::Result<()> {
        let target = shlex::try_quote(target)
            .context(InvalidTargetNameSnafu)?
            .to_string();
        let line = format!("D{:04o} 0 {}\n", permission, target);
        self.channel.send(line.as_bytes()).await?;
        self.channel.flush().await?;

        self.wait_for_response().await?;

        Ok(())
    }

    /// Sends an `E` record to leave the directory entered by [`enter`] and
    /// waits for the peer's acknowledgement.
    ///
    /// [`enter`]: Self::enter
    pub async fn exit(&mut self) -> error::Result<()> {
        self.channel.send("E\n".as_bytes()).await?;
        self.channel.flush().await?;
        self.wait_for_response().await?;
        Ok(())
    }

    /// Wraps a freshly opened channel in a [`Handle`].
    ///
    /// The channel must not have any command executed on it yet: the first
    /// thing the handle does is `exec` the remote `scp` process.
    pub fn new(channel: channel::Channel) -> Self {
        Self {
            channel: BufferChannel::new(channel),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test::*;
    use openssl::md::Md;
    use openssl::md_ctx::MdCtx;

    #[tokio::test]
    #[ignore = "requires a live SSH server configured in Test.toml"]
    async fn test_sending_file() -> anyhow::Result<()> {
        let _ = tracing_subscriber::fmt::try_init();

        let config = Config::load().await?;
        let session = config.open_session().await?;

        session.request_authentication().await?;
        config.authenticate_password(&session).await?;

        let mut file = vec![0; 16 * 1024];

        openssl::rand::rand_bytes(&mut file[..])?;

        let md5 = {
            let mut ctx = MdCtx::new()?;
            ctx.digest_init(Md::md5())?;
            ctx.digest_update(&file[..])?;
            let mut md5 = vec![0; ctx.size()];

            ctx.digest_final(&mut md5)?;

            hex::encode(&md5[..])
        };

        let name = "test.bin";

        let channel = session.channel_open(1024 * 1024, 30000).await?;
        let mut stream = Handle::new(channel);

        stream.start_sending("/tmp/", true).await?;
        stream.enter(0o755, "scp").await?;

        let mut file_sender = stream
            .start_sending_file(0o655, file.len() as u64, name)
            .await?;

        file_sender.send(&file).await?;
        file_sender.finish().await?;

        stream.exit().await?;
        stream.close().await?;

        {
            let mut channel = session.channel_open(1024 * 1024, 30000).await?;
            channel
                .request_exec(true, "md5sum /tmp/scp/test.bin")
                .await?;
            loop {
                match channel.receive().await? {
                    channel::Message::Close => {
                        tracing::info!("channel.close");
                        break;
                    }
                    channel::Message::Eof => {}
                    channel::Message::Stdout(data) => {
                        let data = String::from_utf8(data)?;
                        assert!(data.starts_with(md5.as_str()));
                    }
                    channel::Message::Stderr(_) => {}
                    channel::Message::Exit(_) => {}
                    channel::Message::FlowControl { .. } => {}
                    channel::Message::WindowChange { .. } => {}
                }
            }
        }

        {
            let channel = session.channel_open_default().await?;
            let mut stream = Handle::new(channel);
            let mut file_receiver = stream.start_receiving("/tmp/scp/test.bin").await?;

            let mut bytes = vec![];

            while !file_receiver.is_finished() {
                bytes.extend_from_slice(&file_receiver.receive().await?);
            }

            assert_eq!(bytes, file);
        }

        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a live SSH server configured in Test.toml"]
    async fn test_create_directory() -> anyhow::Result<()> {
        let _ = tracing_subscriber::fmt::try_init();
        let target = "test_scp";

        let config = Config::load().await?;
        let session = config.open_session().await?;

        session.send_debug_message(false, "DEBUG  NOW").await?;
        session.send_debug_message(false, "DEBUG  NOW").await?;
        session.send_debug_message(false, "DEBUG  NOW").await?;
        session.send_debug_message(false, "DEBUG  NOW").await?;

        session.request_authentication().await?;
        config.authenticate_password(&session).await?;

        let channel = session.channel_open(1024 * 1024, 30000).await?;
        let mut stream = Handle::new(channel);

        stream.start_sending("/tmp/", true).await?;
        stream.enter(0o755, target).await?;
        stream.exit().await?;
        stream.close().await?;
        Ok(())
    }
}
