use ironrdp::cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use ironrdp::client::rdp::RdpInputSender;
use tracing::error;

/// Shim for sending and receiving CLIPRDR events as `RdpInputEvent`
#[derive(Clone, Debug)]
pub struct ClientClipboardMessageProxy {
    tx: RdpInputSender,
}

impl ClientClipboardMessageProxy {
    pub fn new(tx: RdpInputSender) -> Self {
        Self { tx }
    }
}

impl ClipboardMessageProxy for ClientClipboardMessageProxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        match self.tx.send_clipboard(message) {
            Ok(()) => {}
            Err(_) => {
                error!("Unable to enqueue OS clipboard message because the RDP session is closed");
            }
        }
    }
}
