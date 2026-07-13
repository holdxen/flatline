use super::channel::{self, BufferChannel};
use crate::error;
use regex::Regex;
use snafu::{OptionExt, ResultExt};
use std::str::Utf8Error;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("secure copy protocol failure: {}", msg))]
    Failure { msg: String },
    #[snafu(display("secure copy protocol critical: {}", msg))]
    Critical { msg: String },
    #[snafu(display("secure copy protocol: {}", detail))]
    UnexpectedResponse { detail: String },
    #[snafu(display("Unexpected error message: {}", source))]
    UnexpectedErrorMessage { source: Utf8Error },
    #[snafu(display("Invalid target name: {}", source))]
    InvalidTargetName { source: shlex::QuoteError },
}

impl Error {
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

    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    pub fn mode(&self) -> u16 {
        self.mode
    }

    pub fn is_finished(&self) -> bool {
        debug_assert!(self.received <= self.size + 1);
        self.received == self.size + 1
    }

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

#[derive(Debug)]
pub struct FileSender<'a> {
    stream: &'a mut Handle,
}

impl<'a> FileSender<'a> {
    fn new(stream: &'a mut Handle) -> Self {
        Self { stream }
    }
    pub async fn send(&mut self, data: &[u8]) -> error::Result<()> {
        self.stream.send(data).await
    }
    pub async fn finish(&mut self) -> error::Result<()> {
        self.stream.send(&[0]).await?;
        self.stream.flush().await?;

        self.stream.wait_for_response().await?;

        Ok(())
    }
}

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

    pub async fn close(self) -> error::Result<()> {
        self.channel.close().await
    }

    async fn receive(&mut self) -> error::Result<Vec<u8>> {
        let data = self.channel.fill().await?.to_vec();

        self.channel.consumer_read_buffer(data.len());

        Ok(data)
    }

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

    pub async fn exit(&mut self) -> error::Result<()> {
        self.channel.send("E\n".as_bytes()).await?;
        self.channel.flush().await?;
        self.wait_for_response().await?;
        Ok(())
    }

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
    async fn test_sending_file() -> anyhow::Result<()> {
        tracing_subscriber::fmt::init();

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
    async fn test_create_directory() -> anyhow::Result<()> {
        tracing_subscriber::fmt::init();
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
