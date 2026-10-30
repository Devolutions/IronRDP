//! Server processor for the MS-RDPEMSC mouse cursor channel.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::PduResult;
use tracing::{debug, warn};

use crate::CHANNEL_NAME;
use crate::pdu::{CAPVERSION_1, CapabilitySet, CapsConfirmPdu, MouseCursorPdu};

/// Server processor for `Microsoft::Windows::RDS::MouseCursor`.
///
/// The server answers a client advertising version 1 capabilities with a confirmation ([3.2.3]).
/// Once [`MouseCursorServer::is_ready`] holds, the server sends [`MouseCursorPdu::PointerUpdate`]
/// messages on the channel instead of [MS-RDPBCGR] pointer updates.
///
/// [3.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
#[derive(Debug, Default)]
pub struct MouseCursorServer {
    ready: bool,
}

impl MouseCursorServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the capabilities were confirmed, so that pointer updates can be sent.
    pub fn is_ready(&self) -> bool {
        self.ready
    }
}

impl_as_any!(MouseCursorServer);

impl DvcProcessor for MouseCursorServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        // The client opens the Initializing Phase ([3.3.3]).
        self.ready = false;
        debug!(channel_id, "Mouse cursor channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        match decode::<MouseCursorPdu>(payload) {
            Ok(MouseCursorPdu::CapsAdvertise(advertise)) => {
                if advertise
                    .caps_sets
                    .iter()
                    .any(|caps_set| caps_set.version == CAPVERSION_1)
                {
                    debug!("Confirming mouse cursor capabilities");
                    self.ready = true;
                    let confirm = MouseCursorPdu::CapsConfirm(CapsConfirmPdu {
                        caps_set: CapabilitySet::version1(),
                    });
                    return Ok(vec![Box::new(confirm)]);
                }
                warn!("Client advertised no supported mouse cursor capability set");
            }
            // Messages a server does not expect are ignored ([3.1.5.1]).
            Ok(_) => debug!("Ignoring a server-to-client mouse cursor message from the client"),
            Err(error) => warn!(%error, "Ignoring malformed mouse cursor message"),
        }
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.ready = false;
        debug!("Mouse cursor channel closed");
    }
}

impl DvcServerProcessor for MouseCursorServer {}
