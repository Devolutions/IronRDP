//! Device discovery and the URBDRC device manager.

use core::fmt;
use core::time::Duration;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;

use futures_core::Stream as _;
use ironrdp_dvc::DvcMessage;
use ironrdp_pdu::PduResult;
use ironrdp_rdpeusb::client::{DeviceManagerBackend, UrbdrcDeviceBackend};
use nusb::MaybeFuture as _;
use nusb::hotplug::HotplugEvent;
use tokio::sync::oneshot;
use tokio::task::AbortHandle;
use tracing::{debug, info, warn};

use crate::device::{self, OpenedDevice, Reply};
use crate::filter::{UsbDeviceIdentity, UsbRedirectionConfig};
use crate::{EventOrigin, UrbdrcEventKind, UrbdrcEventSink};

/// Attempts to open a hotplugged device, whose driver may still be loading when it is announced.
const HOTPLUG_OPEN_ATTEMPTS: u32 = 5;
const HOTPLUG_OPEN_DELAY: Duration = Duration::from_millis(400);

/// Identifies a local device across discovery, announcement and redirection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum DeviceKey {
    Nusb(nusb::DeviceId),
    /// A device claimed through the RemoteFX USB redirection drivers, by its proprietary ID.
    #[cfg(windows)]
    TsUsb(String),
}

/// A local device opened for redirection, waiting for its device channel.
pub(crate) trait RedirectableDevice: Send {
    fn key(&self) -> DeviceKey;

    fn vendor_product(&self) -> (u16, u16);

    /// Serve the device on the device channel `reply` completes requests on.
    ///
    /// `events_token` keeps the channel's events current. `on_close` runs once the device is
    /// released, so that it can be offered again.
    fn start(
        self: Box<Self>,
        reply: Reply,
        events_token: Arc<()>,
        on_close: Box<dyn FnOnce() + Send>,
    ) -> Box<dyn UrbdrcDeviceBackend>;
}

/// URBDRC device manager.
///
/// On Windows, devices are claimed through the RemoteFX USB redirection drivers when the policy
/// allows it; otherwise, and on other platforms, they are opened with nusb. Devices are opened before
/// they are announced to the server, so that a device the client cannot claim is never offered. The
/// manager hands them out, in announcement order, as the server creates one device DVC per
/// `ADD_VIRTUAL_CHANNEL`.
pub(crate) struct NusbDeviceManager {
    shared: Arc<Shared>,
    /// Keeps the manager's events current; they are dropped once the connection is torn down.
    _events_token: Arc<()>,
}

pub(crate) struct Shared {
    pub(crate) config: UsbRedirectionConfig,
    events: EventOrigin,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    control_channel_id: Option<u32>,
    /// Opened devices announced with `ADD_VIRTUAL_CHANNEL`, waiting for their DVC.
    pending: PendingQueue<DeviceKey, Box<dyn RedirectableDevice>>,
    /// Devices announced or redirected, with their device DVC once it exists.
    redirected: HashMap<DeviceKey, Option<Reply>>,
    /// Stops the discovery thread.
    discovery: Option<oneshot::Sender<()>>,
}

/// Devices announced to the server, in order, waiting for their device channel.
///
/// The server creates one device channel per `ADD_VIRTUAL_CHANNEL`, in order. A device unplugged
/// before its channel exists keeps its place, so that the channel created for it is refused instead of
/// receiving the next device.
struct PendingQueue<K, T> {
    next_announcement: u64,
    slots: VecDeque<PendingSlot<K, T>>,
}

struct PendingSlot<K, T> {
    announcement: u64,
    key: K,
    /// `None` once the device is unplugged.
    device: Option<T>,
}

impl<K, T> Default for PendingQueue<K, T> {
    fn default() -> Self {
        Self {
            next_announcement: 0,
            slots: VecDeque::new(),
        }
    }
}

impl<K: Clone + PartialEq, T> PendingQueue<K, T> {
    /// Queue a device, returning its announcement.
    fn push(&mut self, key: K, device: T) -> u64 {
        let announcement = self.next_announcement;
        self.next_announcement = self.next_announcement.wrapping_add(1);
        self.slots.push_back(PendingSlot {
            announcement,
            key,
            device: Some(device),
        });
        announcement
    }

    /// Take the device for the next device channel: `Some(None)` when it was unplugged meanwhile.
    fn pop(&mut self) -> Option<Option<T>> {
        self.slots.pop_front().map(|slot| slot.device)
    }

