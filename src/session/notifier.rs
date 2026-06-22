use crate::error;
use crate::ssh::msg::DisconnectReason;

pub trait Notifier {

    fn verify_server_host_key(&mut self, r#type: &str, host_key: &[u8]) -> impl Future<Output = bool> + Send;

    fn disconnected(&mut self, reason: DisconnectReason, description: &str) -> impl Future<Output = ()> + Send;
    fn exited(&mut self, result: error::Result<()>) -> impl Future<Output = ()> + Send;
}

#[derive(Debug, Default)]
pub struct DefaultNotifier;

impl Notifier for DefaultNotifier {
    async fn verify_server_host_key(&mut self, r#type: &str, _: &[u8]) -> bool {
        tracing::info!("Verifying server host key: {}", r#type);
        true
    }

    async fn disconnected(&mut self, reason: DisconnectReason, description: &str) {
        tracing::info!("Disconnected with reason: {:?}, {}", reason, description);
    }

    async fn exited(&mut self, result: error::Result<()>)  {
        tracing::info!("Session exit with result: {:?}", result);
    }
}