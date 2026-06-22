use crate::{error, ssh::buffer::Consumer};

#[derive(Debug)]
pub struct Packet {
    pub payload: Vec<u8>,
    pub padding: Vec<u8>,
    pub mac: Option<Vec<u8>>,
}

impl Packet {
    // pub fn parse(data: &[u8], mac: Option<Vec<u8>>) -> Option<Self> {
    //     let data = Buffer::from_slice(data);

    //     let padding_len = data.take_u8()?;

    //     let payload_len = data.len() - padding_len as usize;
    //     let payload = data.take_bytes(payload_len)?.to_vec();

    //     let padding = data.take_bytes(padding_len as usize)?.to_vec();

    //     Some(Packet {
    //         payload,
    //         padding,
    //         mac,
    //     })
    // }

    pub fn parse(data: &[u8], mac: Option<Vec<u8>>) -> Result<Self, error::Error> {

        let mut consumer = Consumer::new(data);

        let padding_len = consumer.consume_u8()?;

        let payload_len = consumer.peek().len() - padding_len as usize;
        let payload = consumer.consume_bytes(payload_len)?.to_vec();

        let padding = consumer.consume_bytes(padding_len as usize)?.to_vec();


        Ok(Packet {
            payload,
            padding,
            mac,
        })
    }
}
