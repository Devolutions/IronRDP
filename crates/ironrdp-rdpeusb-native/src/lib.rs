#![cfg_attr(doc, doc = include_str!("../README.md"))]

use std::sync::{Arc, Weak};

use ironrdp_rdpeusb::client::UrbdrcListener;
use ironrdp_rdpeusb::io::{CompletionData, RequestId};
use nusb::MaybeFuture as _;

mod device;
mod filter;
mod manager;
mod urb;

#[cfg(windows)]
mod tsusb;

pub use device::OpenError;
pub use filter::{DEFAULT_LOCAL_CLASSES, UsbDeviceIdentity, UsbDeviceMatch, UsbFilterParseError, UsbRedirectionConfig};
pub use manager::Announcement;

/// Work the RDP session loop must perform on behalf of the USB backend.
///
/// The backend runs USB I/O on its own threads. It reports through these events, which the session
/// loop applies to the DRDYNVC client it owns.
///
/// An event can still be queued after what produced it is gone: the device channel was closed, or the
/// connection was replaced by a reconnection that may reuse the same channel IDs.
/// [`Self::into_current`] filters those out.
///
/// The producers are the device manager, owned by the URBDRC listener, and each device backend, owned
/// by its device channel processor: both live in the DRDYNVC client of the session loop that applies
/// the events. Background threads only hold weak references to them, so a producer is dropped on that
/// same loop, before or after an event is applied, never while one is being applied.
#[derive(Debug)]
pub struct UrbdrcEvent {
    /// Alive as long as the device backend or device manager that produced the event.
    origin: Weak<()>,
    kind: UrbdrcEventKind,
}

impl UrbdrcEvent {
    /// Returns the work to perform, or `None` when the device channel or the connection that produced
    /// the event no longer exists.
    pub fn into_current(self) -> Option<UrbdrcEventKind> {
        // `upgrade` decides atomically, unlike `strong_count`, which may be stale under concurrency.
        self.origin.upgrade().map(|_| self.kind)
    }
}

/// The work carried by an [`UrbdrcEvent`].
#[derive(Debug)]
pub enum UrbdrcEventKind {
    /// A pending request finished.
    ///
    /// Pass it to the [`UrbdrcDeviceClient`] of `channel_id` and send the resulting message. Requests
    /// the server cancelled complete this way too, with `USBD_STATUS_CANCELED` when they were stopped.
    /// A completion for a request that is no longer pending there must be dropped.
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
    ///
    /// Call [`Announcement::sent`] once the message is sent. Dropping the announcement instead
    /// releases the device, which the backend opened and claimed ahead of the announcement.
    AddVirtualChannel {
        control_channel_id: u32,
        announcement: Announcement,
    },
    /// A redirected device went away: close its device DVC.
    CloseChannel { channel_id: u32 },
}

/// Receiver of [`UrbdrcEvent`]s, called from backend threads.
///
/// It must not block; forwarding the event to the session loop through a channel is the expected
/// implementation.
pub type UrbdrcEventSink = Arc<dyn Fn(UrbdrcEvent) + Send + Sync>;

/// Sends events on behalf of one producer, stamped with its liveness token.
#[derive(Clone)]
pub(crate) struct EventOrigin {
    token: Weak<()>,
    sink: UrbdrcEventSink,
}

impl EventOrigin {
    /// Events sent through the returned origin are current while `token` is alive.
    pub(crate) fn new(token: &Arc<()>, sink: UrbdrcEventSink) -> Self {
        Self {
            token: Arc::downgrade(token),
            sink,
        }
    }

    pub(crate) fn sink(&self) -> &UrbdrcEventSink {
        &self.sink
    }

    pub(crate) fn send(&self, kind: UrbdrcEventKind) {
        (self.sink)(UrbdrcEvent {
            origin: Weak::clone(&self.token),
            kind,
        });
    }
}

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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn events_outlive_their_producer_as_stale() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink: UrbdrcEventSink = {
            let received = Arc::clone(&received);
            Arc::new(move |event| received.lock().expect("not poisoned").push(event))
        };

        let token = Arc::new(());
        let origin = EventOrigin::new(&token, sink);
        origin.send(UrbdrcEventKind::CloseChannel { channel_id: 7 });
        origin.send(UrbdrcEventKind::CloseChannel { channel_id: 8 });

        let mut events = received
            .lock()
            .expect("not poisoned")
            .drain(..)
            .collect::<Vec<_>>()
            .into_iter();
        let current = events.next().expect("first event").into_current();
        assert!(matches!(current, Some(UrbdrcEventKind::CloseChannel { channel_id: 7 })));

        // A reconnection drops the previous connection's producers before their queued events are applied.
        drop(token);
        assert!(events.next().expect("second event").into_current().is_none());
    }
}
