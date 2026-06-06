#[macro_use]
mod ssh;
pub mod channel;
pub mod cipher;
pub mod error;
pub mod forward;
pub mod handshake;
pub mod keys;
mod msg;
mod project;
pub mod scp;
pub mod session;
pub mod sftp;

#[cfg(test)]
mod test;

type MSender<T> = mpsc::UnboundedSender<T>;
type MReceiver<T> = mpsc::UnboundedReceiver<T>;
type OSender<T> = oneshot::Sender<T>;
type OReceiver<T> = oneshot::Receiver<T>;
type MWSender<T> = mpsc::WeakUnboundedSender<T>;
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + Sync + 'a>>;

trait BigNumExt {
    fn to_ssh_bytes(&self) -> Vec<u8>;
}

impl BigNumExt for openssl::bn::BigNum {
    fn to_ssh_bytes(&self) -> Vec<u8> {
        // https://github.com/openssh/openssh-portable/blob/master/sshbuf-getput-basic.c#L585
        let mut bytes = vec![0; 1];
        let bn = self.to_vec();
        if !bn.is_empty() && bn[0] & 0x80 != 0 {
            bytes.extend(bn);
        } else {
            bytes = bn;
        }
        bytes
    }
}

impl BigNumExt for openssl::bn::BigNumRef {
    fn to_ssh_bytes(&self) -> Vec<u8> {
        // https://github.com/openssh/openssh-portable/blob/master/sshbuf-getput-basic.c#L585
        let mut bytes = vec![0; 1];
        let bn = self.to_vec();
        if !bn.is_empty() && bn[0] & 0x80 != 0 {
            bytes.extend(bn);
        } else {
            bytes = bn;
        }
        bytes
    }
}

fn m_channel<T>() -> (MSender<T>, MReceiver<T>) {
    mpsc::unbounded_channel()
}

fn o_channel<T>() -> (OSender<T>, OReceiver<T>) {
    oneshot::channel()
}

use std::{future::Future, pin::Pin};

pub use async_trait::async_trait;
pub use cipher::{
    compress::{Decode, Encode},
    crypt::{Decrypt, Encrypt},
    hash::Hash,
    kex::{KeyExChange, KexAlgorithm, StandardKexProtocol},
    mac::Mac,
    sign::{Signature, Verify},
    AlgoFactory,
};
pub use openssl;
use tokio::sync::{mpsc, oneshot};
