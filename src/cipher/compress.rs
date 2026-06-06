use super::*;
use crate::error::{builder, Result};
use flate2::{Compress, Compression, Decompress, Status};
use indexmap::IndexMap;
use std::mem;

algo_list!(
    encode_all,
    new_encode_all,
    new_encode_by_name,
    dyn Encode + Send,
    "zlib" => ZEncoder::new(true),
    "zlib@openssh.com" => ZEncoder::new(false),
    "none" => Never::default(),
);

algo_list!(
    decode_all,
    new_decode_all,
    new_decode_by_name,
    dyn Decode + Send,
    "zlib" => ZDecoder::new(true),
    "zlib@openssh.com" => ZDecoder::new(false),
    "none" => Never::default(),
);

pub fn none_encode() -> AlgoFactory<dyn Encode + Send> {
    create_factory!(Never::default())
}

pub fn none_decode() -> AlgoFactory<dyn Decode + Send> {
    create_factory!(Never::default())
}

pub trait Encode {
    fn compress_in_auth(&self) -> bool;
    fn update(&mut self, data: &[u8]) -> Result<()>;
    fn finalize(&mut self) -> Result<Vec<u8>>;
}

pub trait Decode {
    fn compress_in_auth(&self) -> bool;
    fn update(&mut self, data: &[u8]) -> Result<()>;
    fn finalize(&mut self) -> Result<Vec<u8>>;
}

impl Encode for Never {
    fn compress_in_auth(&self) -> bool {
        false
    }

    fn update(&mut self, data: &[u8]) -> Result<()> {
        self.buf.extend(data);
        Ok(())
    }

    fn finalize(&mut self) -> Result<Vec<u8>> {
        Ok(mem::take(&mut self.buf))
    }
}

impl Decode for Never {
    fn compress_in_auth(&self) -> bool {
        false
    }

    fn update(&mut self, data: &[u8]) -> Result<()> {
        self.buf.extend(data);
        Ok(())
    }

    fn finalize(&mut self) -> Result<Vec<u8>> {
        Ok(mem::take(&mut self.buf))
    }
}

#[derive(Default)]
struct Never {
    buf: Vec<u8>,
}

struct ZEncoder {
    compress_in_auth: bool,
    encoder: Compress,
    buf: Vec<u8>,
}

impl ZEncoder {
    fn new(compress_in_auth: bool) -> Self {
        Self {
            compress_in_auth,
            encoder: Compress::new(Compression::default(), true),
            buf: vec![],
        }
    }
}

impl Encode for ZEncoder {
    fn compress_in_auth(&self) -> bool {
        self.compress_in_auth
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
                _ => builder::CompressFailed.fail(), //Err(Error::CompressFailed),
            };
        }
    }

    fn finalize(&mut self) -> Result<Vec<u8>> {
        Ok(mem::take(&mut self.buf))
    }
}

struct ZDecoder {
    compress_in_auth: bool,
    decoder: Decompress,
    buf: Vec<u8>,
}

impl ZDecoder {
    fn new(compress_in_auth: bool) -> Self {
        Self {
            compress_in_auth,
            decoder: Decompress::new(true),
            buf: vec![],
        }
    }
}

impl Decode for ZDecoder {
    fn compress_in_auth(&self) -> bool {
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
                _ => builder::CompressFailed.fail(), //Err(Error::CompressFailed),
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
}
