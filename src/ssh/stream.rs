use snafu::ResultExt;
use tokio::io::{AsyncRead, AsyncWrite};

use super::*;
use crate::{
    cipher::{
        compress::{Decode, Encode},
        crypt::{Decrypt, Encrypt},
        mac::Mac,
    },
    error::{self, builder},
    ssh::buffer::{Consumer, Producer},
    stream::BufferStream,
};
use rand::RngExt;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("payload is too long"))]
    PayloadTooLong { maximum: usize, actual: usize },
    #[snafu(display("packet is too long"))]
    PacketTooLong { maximum: usize, actual: usize },
    // #[snafu(display("padding length is incorrect"))]
    // PaddingLengthIncorrect,
    #[snafu(display("Payload is empty"))]
    PayloadIsEmpty,
    #[snafu(display("Unexpected block size: {}", size))]
    UnexpectBlockSize { size: usize },
    #[snafu(display("MAC verification failed"))]
    MacVerificationFailed,
}

pub trait Stream: Send {
    fn send_payload(&mut self, payload: &[u8]) -> impl Future<Output = error::Result<()>> + Send;
    fn recv_packet(&mut self) -> impl Future<Output = error::Result<msg::Packet>> + Send;
}

#[derive(Default, Debug)]
pub struct NormalEndpoint {
    pub kex_strict: bool,
    pub ext: bool,
    pub sequence_number: u32,
}

pub struct PlainStream<T>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    socket: BufferStream<T>,
    client: NormalEndpoint,
    server: NormalEndpoint,
}

impl<T> PlainStream<T>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    pub fn client_mut(&mut self) -> &mut NormalEndpoint {
        &mut self.client
    }
    pub fn server_mut(&mut self) -> &mut NormalEndpoint {
        &mut self.server
    }
    pub fn new(socket: BufferStream<T>) -> Self {
        Self {
            socket,
            client: NormalEndpoint::default(),
            server: NormalEndpoint::default(),
        }
    }
    pub fn upgrade(
        self,
        encrypt: Box<dyn Encrypt + Send>,
        decrypt: Box<dyn Decrypt + Send>,
        encode: Box<dyn Encode + Send>,
        decode: Box<dyn Decode + Send>,
        calculator: Box<dyn Mac + Send>,
        verify: Box<dyn Mac + Send>,
    ) -> CipherStream<T> {
        CipherStream {
            socket: self.socket,
            encrypt,
            decrypt,
            decode,
            encode,
            calculator,
            verify,
            authenticated: false,
            client: self.client,
            server: self.server,
            state: State::None,
        }
    }
}

impl<T> Stream for PlainStream<T>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    async fn send_payload(&mut self, payload: &[u8]) -> error::Result<()> {
        assert!(!payload.is_empty(), "payload must not be empty");
        let mut reset = false;
        if payload[0] == protocol::SSH_MSG_NEWKEYS
            && self.client.kex_strict
            && self.server.kex_strict
        {
            reset = true;
        }

        let payload_len = payload.len();
        if payload_len > protocol::MAX_PACKET_PAYLOAD_LENGTH {
            PayloadTooLongSnafu {
                maximum: protocol::MAX_PACKET_PAYLOAD_LENGTH,
                actual: payload_len,
            }
            .fail()?;
        }

        let block_size = 8;
        // let integerated_mac = self.hostkey_cts.as_ref().map(|v| v.integrated_mac()) == Some(true);

        let mut padding_len = block_size - ((4 + 1 + payload_len) % block_size);
        if padding_len < protocol::MIN_PADDING_LENGTH {
            padding_len += block_size;
        }

        let packet_len = payload_len + padding_len + 1;
        if packet_len + 4 > protocol::MAX_PACKET_LENGTH {
            PacketTooLongSnafu {
                maximum: protocol::MAX_PACKET_LENGTH,
                actual: packet_len + 4,
            }
            .fail()?;
        }

        let mut rand_padding = vec![0u8; padding_len];

