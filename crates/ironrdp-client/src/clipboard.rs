use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use tracing::error;

use crate::rdp::RdpInputSender;

/// Shim that forwards CLIPRDR events into the `RdpInputEvent` channel.
#[derive(Clone, Debug)]
pub(crate) struct ClientClipboardMessageProxy {
    tx: RdpInputSender,
}

impl ClientClipboardMessageProxy {
    pub(crate) fn new(tx: RdpInputSender) -> Self {
        Self { tx }
    }
}

impl ClipboardMessageProxy for ClientClipboardMessageProxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        if self.tx.send_clipboard(message).is_err() {
            error!("Unable to enqueue clipboard message because the RDP session is closed");
        }
    }
}
