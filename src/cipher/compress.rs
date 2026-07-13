//! SSH packet compression (RFC 4253, section 6.2).
//!
//! [`Encode`] compresses outgoing packet payloads before they are encrypted
//! and [`Decode`] decompresses incoming payloads after decryption. Each
//! direction of the connection negotiates its own algorithm, and the
//! compression state is established when the new keys take effect and then
//! persists for the rest of the connection.

use crate::error::Result;
use flate2::{Compress, Compression, Decompress, Status};
use std::mem;

use super::Factory;
use indexmap::IndexMap;

algo_list!(
    "compression algorithm",
    encode_all,
    new_encode_all,
    new_encode_by_name,
    dyn Encode + Send,
    "zlib" => ZEncoder::new("zlib", true),
    "zlib@openssh.com" => ZEncoder::new("zlib@openssh.com", false),
    "none" => Never::default(),
);

algo_list!(
    "compression algorithm",
    decode_all,
    new_decode_all,
    new_decode_by_name,
    dyn Decode + Send,
    "zlib" => ZDecoder::new("zlib", true),
    "zlib@openssh.com" => ZDecoder::new("zlib@openssh.com", false),
    "none" => Never::default(),
);

/// Creates a factory for the `none` encoder, which passes packet payloads
/// through unchanged instead of compressing them.
pub fn none_encode() -> Factory<dyn Encode + Send> {
    create_factory!(Never::default())
}

/// Creates a factory for the `none` decoder, which passes packet payloads
/// through unchanged instead of decompressing them.
pub fn none_decode() -> Factory<dyn Decode + Send> {
    create_factory!(Never::default())
}

/// Compresses outgoing SSH packet payloads before they are encrypted.
///
/// One instance serves one direction of the connection; `update` and
/// `finalize` are invoked once per packet by the packet layer.
pub trait Encode {
    /// Returns the SSH compression algorithm name (`zlib`,
    /// `zlib@openssh.com` or `none`) as it appears in `SSH_MSG_KEXINIT`.
    fn name(&self) -> &str;
    /// Returns whether payloads are also compressed while user authentication
    /// is still in progress.
    ///
    /// This encodes the "delayed compression" distinction: plain `zlib`
    /// compresses from the moment the keys take effect, including the user
    /// authentication exchange, while OpenSSH's `zlib@openssh.com` sends
    /// everything uncompressed until authentication has succeeded and only
    /// compresses from then on.
    fn compress_in_authentication(&self) -> bool;
    /// Compresses one packet payload, buffering the output until
    /// [`finalize`](Encode::finalize) is called.
    ///
    /// Returns an error if the underlying compression stream fails.
    fn update(&mut self, data: &[u8]) -> Result<()>;
    /// Returns all compressed output produced since the last call and clears
    /// the output buffer.
    ///
    /// The compression stream itself stays stateful across packets.
    fn finalize(&mut self) -> Result<Vec<u8>>;
}

/// Decompresses incoming SSH packet payloads after they are decrypted.
///
/// One instance serves one direction of the connection; `update` and
/// `finalize` are invoked once per packet by the packet layer.
pub trait Decode {
    /// Returns the SSH compression algorithm name (`zlib`,
    /// `zlib@openssh.com` or `none`) as it appears in `SSH_MSG_KEXINIT`.
    fn name(&self) -> &str;
    /// Returns whether payloads are also decompressed while user
    /// authentication is still in progress.
    ///
    /// This encodes the "delayed compression" distinction: plain `zlib`
    /// compresses from the moment the keys take effect, including the user
    /// authentication exchange, while OpenSSH's `zlib@openssh.com` sends
    /// everything uncompressed until authentication has succeeded and only
    /// compresses from then on.
    fn compress_in_authentication(&self) -> bool;
    /// Decompresses one packet payload, buffering the output until
    /// [`finalize`](Decode::finalize) is called.
    ///
    /// Returns an error if the underlying decompression stream fails, which
    /// can also indicate a corrupt or malicious payload.
    fn update(&mut self, data: &[u8]) -> Result<()>;
    /// Returns all decompressed output produced since the last call and
    /// clears the output buffer.
    ///
    /// The decompression stream itself stays stateful across packets.
    fn finalize(&mut self) -> Result<Vec<u8>>;
}

impl Encode for Never {
    fn compress_in_authentication(&self) -> bool {
        false
    }

    fn update(&mut self, data: &[u8]) -> Result<()> {
        self.buf.extend(data);
        Ok(())
    }

    fn finalize(&mut self) -> Result<Vec<u8>> {
        Ok(mem::take(&mut self.buf))
    }

