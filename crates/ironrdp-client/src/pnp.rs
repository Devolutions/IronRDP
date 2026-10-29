//! Plug and Play device redirection ([MS-RDPEPNP]) configuration and session wiring.
//!
//! [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/

use core::fmt;
use std::sync::Arc;

use ironrdp_dvc::DrdynvcClient;
use ironrdp_pdu::pdu_other_err;
use ironrdp_rdpepnp::client::{IoUplink, PnpBackend, PnpInfoClient};
use ironrdp_rdpepnp::pdu::info::DeviceDescription;
use tracing::warn;

use crate::rdp::{RdpInputEvent, RdpInputSender};

/// Creates the backend of one connection.
pub type PnpBackendFactory = Arc<dyn Fn() -> Box<dyn PnpBackend> + Send + Sync>;

/// Plug and Play devices to redirect and the backend that performs their I/O.
///
/// IronRDP has no native PnP device backend: the embedder describes the devices and implements
/// [`PnpBackend`]. The factory runs once per connection.
#[derive(Clone)]
pub struct PnpConfig {
    pub devices: Vec<DeviceDescription>,
    pub backend: PnpBackendFactory,
}

impl fmt::Debug for PnpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PnpConfig")
            .field("devices", &self.devices)
            .finish_non_exhaustive()
    }
}

/// Registers the `PNPDR` channel and the `FileRedirectorChannel` listener.
pub(crate) fn attach(drdynvc: DrdynvcClient, config: &PnpConfig, sender: &RdpInputSender) -> DrdynvcClient {
    let info = match PnpInfoClient::new(config.devices.clone()) {
        Ok(info) => info,
        Err(error) => {
            warn!(%error, "Invalid PnP device list; PnP device redirection is disabled");
            return drdynvc;
        }
    };
    let sender = sender.clone();
    // Replies completed outside the session loop share the bounded input queue, so a full queue
    // drops the reply and leaves the server's request pending; the failure is logged.
    let uplink: IoUplink = Arc::new(move |channel_id, messages| {
        sender
            .try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
            .map_err(|_| pdu_other_err!("send PnP device replies to the event loop"))
    });
    let listener = info.file_redirector_listener((config.backend)(), uplink);
    drdynvc.with_dynamic_channel(info).with_listener(listener)
}