        {
            let mut rng = rand::rng();
            rng.fill(&mut rand_padding);
        }

        let mut producer = Producer::with_capacity(packet_len + 4);

        producer.put_u32(packet_len as _);

        producer.put_u8(padding_len as u8);

        producer.put_bytes(payload);

        producer.put_bytes(rand_padding);

        self.socket
            .write(producer.as_bytes())
            .await
            .context(builder::IO)?;
        self.socket.flush().await.context(builder::IO)?;

        self.client.sequence_number = self.client.sequence_number.wrapping_add(1);
        if reset {
            self.client.sequence_number = 0;
        }

        Ok(())
    }

    async fn recv_packet(&mut self) -> error::Result<msg::Packet> {
        // let size = self.stream.read_exact(size_of::<u32>()).await?;
        let size = self.socket.fill(4).await.context(builder::IO)?;
        let size = u32::from_be_bytes(size.try_into().unwrap());
        // let mut size = Buffer::from_vec(size);
        // let size = size.take_u32().unwrap();
        if size as usize + 4 > protocol::MAX_PACKET_LENGTH {
            return Err(PacketTooLongSnafu {
                maximum: protocol::MAX_PACKET_LENGTH,
                actual: size as usize + 4,
            }
            .build()
            .into());
        }

        // let data = self.stream.read_exact(size as _).await?;
        let data = self
            .socket
            .fill(4 + size as usize)
            .await
            .context(builder::IO)?;

        let packet = msg::Packet::parse(&data[4..])?;

        if packet.payload.len() > protocol::MAX_PACKET_PAYLOAD_LENGTH {
            #[cfg(feature = "strict")]
            return Err(PayloadTooLongSnafu {
                maximum: protocol::MAX_PACKET_PAYLOAD_LENGTH,
                actual: packet.payload.len(),
            }
            .build()
            .into());

            #[cfg(not(feature = "strict"))]
            tracing::warn!(
                "Maybe payload is too long, but we ignore it because strict feature is not enabled. payload length: {}",
                packet.payload.len()
            );
        }
        if packet.payload.is_empty() {
            return Err(PayloadIsEmptySnafu.build().into());
        }

        self.socket.consume_read_buffer(4 + size as usize);

        self.server.sequence_number = self.server.sequence_number.wrapping_add(1);
        if packet.payload.is_empty() {
            return Err(PayloadIsEmptySnafu.build().into());
        }
        if packet.payload[0] == protocol::SSH_MSG_NEWKEYS
            && self.client.kex_strict
            && self.server.kex_strict
        {
            self.server.sequence_number = 0;
            // self.client.sequence_number = 0;
        }

        Ok(packet)
    }
}

pub struct CipherStream<T>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    socket: BufferStream<T>,
    encrypt: Box<dyn Encrypt + Send>,
    decrypt: Box<dyn Decrypt + Send>,
    decode: Box<dyn Decode + Send>,
    encode: Box<dyn Encode + Send>,
    calculator: Box<dyn Mac + Send>,
    verify: Box<dyn Mac + Send>,
    pub authenticated: bool,
    client: NormalEndpoint,
    server: NormalEndpoint,
    state: State,
}

impl<T> Stream for CipherStream<T>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    async fn send_payload(&mut self, mut payload: &[u8]) -> error::Result<()> {
        assert!(!payload.is_empty(), "payload must not be empty");
        let mut reset = false;
        if payload[0] == protocol::SSH_MSG_NEWKEYS
            && self.client.kex_strict
            && self.server.kex_strict
        {
            reset = true;
        }

        let mut payload_len = payload.len();
        if payload_len > protocol::MAX_PACKET_PAYLOAD_LENGTH {
            return Err(PayloadTooLongSnafu {
                maximum: protocol::MAX_PACKET_PAYLOAD_LENGTH,
                actual: payload_len,
            }
            .build()
            .into());
        }

