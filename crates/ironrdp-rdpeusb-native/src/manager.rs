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
use tracing::{debug, info, warn};

use crate::device::{self, NusbDeviceBackend, OpenedDevice};
use crate::filter::UsbRedirectionConfig;
use crate::{EventOrigin, UrbdrcEventKind, UrbdrcEventSink};

/// Attempts to open a hotplugged device, whose driver may still be loading when it is announced.
const HOTPLUG_OPEN_ATTEMPTS: u32 = 5;
const HOTPLUG_OPEN_DELAY: Duration = Duration::from_millis(400);

/// URBDRC device manager backed by nusb.
///
/// Devices are opened before they are announced to the server, so that a device the client cannot
/// claim is never offered. The manager hands them out, in announcement order, as the server creates
/// one device DVC per `ADD_VIRTUAL_CHANNEL`.
pub(crate) struct NusbDeviceManager {
    shared: Arc<Shared>,
    /// Keeps the manager's events current; they are dropped once the connection is torn down.
    _events_token: Arc<()>,
}

struct Shared {
    config: UsbRedirectionConfig,
    events: EventOrigin,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    control_channel_id: Option<u32>,
    /// Opened devices announced with `ADD_VIRTUAL_CHANNEL`, waiting for their DVC.
    pending: VecDeque<OpenedDevice>,
    /// Devices announced or redirected, with their device DVC once it exists.
    redirected: HashMap<nusb::DeviceId, Option<u32>>,
    /// Stops the discovery thread.
    discovery: Option<oneshot::Sender<()>>,
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
        let device = {
            let mut state = self.shared.state();
            let device = state.pending.pop_front()?;
            state.redirected.insert(device.id, Some(channel_id));
            device
        };

        let (vendor_id, product_id) = device.vendor_product();
        info!(
            channel_id,
            vendor_id = format_args!("{vendor_id:04x}"),
            product_id = format_args!("{product_id:04x}"),
            "Redirecting USB device"
        );

        let id = device.id;
        let shared = Arc::downgrade(&self.shared);
        let on_close = Box::new(move || {
            if let Some(shared) = shared.upgrade() {
                shared.state().redirected.remove(&id);
            }
        });

        Some(Box::new(NusbDeviceBackend::start(
            device,
            channel_id,
            Arc::clone(self.shared.events.sink()),
            on_close,
        )))
    }
}

fn open(info: &nusb::DeviceInfo) -> Option<OpenedDevice> {
    match OpenedDevice::open(info) {
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
        if shared.config.redirect_at_connect(&device::identity(&info))
            && let Some(device) = open(&info)
            && announce(shared, device)
        {
            count = count.saturating_add(1);
        }
    }
    debug!(count, "Announced USB devices present at connection");
}

/// Queue an opened device for the next device channel and ask the session to request one.
///
/// Returns `false` when the device is already announced or redirected.
fn announce(shared: &Arc<Shared>, device: OpenedDevice) -> bool {
    let id = device.id;
    let control_channel_id = {
        let mut state = shared.state();
        if state.redirected.contains_key(&device.id) {
            return false;
        }
        let Some(control_channel_id) = state.control_channel_id else {
            return false;
        };
        state.redirected.insert(device.id, None);
        state.pending.push_back(device);
        control_channel_id
    };
    let announcement = Announcement {
        shared: Arc::downgrade(shared),
        device: Some(id),
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
    device: Option<nusb::DeviceId>,
}

impl Announcement {
    /// Record that `ADD_VIRTUAL_CHANNEL` was sent, so the device waits for its channel.
    pub fn sent(mut self) {
        self.device = None;
    }
}

impl fmt::Debug for Announcement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Announcement")
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

impl Drop for Announcement {
    fn drop(&mut self) {
        let (Some(id), Some(shared)) = (self.device.take(), self.shared.upgrade()) else {
            return;
        };
        let released = {
            let mut state = shared.state();
            state.redirected.remove(&id);
            let position = state.pending.iter().position(|device| device.id == id);
            position.and_then(|position| state.pending.remove(position))
        };
        if released.is_some() {
            debug!("Released a USB device whose announcement was not sent");
        }
    }
}

async fn run_hotplug(mut watch: nusb::hotplug::HotplugWatch, mut stopped: oneshot::Receiver<()>, shared: Weak<Shared>) {
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
                if !shared.config.redirect_on_hotplug(&device::identity(&info)) {
                    continue;
                }

                let mut device = None;
                for attempt in 1..=HOTPLUG_OPEN_ATTEMPTS {
                    tokio::time::sleep(HOTPLUG_OPEN_DELAY).await;
                    device = if attempt == HOTPLUG_OPEN_ATTEMPTS {
                        open(&info)
                    } else {
                        OpenedDevice::open(&info).ok()
                    };
                    if device.is_some() {
                        break;
                    }
                }
                if let Some(device) = device {
                    announce(&shared, device);
                }
            }
            HotplugEvent::Disconnected(id) => {
                let channel_id = {
                    let mut state = shared.state();
                    state.pending.retain(|device| device.id != id);
                    state.redirected.remove(&id).flatten()
                };
                if let Some(channel_id) = channel_id {
                    debug!(channel_id, "Redirected USB device was unplugged");
                    shared.events.send(UrbdrcEventKind::CloseChannel { channel_id });
                }
            }
        }
    }
}