    /// Release an unplugged device, keeping its place.
    fn unplug(&mut self, key: &K) {
        for slot in self.slots.iter_mut().filter(|slot| slot.key == *key) {
            slot.device = None;
        }
    }

    fn is_unplugged(&self, announcement: u64) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.announcement == announcement && slot.device.is_none())
    }

    /// Remove an announcement that was never sent, returning its device unless it was unplugged.
    fn withdraw(&mut self, announcement: u64) -> Option<(K, T)> {
        let position = self.slots.iter().position(|slot| slot.announcement == announcement)?;
        let slot = self.slots.remove(position)?;
        slot.device.map(|device| (slot.key, device))
    }
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        // The state stays consistent across a panic: every update is a single collection operation.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl NusbDeviceManager {
    pub(crate) fn new(config: UsbRedirectionConfig, sink: UrbdrcEventSink) -> Self {
        let events_token = Arc::new(());
        Self {
            shared: Arc::new(Shared {
                config,
                events: EventOrigin::new(&events_token, sink),
                state: Mutex::new(State::default()),
            }),
            _events_token: events_token,
        }
    }

    /// Build the callback run once the control channel completed its capability exchange.
    ///
    /// The callback runs on the session loop, so it only starts the discovery thread: opening devices
    /// blocks while the OS opens them. Devices are announced through
    /// [`UrbdrcEventKind::AddVirtualChannel`] events as they are opened.
    pub(crate) fn on_capability_exchanged(&self) -> impl FnOnce() -> PduResult<Vec<DvcMessage>> + Send + 'static {
        let shared = Arc::clone(&self.shared);
        move || {
            let discovery = start_discovery(&shared);
            shared.state().discovery = discovery;
            Ok(Vec::new())
        }
    }
}

impl Drop for NusbDeviceManager {
    fn drop(&mut self) {
        if let Some(stop) = self.shared.state().discovery.take() {
            let _ = stop.send(());
        }
    }
}

impl DeviceManagerBackend for NusbDeviceManager {
    fn control_channel_assigned(&mut self, channel_id: u32) {
        debug!(channel_id, "URBDRC control channel assigned");
        self.shared.state().control_channel_id = Some(channel_id);
    }

    fn take_device_for_channel(&mut self, channel_id: u32) -> Option<Box<dyn UrbdrcDeviceBackend>> {
        let (device, reply, events_token) = {
            let mut state = self.shared.state();
            let Some(device) = state.pending.pop()? else {
                debug!(
                    channel_id,
                    "Refusing the device channel of a USB device unplugged before it opened"
                );
                return None;
            };
            let events_token = Arc::new(());
            let reply = Reply::new(channel_id, &events_token, Arc::clone(self.shared.events.sink()));
            state.redirected.insert(device.key(), Some(reply.clone()));
            (device, reply, events_token)
        };

        let (vendor_id, product_id) = device.vendor_product();
        info!(
            channel_id,
            vendor_id = format_args!("{vendor_id:04x}"),
            product_id = format_args!("{product_id:04x}"),
            "Redirecting USB device"
        );

        let key = device.key();
        let shared = Arc::downgrade(&self.shared);
        let closed = reply.clone();
        let on_close = Box::new(move || {
            if let Some(shared) = shared.upgrade() {
                let mut state = shared.state();
                if state
                    .redirected
                    .get(&key)
                    .and_then(Option::as_ref)
                    .is_some_and(|current| current.same_channel(&closed))
                {
                    state.redirected.remove(&key);
                }
            }
        });

        Some(device.start(reply, events_token, on_close))
    }
}

fn open(info: &nusb::DeviceInfo, identity: &UsbDeviceIdentity) -> Option<OpenedDevice> {
    match OpenedDevice::open(info, identity) {
        Ok(device) => Some(device),
        Err(error) => {
            warn!(
                %error,
                vendor_id = format_args!("{:04x}", info.vendor_id()),
                product_id = format_args!("{:04x}", info.product_id()),
                "Skipping USB device that cannot be opened for redirection"
            );
            None
        }
    }
}

/// Start the thread that announces the present devices and then follows hotplug, returning the
/// handle that stops it.
fn start_discovery(shared: &Arc<Shared>) -> Option<oneshot::Sender<()>> {
    #[cfg(windows)]
    if let Some(stop) = crate::tsusb::start_discovery(shared) {
        return Some(stop);
    }
    start_nusb_discovery(shared)
}