        let tmp;
        if self.authenticated || self.encode.compress_in_authentication() {
            self.encode.update(payload)?;
            tmp = self.encode.finalize()?;
            payload = &tmp;
            payload_len = payload.len();
        }
        let block_size = self.encrypt.block_size();
        if self.encrypt.is_galois_counter_mode() {
            let mut padding_len = block_size - ((1 + payload_len) % block_size);
            if padding_len < protocol::MIN_PADDING_LENGTH {
                padding_len += block_size;
            }
            let mut producer = Producer::default();
            producer.put_u32((1 + payload_len + padding_len) as u32);
            producer.put_u8(padding_len as u8);
            producer.put_bytes(payload);
            producer.resize(producer.len() + padding_len, 0);
            {
                let mut rng = rand::rng();
                let pos = producer.len() - padding_len;
                rng.fill(&mut producer[pos..]);
            }

            self.encrypt
                .update_sequence_number(self.client.sequence_number)?;

            self.encrypt
                .additional_authenticated_data(&mut producer[..4])?;

            let mut encrypted = Vec::with_capacity(producer.len());
            self.encrypt.update(&producer[4..], &mut encrypted)?;
            self.encrypt.finalize(&mut encrypted)?;

            producer.resize(4, 0);
            producer.put_bytes(encrypted);
            producer.resize(producer.len() + self.encrypt.tag_len(), 0);

            let pos = producer.len() - self.encrypt.tag_len();
            self.encrypt.authentication_tag(&mut producer[pos..])?;

            self.socket.put_bytes(&producer[..]);
            self.socket.flush().await.context(builder::IO)?;
        } else {
            // let mut padding_len = block_size - ((4 + 1 + payload_len) % block_size);
            // if padding_len < protocol::MIN_PADDING_LENGTH {
            //     padding_len += block_size;
            // }

            // let mut producer = Producer::default();
            // producer.put_u32((1 + payload_len + padding_len) as u32);
            // producer.put_u8(padding_len as u8);
            // producer.put_bytes(payload);
            // producer.resize(producer.len() + padding_len, 0);

            // {
            //     let mut rng = rand::rng();
            //     let pos = producer.len() - padding_len;
            //     rng.fill(&mut producer[pos..]);
            // }

            // let mut encrypted = Vec::with_capacity(producer.len());
            // if self.calculator.encrypt_then_mac() {
            //     // encrypt
            //     self.encrypt.update(&producer[..], &mut encrypted)?;
            //     self.encrypt.finalize(&mut encrypted)?;

            //     // mac
            //     self.calculator
            //         .update(&self.client.sequence_number.to_be_bytes())?;
            //     self.calculator.update(&encrypted[..])?;
            //     let mac = self.calculator.finalize()?;

            //     // write
            //     self.socket.write(&encrypted).await.context(builder::IO)?;
            //     self.socket.write(&mac).await.context(builder::IO)?;
            // } else {
            //     // mac
            //     self.calculator
            //         .update(&self.client.sequence_number.to_be_bytes())?;
            //     self.calculator.update(&producer[..])?;
            //     let mac = self.calculator.finalize()?;

            //     // encrypt
            //     self.encrypt.update(&producer[..], &mut encrypted)?;
            //     self.encrypt.finalize(&mut encrypted)?;

            //     // write
            //     self.socket.write(&encrypted).await.context(builder::IO)?;
            //     self.socket.write(&mac).await.context(builder::IO)?;
            // }
            //
            if self.calculator.encrypt_then_mac() {
                // packet_length不加密
                let mut padding_len = block_size - ((1 + payload_len) % block_size);
                if padding_len < protocol::MIN_PADDING_LENGTH {
                    padding_len += block_size;
                }
                let mut producer = Producer::default();
                producer.put_u32((1 + payload_len + padding_len) as u32);
                producer.put_u8(padding_len as u8);
                producer.put_bytes(payload);
                producer.resize(producer.len() + padding_len, 0);
                {
                    let mut rng = rand::rng();
                    let pos = producer.len() - padding_len;
                    rng.fill(&mut producer[pos..]);
                }
                let mut encrypted = Vec::with_capacity(producer.len());
                self.encrypt.update(&producer[4..], &mut encrypted)?;
                self.encrypt.finalize(&mut encrypted)?;

                assert_eq!(producer.len(), encrypted.len() + 4);

                // mac
                self.calculator
                    .update(&self.client.sequence_number.to_be_bytes())?;
                self.calculator.update(&producer[..4])?;
                self.calculator.update(&encrypted[..])?;
                let mac = self.calculator.finalize()?;

                self.socket.put_bytes(&producer[..4]);
                self.socket.put_bytes(&encrypted[..]);
                self.socket.put_bytes(&mac);
                self.socket.flush().await.context(builder::IO)?;
            } else {
                let mut padding_len = block_size - ((4 + 1 + payload_len) % block_size);
                if padding_len < protocol::MIN_PADDING_LENGTH {
                    padding_len += block_size;
                }

                let mut producer = Producer::default();
                producer.put_u32((1 + payload_len + padding_len) as u32);
                producer.put_u8(padding_len as u8);
                producer.put_bytes(payload);
                producer.resize(producer.len() + padding_len, 0);

                {
                    let mut rng = rand::rng();
                    let pos = producer.len() - padding_len;
                    rng.fill(&mut producer[pos..]);
                }

                let mut encrypted = Vec::with_capacity(producer.len());
                // mac
                self.calculator
                    .update(&self.client.sequence_number.to_be_bytes())?;
                self.calculator.update(&producer[..])?;
                let mac = self.calculator.finalize()?;

                // encrypt
                self.encrypt.update(&producer[..], &mut encrypted)?;
                self.encrypt.finalize(&mut encrypted)?;

                // write
                self.socket.put_bytes(&encrypted);
                self.socket.put_bytes(&mac);

                self.socket.flush().await.context(builder::IO)?;
            }
        }

