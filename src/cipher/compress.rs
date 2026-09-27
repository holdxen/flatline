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

#[cfg(test)]
mod test {
    use super::*;

    fn encoder(name: &str) -> Box<dyn Encode + Send> {
        new_encode_by_name(name).expect("unknown encoder")()
    }

    fn decoder(name: &str) -> Box<dyn Decode + Send> {
        new_decode_by_name(name).expect("unknown decoder")()
    }

    /// Compresses one packet payload and returns the wire bytes.
    fn compress(name: &str, payload: &[u8]) -> Vec<u8> {
        let mut enc = encoder(name);
        enc.update(payload).expect("update failed");
        enc.finalize().expect("finalize failed")
    }

    #[test]
    fn none_passes_payload_through_unchanged() {
        let payload = b"the quick brown fox jumps over the lazy dog".repeat(10);
        assert_eq!(compress("none", &payload), payload);
    }

    #[test]
    fn zlib_alters_the_bytes_it_emits() {
        let payload = b"the quick brown fox jumps over the lazy dog".repeat(10);
        assert_ne!(compress("zlib", &payload), payload);
    }

    #[test]
    fn none_reports_its_name_and_no_auth_compression() {
        assert_eq!(encoder("none").name(), "none");
        assert_eq!(decoder("none").name(), "none");
        assert!(!encoder("none").compress_in_authentication());
        assert!(!decoder("none").compress_in_authentication());
    }

    #[test]
    fn delayed_compression_flag_matches_openssh_semantics() {
        // Plain zlib compresses during authentication; OpenSSH's variant
        // waits until authentication has finished.
        assert!(encoder("zlib").compress_in_authentication());
        assert!(decoder("zlib").compress_in_authentication());
        assert!(!encoder("zlib@openssh.com").compress_in_authentication());
        assert!(!decoder("zlib@openssh.com").compress_in_authentication());
    }

    #[test]
    fn zlib_round_trips_a_payload() {
        let payload: Vec<u8> = (0..4096u32).map(|v| (v % 251) as u8).collect();

        let compressed = compress("zlib", &payload);
        let mut dec = decoder("zlib");
        dec.update(&compressed).expect("update failed");
        let out = dec.finalize().expect("finalize failed");

        assert_eq!(out, payload);
    }

    #[test]
    fn zlib_repeatedly_compresses_to_smaller_output() {
        let payload = vec![b'a'; 8192];
        let compressed = compress("zlib", &payload);
        assert!(
            compressed.len() < payload.len() / 8,
            "expected strong compression, got {} -> {}",
            payload.len(),
            compressed.len()
        );
    }

    #[test]
    fn zlib_state_is_kept_across_packets() {
        let mut enc = encoder("zlib");
        let mut dec = decoder("zlib");

        for chunk in [b"first packet".as_slice(), b"second packet", b"third"] {
            enc.update(chunk).unwrap();
            let wire = enc.finalize().unwrap();

            dec.update(&wire).unwrap();
            let out = dec.finalize().unwrap();
            assert_eq!(out, chunk);
        }
    }

    #[test]
    fn zlib_decode_of_garbage_errors() {
        let mut dec = decoder("zlib");
        // Not a zlib stream at all.
        let result = dec.update(&[0xde, 0xad, 0xbe, 0xef]);
        let failed = result.is_err() || dec.finalize().is_err();
        assert!(failed, "expected an error decoding garbage");
    }

    #[test]
    fn finalize_clears_the_pending_buffer() {
        let mut enc = encoder("none");
        enc.update(b"abc").unwrap();
        assert_eq!(enc.finalize().unwrap(), b"abc");
        // Second finalize with no intervening update yields nothing.
        assert!(enc.finalize().unwrap().is_empty());
    }

    #[test]
    fn encoders_and_decoders_agree_on_names() {
        for name in encode_all() {
            assert_eq!(encoder(name).name(), *name);
        }
        for name in decode_all() {
            assert_eq!(decoder(name).name(), *name);
        }
        assert_eq!(encode_all(), decode_all());
    }
}