fn start_nusb_discovery(shared: &Arc<Shared>) -> Option<oneshot::Sender<()>> {
    // Subscribe before enumerating, so that a device plugged in meanwhile is not missed. Watch even
    // when no rule follows new devices: unplugging a redirected device must close its channel, and
    // `redirect_on_hotplug` already ignores rules that apply at connection only.
    let watch = nusb::watch_devices()
        .inspect_err(|error| {
            warn!(%error, "USB hotplug is unavailable; plugged and unplugged devices are not followed");
        })
        .ok();
    let (stop, stopped) = oneshot::channel();
    let shared = Arc::downgrade(shared);

    let spawned = thread::Builder::new()
        .name("urbdrc-discovery".to_owned())
        .spawn(
            move || match tokio::runtime::Builder::new_current_thread().enable_time().build() {
                Ok(runtime) => runtime.block_on(run_discovery(watch, stopped, shared)),
                Err(error) => warn!(%error, "Failed to start the USB discovery runtime"),
            },
        );
    if let Err(error) = spawned {
        warn!(%error, "Failed to start the USB discovery thread");
        return None;
    }

    Some(stop)
}

async fn run_discovery(
    watch: Option<nusb::hotplug::HotplugWatch>,
    stopped: oneshot::Receiver<()>,
    shared: Weak<Shared>,
) {
    if let Some(shared) = shared.upgrade() {
        announce_present_devices(&shared);
    }
    if let Some(watch) = watch {
        run_hotplug(watch, stopped, shared).await;
    }
}

/// Open the devices selected at connection time and announce them.
fn announce_present_devices(shared: &Arc<Shared>) {
    let devices = match nusb::list_devices().wait() {
        Ok(devices) => devices,
        Err(error) => {
            warn!(%error, "Failed to enumerate USB devices");
            return;
        }
    };

    let mut count = 0usize;
    for info in devices {
        let identity = device::identity(&info);
        if shared.config.redirect_at_connect(&identity)
            && let Some(device) = open(&info, &identity)
            && announce(shared, Box::new(device))
        {
            count = count.saturating_add(1);
        }
    }
    debug!(count, "Announced USB devices present at connection");
}

/// Whether a device is announced or redirected.
pub(crate) fn is_redirected(shared: &Shared, key: &DeviceKey) -> bool {
    shared.state().redirected.contains_key(key)
}

/// Queue an opened device for the next device channel and ask the session to request one.
///
/// Returns `false` when the device is already announced or redirected.
pub(crate) fn announce(shared: &Arc<Shared>, device: Box<dyn RedirectableDevice>) -> bool {
    let key = device.key();
    let (control_channel_id, announcement) = {
        let mut state = shared.state();
        if state.redirected.contains_key(&key) {
            return false;
        }
        let Some(control_channel_id) = state.control_channel_id else {
            return false;
        };
        state.redirected.insert(key.clone(), None);
        let announcement = state.pending.push(key, device);
        (control_channel_id, announcement)
    };
    let announcement = Announcement {
        shared: Arc::downgrade(shared),
        announcement: Some(announcement),
    };
    shared.events.send(UrbdrcEventKind::AddVirtualChannel {
        control_channel_id,
        announcement,
    });
    true
}

/// A device queued for the next device channel, until its `ADD_VIRTUAL_CHANNEL` is sent.
///
/// The device is opened, with its interfaces claimed, before it is announced. If the announcement
/// is dropped without being sent, for example because the control channel closed, the device is
/// released instead of staying claimed and unusable locally until it is unplugged.
pub struct Announcement {
    shared: Weak<Shared>,
    /// Cleared once the announcement is sent.
    announcement: Option<u64>,
}

impl Announcement {
    /// Whether the device was unplugged before being announced: the announcement is not worth sending.
    pub fn is_unplugged(&self) -> bool {
        match (self.announcement, self.shared.upgrade()) {
            (Some(announcement), Some(shared)) => shared.state().pending.is_unplugged(announcement),
            _ => true,
        }
    }

    /// Record that `ADD_VIRTUAL_CHANNEL` was sent, so the device waits for its channel.
    pub fn sent(mut self) {
        self.announcement = None;
    }
}