        self.client.sequence_number = self.client.sequence_number.wrapping_add(1);

        if reset {
            self.client.sequence_number = 0;
        }
        Ok(())

        // let mut mac = None;
        // {
        //     if !self.calculator.encrypt_then_mac() {
        //         self.calculator
        //             .update(&self.client.sequence_number.to_be_bytes())?;
        //         self.calculator.update(&producer[..])?;
        //         mac = Some(self.calculator.finalize()?);
        //     }
        // }
    }

    // async fn recv_packet(&mut self) -> error::Result<msg::Packet> {
    //     self.decrypt
    //         .update_sequence_number(self.server.sequence_number)?;
    //     if self.decrypt.is_galois_counter_mode() {
    //         let mut bytes = self
    //             .socket
    //             .fill(std::mem::size_of::<u32>())
    //             .await
    //             .context(builder::IO)?
    //             .to_vec();

    //         println!("recv_packet: bytes={:?}", bytes);
    //         self.decrypt.additional_authenticated_data(&mut bytes)?;
    //         let length = u32::from_be_bytes(bytes.try_into().unwrap());

    //         if length as usize + 4 > protocol::MAX_PACKET_LENGTH {
    //             return Err(PacketTooLongSnafu {
    //                 maximum: protocol::MAX_PACKET_LENGTH,
    //                 actual: length as usize + 4,
    //             }
    //             .build()
    //             .into());
    //         }

    //         let bytes = &self
    //             .socket
    //             .fill(4 + length as usize + self.decrypt.tag_len())
    //             .await
    //             .context(builder::IO)?[4..];

    //         let mut output = Vec::with_capacity(1024);

    //         self.decrypt
    //             .update(&bytes[..length as usize], &mut output)?;
    //         self.decrypt.authentication_tag(&bytes[length as usize..])?;
    //         self.decrypt.finalize(&mut output)?;
    //         let mut consumer = Consumer::new(&output);
    //         let padding_len = consumer.consume_u8()?;
    //         let content = consumer.consume_bytes(output.len() - padding_len as usize - 1)?;

    //         let mut packet = msg::Packet::default();
    //         {
    //             if self.authenticated || self.decode.compress_in_authentication() {
    //                 self.decode.update(content)?;
    //                 packet.payload = self.decode.finalize()?;
    //             } else {
    //                 packet.payload = content.to_vec();
    //             }
    //             if packet.payload.len() > protocol::MAX_PACKET_PAYLOAD_LENGTH {
    //                 return Err(PayloadTooLongSnafu {
    //                     maximum: protocol::MAX_PACKET_PAYLOAD_LENGTH,
    //                     actual: packet.payload.len(),
    //                 }
    //                 .build()
    //                 .into());
    //             }
    //         }
    //         packet.padding = consumer.peek().to_vec();
    //         if packet.padding.len() != padding_len as usize {
    //             return Err(PaddingLengthIncorrectSnafu.build().into());
    //         }
    //         self.socket
    //             .consume_read_buffer(4 + length as usize + self.decrypt.tag_len());
    //         Ok(packet)
    //     } else {
    //         let block_size = self.decrypt.block_size();

    //         let mut size = block_size;

    //         while size < 4 {
    //             size += size;
    //         }

    //         if self.verify.encrypt_then_mac() {
    //             let header = self.socket.read_exact(size).await.context(builder::IO)?;
    //             let mut output = Vec::with_capacity(1024);
    //             self.decrypt.update(&header, &mut output)?;
    //             let packet_len = u32::from_be_bytes(output[..4].try_into().unwrap());

    //             if packet_len as usize + 4 > protocol::MAX_PACKET_LENGTH {
    //                 return Err(PacketTooLongSnafu {
    //                     maximum: protocol::MAX_PACKET_LENGTH,
    //                     actual: packet_len as usize + 4,
    //                 }
    //                 .build()
    //                 .into());
    //             }
    //             let len = packet_len as usize - (header.len() - 4);

    //             let left = self.socket.read_exact(len).await.context(builder::IO)?;

    //             let mac = self
    //                 .socket
    //                 .read_exact(self.verify.mac_len())
    //                 .await
    //                 .context(builder::IO)?;

    //             self.verify
    //                 .update(&self.server.sequence_number.to_be_bytes())?;
    //             self.verify.update(&header)?;
    //             self.verify.update(&left)?;
    //             if self.verify.finalize()? != mac {
    //                 return Err(MacVerificationFailedSnafu.build().into());
    //             }

    //             let mut consumer = Consumer::new(&output[4..]);
    //             let padding_len = consumer.consume_u8()?;

    //             let content =
    //                 consumer.consume_bytes(packet_len as usize - 1 - padding_len as usize)?;

    //             let mut packet = msg::Packet::default();

    //             packet.padding = consumer.peek().to_vec();
    //             if packet.padding.len() != padding_len as usize {
    //                 return Err(PaddingLengthIncorrectSnafu.build().into());
    //             }

    //             if self.authenticated || self.decode.compress_in_authentication() {
    //                 self.decode.update(content)?;
    //                 packet.payload = self.decode.finalize()?;
    //             } else {
    //                 packet.payload = content.to_vec();
    //             }
    //             Ok(packet)
    //         } else {
    //             let header = self.socket.read_exact(size).await.context(builder::IO)?;
    //             let mut output = Vec::with_capacity(1024);
    //             self.decrypt.update(&header, &mut output)?;

    //             let packet_len = u32::from_be_bytes(output[..4].try_into().unwrap());

    //             if packet_len as usize + 4 > protocol::MAX_PACKET_LENGTH {
    //                 return Err(PacketTooLongSnafu {
    //                     maximum: protocol::MAX_PACKET_LENGTH,
    //                     actual: packet_len as usize + 4,
    //                 }
    //                 .build()
    //                 .into());
    //             }

    //             let len = packet_len as usize - (header.len() - 4);

    //             let left = self.socket.read_exact(len).await.context(builder::IO)?;

    //             let mac = self
    //                 .socket
    //                 .read_exact(self.verify.mac_len())
    //                 .await
    //                 .context(builder::IO)?;

    //             self.decrypt.update(&left, &mut output)?;
    //             self.decrypt.finalize(&mut output)?;

    //             self.verify
    //                 .update(&self.server.sequence_number.to_be_bytes())?;
    //             self.verify.update(&output)?;
    //             if self.verify.finalize()? != mac {
    //                 return Err(MacVerificationFailedSnafu {}.build().into());
    //             }

    //             let mut consumer = Consumer::new(&output[4..]);
    //             let padding_len = consumer.consume_u8()?;

    //             let content =
    //                 consumer.consume_bytes(packet_len as usize - 1 - padding_len as usize)?;

    //             let mut packet = msg::Packet::default();

    //             packet.padding = consumer.peek().to_vec();
    //             if packet.padding.len() != padding_len as usize {
    //                 return Err(PaddingLengthIncorrectSnafu.build().into());
    //             }

    //             if self.authenticated || self.decode.compress_in_authentication() {
    //                 self.decode.update(content)?;
    //                 packet.payload = self.decode.finalize()?;
    //             } else {
    //                 packet.payload = content.to_vec();
    //             }
    //             Ok(packet)
    //         }
    //     }
    // }

    async fn recv_packet(&mut self) -> error::Result<msg::Packet> {
        self.read_packet().await
    }
}

