use ironrdp::cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use ironrdp::client::rdp::{RdpInputEvent, RdpInputSender};
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
        if self.tx.try_send(RdpInputEvent::Clipboard(message)).is_err() {
            self.tx.request_close();
            error!("Unable to enqueue OS clipboard message; cancelling RDP session");
        }
    }
}
