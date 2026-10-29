//! Client processor for the MS-RDPEMSC mouse cursor channel.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::PduResult;
use tracing::{debug, trace, warn};

use crate::CHANNEL_NAME;
use crate::pdu::{CAPVERSION_1, CapabilitySet, CapsAdvertisePdu, MouseCursorPdu, PointerUpdate};

/// Largest number of pointer updates kept until [`MouseCursorClient::take_updates`] collects them.
pub const MAX_PENDING_UPDATES: usize = 256;

/// Client processor for `Microsoft::Windows::RDS::MouseCursor`.
///
/// The client advertises version 1 capabilities when the channel opens ([3.3.3]) and queues the
/// server's pointer updates for the caller, which applies them like [MS-RDPBCGR] pointer updates
/// and keeps the Pointer Image Cache ([3.3.1.1]).
///
/// [3.3.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
/// [3.3.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
#[derive(Debug, Default)]
pub struct MouseCursorClient {
    ready: bool,
    /// INVARIANT: `pending.len() <= MAX_PENDING_UPDATES`.
    pending: Vec<PointerUpdate>,
}

impl MouseCursorClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the server confirmed the capabilities, completing the Initializing Phase.
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Pointer updates received since the last call, oldest first.
    ///
    /// When more than [`MAX_PENDING_UPDATES`] updates arrive between two calls, the oldest are dropped.
    pub fn take_updates(&mut self) -> Vec<PointerUpdate> {
        core::mem::take(&mut self.pending)
    }

    fn reset(&mut self) {
        self.ready = false;
        self.pending.clear();
    }

    fn queue(&mut self, update: PointerUpdate) {
        if self.pending.len() == MAX_PENDING_UPDATES {
            warn!("Dropping the oldest pending pointer update");
            self.pending.remove(0);
        }
        self.pending.push(update);
    }
}

impl_as_any!(MouseCursorClient);

impl DvcProcessor for MouseCursorClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.reset();
        debug!(channel_id, "Mouse cursor channel started");

        let advertise = MouseCursorPdu::CapsAdvertise(CapsAdvertisePdu {
            caps_sets: vec![CapabilitySet::version1()],
        });
        Ok(vec![Box::new(advertise)])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        match decode::<MouseCursorPdu>(payload) {
            Ok(MouseCursorPdu::PointerUpdate(update)) => {
                trace!(?update, "Pointer update received");
                self.queue(update);
            }
            Ok(MouseCursorPdu::CapsConfirm(confirm)) => {
                if confirm.caps_set.version == CAPVERSION_1 {
                    debug!("Mouse cursor capabilities confirmed");
                    self.ready = true;
                } else {
                    warn!(
                        version = confirm.caps_set.version,
                        "Ignoring confirmation of capabilities that were not advertised"
                    );
                }
            }
            // Messages a client does not expect are ignored ([3.1.5.1]).
            Ok(MouseCursorPdu::CapsAdvertise(_)) => debug!("Ignoring a capabilities advertisement from the server"),
            Err(error) => warn!(%error, "Ignoring malformed mouse cursor message"),
        }
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.reset();
        debug!("Mouse cursor channel closed");
    }
}

impl DvcClientProcessor for MouseCursorClient {}