impl<T> CipherStream<T>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    pub fn upgrade_client(
        &mut self,
        encrypt: Box<dyn Encrypt + Send>,
        encode: Box<dyn Encode + Send>,
        calculator: Box<dyn Mac + Send>,
    ) {
        self.encrypt = encrypt;
        self.encode = encode;
        self.calculator = calculator;
    }
    pub fn upgrade_server(
        &mut self,
        decrypt: Box<dyn Decrypt + Send>,
        decode: Box<dyn Decode + Send>,
        verify: Box<dyn Mac + Send>,
    ) {
        self.decrypt = decrypt;
        self.decode = decode;
        self.verify = verify;
    }
    pub fn server(&self) -> &NormalEndpoint {
        &self.server
    }

    pub fn client(&self) -> &NormalEndpoint {
        &self.client
    }

    async fn read_packet(&mut self) -> error::Result<msg::Packet> {
        loop {
            match &mut self.state {
                State::None => {
                    self.decrypt
                        .update_sequence_number(self.server.sequence_number)?;
                    self.state = State::Ready {
                        length: None,
                        encrypted: Vec::new(),
                        decrypted: Vec::new(),
                    };
                }
                State::Ready {
                    length,
                    encrypted,
                    decrypted,
                } => {
                    let packet = if self.decrypt.is_galois_counter_mode() {
                        let length = if let Some(length) = length {
                            *length
                        } else {
                            let mut bytes = self
                                .socket
                                .fill(std::mem::size_of::<u32>())
                                .await
                                .context(builder::IO)?
                                .to_vec();

                            self.decrypt.additional_authenticated_data(&mut bytes)?;
                            let l = {
                                let length = u32::from_be_bytes(bytes.try_into().unwrap());

                                if length as usize + 4 + self.decrypt.tag_len()
                                    > protocol::MAX_PACKET_LENGTH
                                {
                                    return Err(PacketTooLongSnafu {
                                        maximum: protocol::MAX_PACKET_LENGTH,
                                        actual: length as usize + 4,
                                    }
                                    .build()
                                    .into());
                                }
                                length
                            };
                            *length = Some(l);
                            l
                        };

                        let bytes = &self
                            .socket
                            .fill(4 + length as usize + self.decrypt.tag_len())
                            .await
                            .context(builder::IO)?[4..];

                        let mut output = Vec::with_capacity(1024);

                        self.decrypt
                            .update(&bytes[..length as usize], &mut output)?;
                        self.decrypt.authentication_tag(&bytes[length as usize..])?;
                        self.decrypt.finalize(&mut output)?;
                        let mut consumer = Consumer::new(&output);
                        let padding_len = consumer.consume_u8()?;
                        let content =
                            consumer.consume_bytes(output.len() - padding_len as usize - 1)?;

                        let mut packet = msg::Packet::default();
                        {
                            if self.authenticated || self.decode.compress_in_authentication() {
                                self.decode.update(content)?;
                                packet.payload = self.decode.finalize()?;
                            } else {
                                packet.payload = content.to_vec();
                            }
                            // if packet.payload.len() > protocol::MAX_PACKET_PAYLOAD_LENGTH {
                            //     return Err(PayloadTooLongSnafu {
                            //         maximum: protocol::MAX_PACKET_PAYLOAD_LENGTH,
                            //         actual: packet.payload.len(),
                            //     }
                            //     .build()
                            //     .into());
                            // }
                        }
                        packet.padding = consumer.peek().to_vec();
                        // if packet.padding.len() != padding_len as usize {
                        //     return Err(PaddingLengthIncorrectSnafu.build().into());
                        // }
                        self.socket
                            .consume_read_buffer(4 + length as usize + self.decrypt.tag_len());
                        packet
                    } else if self.verify.encrypt_then_mac() {
                        let length = if let Some(length) = length {
                            *length
                        } else {
                            let header = self.socket.fill(4).await.context(builder::IO)?;

                            let len = u32::from_be_bytes(header.try_into().unwrap());
                            if len as usize + 4 + self.verify.mac_len()
                                > protocol::MAX_PACKET_LENGTH
                            {
                                return Err(PacketTooLongSnafu {
                                    maximum: protocol::MAX_PACKET_LENGTH,
                                    actual: len as usize + 4,
                                }
                                .build()
                                .into());
                            }

                            len
                        };

                        assert!(decrypted.is_empty());

                        let packet = self
                            .socket
                            .fill(4 + length as usize + self.verify.mac_len())
                            .await
                            .context(builder::IO)?;

                        self.verify
                            .update(&self.server.sequence_number.to_be_bytes())?;
                        self.verify.update(&length.to_be_bytes())?;
                        self.verify.update(&packet[4..length as usize + 4])?;

                        let mac = self.verify.finalize()?;

                        if mac != packet[length as usize + 4..] {
                            return Err(MacVerificationFailedSnafu.build().into());
                        }

                        self.decrypt
                            .update(&packet[4..length as usize + 4], decrypted)?;
                        self.decrypt.finalize(decrypted)?;

                        let mut consumer = Consumer::new(decrypted);

                        let padding_len = consumer.consume_u8()?;
                        let content =
                            consumer.consume_bytes(length as usize - 1 - padding_len as usize)?;

                        let mut packet = msg::Packet {
                            padding: consumer.peek().to_vec(),
                            ..Default::default()
                        };

                        if self.authenticated || self.decode.compress_in_authentication() {
                            self.decode.update(content)?;
                            packet.payload = self.decode.finalize()?;
                        } else {
                            packet.payload = content.to_vec();
                        }

                        self.socket
                            .consume_read_buffer(4 + length as usize + self.verify.mac_len());

                        packet
                    } else {
                        let length = if let Some(length) = length {
                            *length
                        } else {
                            let block_size = self.decrypt.block_size();

                            assert_eq!(decrypted.len(), 0, "block size: {}", block_size);

                            debug_assert!(block_size >= 4);

                            let header = self.socket.fill(block_size).await.context(builder::IO)?;

                            self.decrypt.update(header, decrypted)?;

                            assert_eq!(header.len(), decrypted.len());

                            let len = u32::from_be_bytes(decrypted[..4].try_into().unwrap());

                            if len as usize + 4 + self.verify.mac_len()
                                > protocol::MAX_PACKET_LENGTH
                            {
                                return Err(PacketTooLongSnafu {
                                    maximum: protocol::MAX_PACKET_LENGTH,
                                    actual: len as usize + 4,
                                }
                                .build()
                                .into());
                            }

                            *encrypted = header.to_vec();

                            *length = Some(len);
                            len
                        };

                        let packet = self
                            .socket
                            .fill(length as usize + 4 + self.verify.mac_len())
                            .await
                            .context(builder::IO)?;

                        self.decrypt
                            .update(&packet[encrypted.len()..length as usize + 4], decrypted)?;
                        self.decrypt.finalize(decrypted)?;

                        self.verify
                            .update(&self.server.sequence_number.to_be_bytes())?;
                        self.verify.update(decrypted)?;

                        let mac = self.verify.finalize()?;

                        if mac != packet[length as usize + 4..] {
                            return Err(MacVerificationFailedSnafu.build().into());
                        }

                        // if self.verify.encrypt_then_mac() {
                        //     self.verify
                        //         .update(&self.server.sequence_number.to_be_bytes())?;
                        //     self.verify.update(&packet[..length as usize + 4])?;
                        //     if self.verify.finalize()? != &packet[length as usize + 4..] {
                        //         return Err(MacVerificationFailedSnafu.build().into());
                        //     }

                        //     self.decrypt
                        //         .update(&packet[encrypted.len()..length as usize + 4], decrypted)?;
                        //     self.decrypt.finalize(decrypted)?;
                        // } else {
                        //     self.decrypt
                        //         .update(&packet[encrypted.len()..length as usize + 4], decrypted)?;
                        //     self.decrypt.finalize(decrypted)?;
                        //     self.verify
                        //         .update(&self.server.sequence_number.to_be_bytes())?;
                        //     self.verify.update(&decrypted[..])?;
                        //     if self.verify.finalize()? != &packet[length as usize + 4..] {
                        //         return Err(MacVerificationFailedSnafu.build().into());
                        //     }
                        // }

                        let mut consumer = Consumer::new(&decrypted[4..length as usize + 4]);
                        let padding_len = consumer.consume_u8()?;

                        let content =
                            consumer.consume_bytes(length as usize - 1 - padding_len as usize)?;

                        // let mut packet = msg::Packet::default();

                        // packet.padding = consumer.peek().to_vec();
                        // if packet.padding.len() != padding_len as usize {
                        //     return Err(PaddingLengthIncorrectSnafu.build().into());
                        // }
                        let mut packet = msg::Packet {
                            padding: consumer.peek().to_vec(),
                            ..Default::default()
                        };

                        if self.authenticated || self.decode.compress_in_authentication() {
                            self.decode.update(content)?;
                            packet.payload = self.decode.finalize()?;
                        } else {
                            packet.payload = content.to_vec();
                        }
                        self.socket
                            .consume_read_buffer(length as usize + 4 + self.verify.mac_len());
                        packet
                    };

                    self.state = State::None;
                    self.server.sequence_number = self.server.sequence_number.wrapping_add(1);
                    if packet.payload.is_empty() {
                        return Err(PayloadIsEmptySnafu.build().into());
                    }

                    if packet.payload.len() > protocol::MAX_PACKET_PAYLOAD_LENGTH {
                        #[cfg(feature = "strict")]
                        return Err(PayloadTooLongSnafu {
                            maximum: protocol::MAX_PACKET_PAYLOAD_LENGTH,
                            actual: packet.payload.len(),
                        }
                        .build()
                        .into());

                        #[cfg(not(feature = "strict"))]
                        tracing::warn!(
                            "Maybe payload is too long, but we ignore it because strict feature is not enabled. payload length: {}",
                            packet.payload.len()
                        );
                    }
                    if packet.payload[0] == protocol::SSH_MSG_NEWKEYS
                        && self.client.kex_strict
                        && self.server.kex_strict
                    {
                        self.server.sequence_number = 0;
                    }

                    break Ok(packet);
                }
            }
        }
    }
}

enum State {
    None,
    Ready {
        length: Option<u32>,
        encrypted: Vec<u8>,
        decrypted: Vec<u8>,
    },
}
