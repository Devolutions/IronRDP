use ironrdp::cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use tokio::sync::mpsc;
use tracing::error;

use crate::rdp::RdpInputEvent;

/// Shim for sending and receiving CLIPRDR events as `RdpInputEvent`
#[derive(Clone, Debug)]
pub struct ClientClipboardMessageProxy {
    tx: mpsc::UnboundedSender<RdpInputEvent>,
}

impl ClientClipboardMessageProxy {
    pub fn new(tx: mpsc::UnboundedSender<RdpInputEvent>) -> Self {
        Self { tx }
    }
}

impl ClipboardMessageProxy for ClientClipboardMessageProxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        if self.tx.send(RdpInputEvent::Clipboard(message)).is_err() {
            error!("Failed to send os clipboard message, receiver is closed");
        }
    }
}