impl fmt::Debug for Announcement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Announcement")
            .field("announcement", &self.announcement)
            .finish_non_exhaustive()
    }
}

impl Drop for Announcement {
    fn drop(&mut self) {
        let (Some(announcement), Some(shared)) = (self.announcement.take(), self.shared.upgrade()) else {
            return;
        };
        let released = {
            let mut state = shared.state();
            let released = state.pending.withdraw(announcement);
            if let Some((key, _)) = &released {
                state.redirected.remove(key);
            }
            released
        };
        if released.is_some() {
            debug!("Released a USB device whose announcement was not sent");
        }
    }
}

async fn run_hotplug(mut watch: nusb::hotplug::HotplugWatch, mut stopped: oneshot::Receiver<()>, shared: Weak<Shared>) {
    // Devices being opened, whose attempts stop once they are unplugged.
    let mut opening: HashMap<nusb::DeviceId, AbortHandle> = HashMap::new();
    loop {
        let event = tokio::select! {
            _ = &mut stopped => break,
            event = core::future::poll_fn(|cx| core::pin::Pin::new(&mut watch).poll_next(cx)) => event,
        };
        let (Some(event), Some(shared)) = (event, shared.upgrade()) else {
            break;
        };

        match event {
            HotplugEvent::Connected(info) => {
                let identity = device::identity(&info);
                if shared.config.redirect_on_hotplug(&identity) {
                    // Retried apart, so that unplugged devices are still followed meanwhile.
                    opening.retain(|_, task| !task.is_finished());
                    let id = info.id();
                    let task = tokio::spawn(open_hotplugged(info, identity, Arc::downgrade(&shared)));
                    if let Some(previous) = opening.insert(id, task.abort_handle()) {
                        previous.abort();
                    }
                }
            }
            HotplugEvent::Disconnected(id) => {
                if let Some(task) = opening.remove(&id) {
                    task.abort();
                }
                unplugged(&shared, &DeviceKey::Nusb(id));
            }
        }
    }
}

/// Stop redirecting a device that is gone, closing its device channel.
pub(crate) fn unplugged(shared: &Shared, key: &DeviceKey) {
    let channel = {
        let mut state = shared.state();
        state.pending.unplug(key);
        state.redirected.remove(key).flatten()
    };
    if let Some(channel) = channel {
        debug!(channel_id = channel.channel_id(), "Redirected USB device was unplugged");
        channel.terminate();
    }
}

/// Open and announce a plugged device, retrying while its driver may still be loading.
async fn open_hotplugged(info: nusb::DeviceInfo, identity: UsbDeviceIdentity, shared: Weak<Shared>) {
    for attempt in 1..=HOTPLUG_OPEN_ATTEMPTS {
        if attempt != 1 {
            tokio::time::sleep(HOTPLUG_OPEN_DELAY).await;
        }
        let Some(shared) = shared.upgrade() else {
            return;
        };
        // Already claimed, for example by the enumeration at connection time.
        if is_redirected(&shared, &DeviceKey::Nusb(info.id())) {
            return;
        }
        let device = if attempt == HOTPLUG_OPEN_ATTEMPTS {
            open(&info, &identity)
        } else {
            OpenedDevice::open(&info, &identity).ok()
        };
        if let Some(device) = device {
            announce(&shared, Box::new(device));
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unplugged_devices_keep_their_place() {
        let mut queue = PendingQueue::default();
        let first = queue.push(1, "first");
        let second = queue.push(2, "second");

        queue.unplug(&1);
        assert!(queue.is_unplugged(first));
        assert!(!queue.is_unplugged(second));

        // The channel created for the unplugged device is refused, and the next one still gets its device.
        assert_eq!(queue.pop(), Some(None));
        assert_eq!(queue.pop(), Some(Some("second")));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn withdrawn_announcements_leave_the_queue() {
        let mut queue = PendingQueue::default();
        let first = queue.push(1, "first");
        let second = queue.push(1, "replugged");
        queue.unplug(&1);
        let third = queue.push(1, "plugged again");

        // An unplugged device was already released; the others are returned to be released.
        assert_eq!(queue.withdraw(first), None);
        assert_eq!(queue.withdraw(third), Some((1, "plugged again")));
        assert_eq!(queue.withdraw(third), None);
        assert_eq!(queue.pop(), Some(None));
        assert!(
            !queue.is_unplugged(second),
            "a popped announcement is no longer pending"
        );
    }
}
