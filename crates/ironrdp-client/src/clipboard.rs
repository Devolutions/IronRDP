use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use tracing::error;

use crate::rdp::{RdpInputEvent, RdpInputSender};

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
        if self.tx.try_send(RdpInputEvent::Clipboard(message)).is_err() {
            // Continuing after losing a clipboard protocol message can desynchronize the channel.
            self.tx.request_close();
            error!("Unable to enqueue clipboard message; cancelling RDP session");
        }
    }
}
