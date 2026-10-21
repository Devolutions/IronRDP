//! Device discovery and the URBDRC device manager.

use core::time::Duration;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;

use futures_core::Stream as _;
use ironrdp_dvc::DvcMessage;
use ironrdp_pdu::PduResult;
use ironrdp_rdpeusb::client::{DeviceManagerBackend, UrbdrcDeviceBackend};
use ironrdp_rdpeusb::pdu::sink::AddVirtualChannel;
use nusb::MaybeFuture as _;
use nusb::hotplug::HotplugEvent;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};

use crate::device::{self, NusbDeviceBackend, OpenedDevice};
use crate::filter::UsbRedirectionConfig;
use crate::{UrbdrcEvent, UrbdrcEventSink};

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
}

struct Shared {
    config: UsbRedirectionConfig,
    sink: UrbdrcEventSink,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    control_channel_id: Option<u32>,
    /// Opened devices announced with `ADD_VIRTUAL_CHANNEL`, waiting for their DVC.
    pending: VecDeque<OpenedDevice>,
    /// Devices announced or redirected, with their device DVC once it exists.
    redirected: HashMap<nusb::DeviceId, Option<u32>>,
    hotplug: Option<oneshot::Sender<()>>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        // The state stays consistent across a panic: every update is a single collection operation.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl NusbDeviceManager {
    pub(crate) fn new(config: UsbRedirectionConfig, sink: UrbdrcEventSink) -> Self {
        Self {
            shared: Arc::new(Shared {
                config,
                sink,
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// Build the callback run once the control channel completed its capability exchange.
    pub(crate) fn on_capability_exchanged(&self) -> impl FnOnce() -> PduResult<Vec<DvcMessage>> + Send + 'static {
        let shared = Arc::clone(&self.shared);
        move || Ok(announce_present_devices(&shared))
    }
}

impl Drop for NusbDeviceManager {
    fn drop(&mut self) {
        if let Some(stop) = self.shared.state().hotplug.take() {
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
            Arc::clone(&self.shared.sink),
            on_close,
        )))
    }
}

/// Open the devices selected at connection time and announce them.
fn announce_present_devices(shared: &Arc<Shared>) -> Vec<DvcMessage> {
    let mut messages: Vec<DvcMessage> = Vec::new();

    match nusb::list_devices().wait() {
        Ok(devices) => {
            for info in devices {
                if !shared.config.redirect_at_connect(&device::identity(&info)) {
                    continue;
                }
                let Some(device) = open(&info) else {
                    continue;
                };

                let mut state = shared.state();
                if state.redirected.contains_key(&device.id) {
                    continue;
                }
                state.redirected.insert(device.id, None);
                state.pending.push_back(device);
                messages.push(Box::new(AddVirtualChannel { msg_id: 0 }));
            }
        }
        Err(error) => warn!(%error, "Failed to enumerate USB devices"),
    }

    if shared.config.needs_hotplug() {
        let hotplug = watch_hotplug(Arc::downgrade(shared));
        shared.state().hotplug = hotplug;
    }

    debug!(count = messages.len(), "Announcing USB devices");
    messages
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

/// Start the hotplug watcher thread, returning the handle that stops it.
fn watch_hotplug(shared: Weak<Shared>) -> Option<oneshot::Sender<()>> {
    let watch = match nusb::watch_devices() {
        Ok(watch) => watch,
        Err(error) => {
            warn!(%error, "USB hotplug is unavailable; only devices present at connection are redirected");
            return None;
        }
    };
    let (stop, stopped) = oneshot::channel();

    let spawned = thread::Builder::new().name("urbdrc-hotplug".to_owned()).spawn(move || {
        match tokio::runtime::Builder::new_current_thread().enable_time().build() {
            Ok(runtime) => runtime.block_on(run_hotplug(watch, stopped, shared)),
            Err(error) => warn!(%error, "Failed to start the USB hotplug runtime"),
        }
    });
    if let Err(error) = spawned {
        warn!(%error, "Failed to start the USB hotplug thread");
        return None;
    }

    Some(stop)
}

async fn run_hotplug(mut watch: nusb::hotplug::HotplugWatch, mut stopped: oneshot::Receiver<()>, shared: Weak<Shared>) {
    loop {
        let event = tokio::select! {
            _ = &mut stopped => break,
            event = core::future::poll_fn(|cx| Pin::new(&mut watch).poll_next(cx)) => event,
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
                let Some(device) = device else {
                    continue;
                };

                let control_channel_id = {
                    let mut state = shared.state();
                    if state.redirected.contains_key(&device.id) {
                        continue;
                    }
                    let Some(control_channel_id) = state.control_channel_id else {
                        continue;
                    };
                    state.redirected.insert(device.id, None);
                    state.pending.push_back(device);
                    control_channel_id
                };
                (shared.sink)(UrbdrcEvent::AddVirtualChannel { control_channel_id });
            }
            HotplugEvent::Disconnected(id) => {
                let channel_id = {
                    let mut state = shared.state();
                    state.pending.retain(|device| device.id != id);
                    state.redirected.remove(&id).flatten()
                };
                if let Some(channel_id) = channel_id {
                    debug!(channel_id, "Redirected USB device was unplugged");
                    (shared.sink)(UrbdrcEvent::CloseChannel { channel_id });
                }
            }
        }
    }
}
