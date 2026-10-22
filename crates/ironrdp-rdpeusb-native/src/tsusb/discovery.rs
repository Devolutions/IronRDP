//! Claiming devices through the RemoteFX USB redirection drivers, and following the claimed devices.
//!
//! Devices are matched against the redirection rules while they still use their own drivers, and
//! described then: once claimed, they are only reachable through the generic redirection driver.
//! The filter reports claimed devices as device interfaces of the claim GUID, which are polled.

use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;

use ironrdp_rdpeusb::io::device::DeviceInfo;
use nusb::MaybeFuture as _;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use windows::core::{Error, GUID};

use super::cfgmgr;
use super::control::{FilterControl, GUID_DEVINTERFACE_TSUSBBUS_FILTER};
use super::device::ClaimedDevice;
use crate::device;
use crate::manager::{DeviceKey, Shared, announce, unplugged};

/// How often claimed devices are looked for.
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Polls between two scans for newly plugged devices.
const HOTPLUG_SCAN_POLLS: u32 = 4;

/// The claim of this connection, held by its filter control handle.
struct Session {
    control: FilterControl,
    /// Interface class of the claimed devices, and the file name they are opened under.
    guid: GUID,
    claims: Mutex<Claims>,
}

#[derive(Default)]
struct Claims {
    /// Claimed devices by proprietary ID, as described before they were claimed.
    devices: HashMap<String, (DeviceInfo, String)>,
    /// Devices given back during this connection, which hotplug rules do not claim again.
    released: HashSet<String>,
}

impl Session {
    fn claims(&self) -> MutexGuard<'_, Claims> {
        // Every update is a single map operation, so the claims stay consistent across a panic.
        self.claims.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn claim(&self, claims: &Claims) -> Result<(), Error> {
        let ids: Vec<String> = claims.devices.keys().cloned().collect();
        self.control.claim(&ids)
    }

    /// Give a device back to its own driver.
    fn release(&self, id: &str) {
        let mut claims = self.claims();
        if claims.devices.remove(id).is_some() {
            claims.released.insert(id.to_owned());
            if let Err(error) = self.claim(&claims) {
                warn!(%error, "Failed to release a USB device from redirection");
            }
        }
    }
}

/// Start following devices through the RemoteFX USB redirection drivers, returning the handle that
/// stops it.
///
/// Returns `None` when the drivers cannot be used, most often because the RemoteFX USB redirection
/// policy does not allow this user.
pub(crate) fn start_discovery(shared: &Arc<Shared>) -> Option<oneshot::Sender<()>> {
    let control = FilterControl::open()
        .inspect_err(|error| debug!(%error, "RemoteFX USB redirection is unavailable"))
        .ok()?;
    let guid = control
        .interface_guid()
        .inspect_err(|error| warn!(%error, "Failed to query the RemoteFX USB redirection claim"))
        .ok()?;

    let session = Arc::new(Session {
        control,
        guid,
        claims: Mutex::default(),
    });
    let (stop, stopped) = oneshot::channel();
    let shared = Arc::downgrade(shared);
    thread::Builder::new()
        .name("urbdrc-tsusb-discovery".to_owned())
        .spawn(move || run(&session, &shared, stopped))
        .inspect_err(|error| warn!(%error, "Failed to start the RemoteFX USB discovery thread"))
        .ok()?;

    info!("Redirecting USB devices through the RemoteFX USB redirection drivers");
    Some(stop)
}

fn run(session: &Arc<Session>, shared: &Weak<Shared>, mut stopped: oneshot::Receiver<()>) {
    // Interfaces of claimed devices, with the proprietary ID of those that were opened.
    let mut interfaces = HashMap::new();
    // Devices seen by the last scan, so that the next one tells which were plugged meanwhile.
    let mut seen = None;
    let mut poll = 0u32;

    while matches!(stopped.try_recv(), Err(oneshot::error::TryRecvError::Empty)) {
        let Some(shared) = shared.upgrade() else {
            break;
        };
        if poll.is_multiple_of(HOTPLUG_SCAN_POLLS) {
            claim_devices(session, &shared, &mut seen);
        }
        follow_claimed(session, &shared, &mut interfaces);
        drop(shared);

        poll = poll.wrapping_add(1);
        thread::sleep(POLL_INTERVAL);
    }

    // Closing the control handle releases the claim too; releasing it first returns the devices to
    // their own drivers before the handle is gone.
    if let Err(error) = session.control.claim(&[]) {
        debug!(%error, "Failed to release the USB devices claimed for redirection");
    }
}

