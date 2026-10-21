#![cfg_attr(doc, doc = include_str!("../README.md"))]

use std::sync::Arc;

use ironrdp_rdpeusb::client::UrbdrcListener;
use ironrdp_rdpeusb::io::{CompletionData, RequestId};
use nusb::MaybeFuture as _;

mod device;
mod filter;
mod manager;
mod urb;

pub use device::OpenError;
pub use filter::{DEFAULT_LOCAL_CLASSES, UsbDeviceIdentity, UsbDeviceMatch, UsbFilterParseError, UsbRedirectionConfig};

/// Work the RDP session loop must perform on behalf of the USB backend.
///
/// The backend runs USB I/O on its own threads. It reports through these events, which the session
/// loop applies to the DRDYNVC client it owns.
#[derive(Debug)]
pub enum UrbdrcEvent {
    /// A pending request finished.
    ///
    /// Pass it to the [`UrbdrcDeviceClient`] of `channel_id` and send the resulting message. A request
    /// that the server cancelled in the meantime is no longer pending there, and its completion must be
    /// dropped.
    ///
    /// [`UrbdrcDeviceClient`]: ironrdp_rdpeusb::client::UrbdrcDeviceClient
    Completion {
        channel_id: u32,
        request_id: RequestId,
        completion: CompletionData,
    },
    /// A device was plugged in: send `ADD_VIRTUAL_CHANNEL` on the control channel.
    ///
    /// See [`UrbdrcControlClient::add_virtual_channel`].
    ///
    /// [`UrbdrcControlClient::add_virtual_channel`]: ironrdp_rdpeusb::client::UrbdrcControlClient::add_virtual_channel
    AddVirtualChannel { control_channel_id: u32 },
    /// A redirected device went away: close its device DVC.
    CloseChannel { channel_id: u32 },
}

/// Receiver of [`UrbdrcEvent`]s, called from backend threads.
///
/// It must not block; forwarding the event to the session loop through a channel is the expected
/// implementation.
pub type UrbdrcEventSink = Arc<dyn Fn(UrbdrcEvent) + Send + Sync>;

/// Build the URBDRC DVC listener that redirects the local USB devices selected by `config`.
pub fn urbdrc_listener(config: UsbRedirectionConfig, sink: UrbdrcEventSink) -> UrbdrcListener {
    let manager = manager::NusbDeviceManager::new(config, sink);
    let on_capability_exchanged = manager.on_capability_exchanged();

    UrbdrcListener::new(Box::new(on_capability_exchanged), Box::new(manager))
}

/// A USB device attached to this machine.
#[derive(Debug, Clone)]
pub struct LocalUsbDevice {
    pub identity: UsbDeviceIdentity,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    /// Function driver bound to the device, on Windows.
    ///
    /// Only `WinUSB` devices, or composite (`usbccgp`) devices whose interfaces all use WinUSB, can be
    /// redirected on Windows.
    pub driver: Option<String>,
}

/// List the USB devices attached to this machine.
///
/// This helps users write `id:` and `addr:` rules, and shows on Windows which driver owns each device.
pub fn local_devices() -> std::io::Result<Vec<LocalUsbDevice>> {
    let devices = nusb::list_devices().wait()?;

    Ok(devices
        .map(|info| LocalUsbDevice {
            identity: device::identity(&info),
            manufacturer: info.manufacturer_string().map(str::to_owned),
            product: info.product_string().map(str::to_owned),
            #[cfg(windows)]
            driver: info.driver().map(str::to_owned),
            #[cfg(not(windows))]
            driver: None,
        })
        .collect())
}