    fn name(&self) -> &str {
        "none"
    }
}

impl Decode for Never {
    fn compress_in_authentication(&self) -> bool {
        false
    }

    fn update(&mut self, data: &[u8]) -> Result<()> {
        self.buf.extend(data);
        Ok(())
    }

    fn finalize(&mut self) -> Result<Vec<u8>> {
        Ok(mem::take(&mut self.buf))
    }

    fn name(&self) -> &str {
        "none"
    }
}

#[derive(Default)]
struct Never {
    buf: Vec<u8>,
}

struct ZEncoder {
    name: &'static str,
    compress_in_authentication: bool,
    encoder: Compress,
    buf: Vec<u8>,
}

impl ZEncoder {
    fn new(name: &'static str, compress_in_authentication: bool) -> Self {
        Self {
            name,
            compress_in_authentication,
            encoder: Compress::new(Compression::default(), true),
            buf: vec![],
        }
    }
}

impl Encode for ZEncoder {
    fn compress_in_authentication(&self) -> bool {
        self.compress_in_authentication
    }

    fn update(&mut self, data: &[u8]) -> Result<()> {
        let before = self.encoder.total_in() as usize;
        let data_len = data.len();
        let mut pos = 0;
        loop {
            let cap = ((data_len - pos) / 1024 + 1) * 1024;
            let mut tmp = Vec::with_capacity(cap);
            let status =
                self.encoder
                    .compress_vec(&data[pos..], &mut tmp, flate2::FlushCompress::Partial);
            return match status {
                Ok(Status::Ok) => {
                    self.buf.extend(&tmp);
                    let after = self.encoder.total_in() as usize;
                    pos = after - before;
                    if pos < data_len || tmp.len() == cap {
                        continue;
                    }
                    Ok(())
                }
                _ => super::CompressSnafu.fail()?, //Err(Error::CompressFailed),
            };
        }
    }

    fn finalize(&mut self) -> Result<Vec<u8>> {
        Ok(mem::take(&mut self.buf))
    }

    fn name(&self) -> &str {
        self.name
    }
}

struct ZDecoder {
    name: &'static str,
    compress_in_auth: bool,
    decoder: Decompress,
    buf: Vec<u8>,
}

impl ZDecoder {
    fn new(name: &'static str, compress_in_auth: bool) -> Self {
        Self {
            name,
            compress_in_auth,
            decoder: Decompress::new(true),
            buf: vec![],
        }
    }
}

impl Decode for ZDecoder {
    fn compress_in_authentication(&self) -> bool {
        self.compress_in_auth
    }

    fn update(&mut self, data: &[u8]) -> Result<()> {
        let before = self.decoder.total_in();
        let mut pos = 0;
        loop {
            let cap = 1024 * 4;
            let mut tmp = Vec::with_capacity(cap);
            let status =
                self.decoder
                    .decompress_vec(&data[pos..], &mut tmp, flate2::FlushDecompress::Sync);
            pos = (self.decoder.total_in() - before) as usize;
            return match status {
                Ok(Status::Ok) => {
                    self.buf.extend(tmp);
                    continue;
                }
                Ok(_) => {
                    self.buf.extend(tmp);
                    if pos < data.len() {
                        continue;
                    }
                    Ok(())
                }
                _ => super::CompressSnafu.fail()?, //Err(Error::CompressFailed),
            };
        }

        // let mut len = (data.len() / 1024 + 1) * 1024;
        // let out_before = self.decoder.total_out() as usize;
        // let in_before = self.decoder.total_in() as usize;
        // let mut tmp = vec![0; len];
        // loop {
        //     // let status = self
        //     //     .decoder
        //     //     .decompress_vec(data, &mut tmp, flate2::FlushDecompress::None);

        //     let input_index = self.decoder.total_in() as usize - in_before;
        //     let output_index = self.decoder.total_out() as usize - out_before;
        //     let status = self.decoder.decompress(
        //         &data[input_index..],
        //         &mut tmp[output_index..],
        //         flate2::FlushDecompress::Sync,
        //     );

        //     return match status {
        //         Ok(Status::Ok) => {
        //             len *= 2;
        //             tmp.resize(len, 0);
        //             continue;
        //         }
        //         Ok(_) => {
        //             self.buf.extend(tmp);
        //             Ok(())
        //         }
        //         _ => Err(Error::CompressFailed),
        //     };
        // }
    }

    fn finalize(&mut self) -> Result<Vec<u8>> {
        Ok(mem::take(&mut self.buf))
    }

    fn name(&self) -> &str {
        self.name
    }
}