/// Claim the devices that the rules select: those present at connection on the first scan, and those
/// plugged in since the previous scan afterwards.
fn claim_devices(session: &Session, shared: &Shared, seen: &mut Option<HashSet<String>>) {
    let devices = match nusb::list_devices().wait() {
        Ok(devices) => devices.collect::<Vec<_>>(),
        Err(error) => {
            warn!(%error, "Failed to enumerate USB devices");
            return;
        }
    };
    let previous = seen.replace(
        devices
            .iter()
            .map(|info| info.instance_id().to_string_lossy().into_owned())
            .collect(),
    );

    let filtered_hubs: HashSet<String> = cfgmgr::interface_paths(&GUID_DEVINTERFACE_TSUSBBUS_FILTER)
        .iter()
        .filter_map(|path| cfgmgr::interface_instance_id(path))
        .map(|instance_id| instance_id.to_uppercase())
        .collect();

    let mut claims = session.claims();
    let mut claimed = 0usize;
    for info in &devices {
        let instance_id = info.instance_id().to_string_lossy();
        let identity = device::identity(info);
        let selected = match &previous {
            None => shared.config.redirect_at_connect(&identity),
            Some(previous) => !previous.contains(instance_id.as_ref()) && shared.config.redirect_on_hotplug(&identity),
        };
        if !selected {
            continue;
        }

        let hub_instance_id = info.parent_instance_id().to_string_lossy();
        if !filtered_hubs.contains(&hub_instance_id.to_uppercase()) {
            debug!(%instance_id, "Skipping a USB device whose hub the RemoteFX USB filter does not filter");
            continue;
        }
        let Some(id) = proprietary_id(session, &instance_id, &hub_instance_id) else {
            debug!(%instance_id, "Skipping a USB device the RemoteFX USB filter does not know");
            continue;
        };
        if claims.devices.contains_key(&id) || claims.released.contains(&id) {
            continue;
        }

        claims.devices.insert(id, device::describe(info, &identity, true));
        claimed = claimed.saturating_add(1);
    }

    if claimed != 0 {
        match session.claim(&claims) {
            Ok(()) => debug!(claimed, "Claimed USB devices for redirection"),
            Err(error) => warn!(%error, "Failed to claim USB devices for redirection"),
        }
    }
}

fn proprietary_id(session: &Session, instance_id: &str, hub_instance_id: &str) -> Option<String> {
    let device_pdo = cfgmgr::pdo_name(cfgmgr::locate(instance_id)?)?;
    let hub_pdo = cfgmgr::pdo_name(cfgmgr::locate(hub_instance_id)?)?;
    session
        .control
        .proprietary_id(&hub_pdo, &device_pdo)
        .inspect_err(|error| debug!(%error, "Failed to query a USB device proprietary ID"))
        .ok()
}

/// Announce the claimed devices that appeared, and stop redirecting those that disappeared.
fn follow_claimed(session: &Arc<Session>, shared: &Arc<Shared>, interfaces: &mut HashMap<String, Option<String>>) {
    let present: HashSet<String> = cfgmgr::interface_paths(&session.guid).into_iter().collect();

    interfaces.retain(|path, id| {
        let keep = present.contains(path);
        if !keep && let Some(id) = id {
            unplugged(shared, &DeviceKey::TsUsb(id.clone()));
        }
        keep
    });

    for path in present {
        if interfaces.contains_key(&path) {
            continue;
        }
        let claimed = match ClaimedDevice::open(&path, &session.guid) {
            Ok(claimed) => claimed,
            Err(error) => {
                // The driver may still be starting; the next poll retries.
                debug!(%error, "Failed to open a claimed USB device");
                continue;
            }
        };

        let id = claimed.proprietary_id.clone();
        let described = session.claims().devices.get(&id).cloned();
        let Some((info, description)) = described else {
            debug!("Ignoring a claimed USB device that was not described");
            interfaces.insert(path, None);
            continue;
        };

        let release = {
            let session = Arc::downgrade(session);
            let id = id.clone();
            Box::new(move || {
                if let Some(session) = session.upgrade() {
                    session.release(&id);
                }
            })
        };
        announce(shared, Box::new(claimed.describe(info, description, release)));
        interfaces.insert(path, Some(id));
    }
}
