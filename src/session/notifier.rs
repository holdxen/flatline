//! Session lifecycle callbacks.
//!
//! A [`Notifier`] receives peer-driven events that the application may want
//! to react to: host-key verification during the handshake, X11/agent
//! forwarding requests, disconnection, and the final session result. Pass
//! one to [`Session::handshake`](super::Session::handshake) to install it;
//! [`DefaultNotifier`] logs every event and accepts the default policy.

use std::path::PathBuf;

use tokio::sync::oneshot;

use crate::error;
use crate::session::forward;
use crate::ssh::msg::DisconnectReason;

/// Callbacks invoked by the session backend when the peer drives an event.
///
/// Every method is `async` and `Send`, because notifiers are polled on the
/// backend task. Returning `true` accepts the request, `false` declines it.
pub trait Notifier {
    /// Called during the key exchange with the server's host key.
    ///
    /// `type` is the host-key algorithm name (e.g. `"ssh-ed25519"`) and
    /// `host_key` is the key blob in wire format. Return `true` to accept the
    /// key and continue the handshake, `false` to abort it.
    fn verify_server_host_key(
        &mut self,
        r#type: &str,
        host_key: &[u8],
    ) -> impl Future<Output = bool> + Send;

    /// Called once the server has sent all of its host keys (the
    /// `hostkeys-00@openssh.com` extension).
    ///
    /// Each entry in `host_keys` is a key blob in wire format. Return `true`
    /// to accept the set.
    fn server_host_keys(&mut self, host_keys: &[&[u8]]) -> impl Future<Output = bool> + Send;

    /// Called when the server requests an X11 forwarding channel for a
    /// connection from `originator`.
    ///
    /// The `receiver` yields the forwarded stream once it is accepted;
    /// `initial_window_size` and `maximum_packet_size` may be overwritten to
    /// tune flow control. Return `true` to accept the forwarding request.
    fn x11_forward(
        &mut self,
        originator: forward::SocketAddr,
        receiver: oneshot::Receiver<forward::Stream>,
        initial_window_size: &mut u32,
        maximum_packet_size: &mut u32,
    ) -> impl Future<Output = bool> + Send;

    /// Called when the server requests an SSH agent forwarding channel.
    ///
    /// Behaves like [`x11_forward`](Self::x11_forward): fill in the window
    /// parameters and return `true` to accept.
    fn agent_forward(
        &mut self,
        receiver: oneshot::Receiver<forward::Stream>,
        initial_window_size: &mut u32,
        maximum_packet_size: &mut u32,
    ) -> impl Future<Output = bool> + Send;

    /// Called when the peer sends a `SSH_MSG_DISCONNECT`.
    ///
    /// After this returns the session is closed and further calls on it will
    /// fail.
    fn disconnected(
        &mut self,
        reason: DisconnectReason,
        description: &str,
    ) -> impl Future<Output = ()> + Send;

    /// Called once the session's backend task has terminated, with the error
    /// that ended it (or `Ok(())` for a clean shutdown).
    fn exited(&mut self, result: error::Result<()>) -> impl Future<Output = ()> + Send;
}

/// A [`Notifier`] that logs each event with `tracing` and applies a fixed
/// policy: host keys are accepted, X11 and agent forwarding are declined.
///
/// This is the notifier used when none is installed.
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
