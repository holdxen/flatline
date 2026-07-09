use tokio::sync::oneshot;

use crate::error;
use crate::session::forward;
use crate::ssh::msg::DisconnectReason;

pub trait Notifier {
    fn verify_server_host_key(
        &mut self,
        r#type: &str,
        host_key: &[u8],
    ) -> impl Future<Output = bool> + Send;
    fn server_host_keys(&mut self, host_keys: &[&[u8]]) -> impl Future<Output = bool> + Send;
    fn x11_forward(
        &mut self,
        originator: forward::SocketAddr,
        receiver: oneshot::Receiver<forward::Stream>,
        initial_window_size: &mut u32,
        maximum_packet_size: &mut u32,
    ) -> impl Future<Output = bool> + Send;
    fn agent_forward(
        &mut self,
        receiver: oneshot::Receiver<forward::Stream>,
        initial_window_size: &mut u32,
        maximum_packet_size: &mut u32,
    ) -> impl Future<Output = bool> + Send;
    fn disconnected(
        &mut self,
        reason: DisconnectReason,
        description: &str,
    ) -> impl Future<Output = ()> + Send;
    fn exited(&mut self, result: error::Result<()>) -> impl Future<Output = ()> + Send;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, PartialOrd)]
pub struct DefaultNotifier;

impl Notifier for DefaultNotifier {
    async fn verify_server_host_key(&mut self, r#type: &str, _: &[u8]) -> bool {
        tracing::info!("Verifying server host key: {}", r#type);
        true
    }

    async fn disconnected(&mut self, reason: DisconnectReason, description: &str) {
        tracing::info!("Disconnected with reason: {:?}, {}", reason, description);
    }

    async fn exited(&mut self, result: error::Result<()>) {
        tracing::info!("Session exit with result: {:?}", result);
    }

    async fn x11_forward(
        &mut self,
        originator: forward::SocketAddr,
        _: oneshot::Receiver<forward::Stream>,
        _: &mut u32,
        _: &mut u32,
    ) -> bool {
        tracing::info!("x11 forward: {:?}", originator);
        false
    }
    
    async fn server_host_keys(&mut self, _: &[&[u8]]) -> bool {
        tracing::info!("server host keys");
        true
    }
    
    async fn agent_forward(
        &mut self,
        _: oneshot::Receiver<forward::Stream>,
        _: &mut u32,
        _: &mut u32,
    ) -> bool {
        tracing::info!("Ignore agent forward");
        false
    }
}
