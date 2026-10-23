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
/// Scans that try to claim a selected device whose hub filter or proprietary ID is not ready yet.
const CLAIM_ATTEMPTS: u32 = 5;

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
    /// Devices given back while still plugged in, after the server retracted them or closed their
    /// channel. They come back with their own driver, and are not claimed again during this
    /// connection. Unplugged devices are not listed, so that plugging them in again claims them.
    released: HashSet<String>,
}

impl Claims {
    /// Give back a device that is still plugged in, returning whether it was claimed.
    fn release(&mut self, id: &str) -> bool {
        let claimed = self.devices.remove(id).is_some();
        if claimed {
            self.released.insert(id.to_owned());
        }
        claimed
    }

    /// Forget an unplugged device, returning whether it was claimed.
    fn forget(&mut self, id: &str) -> bool {
        self.devices.remove(id).is_some()
    }

    /// Whether a device may be claimed.
    fn may_claim(&self, id: &str) -> bool {
        !self.devices.contains_key(id) && !self.released.contains(id)
    }
}

/// What the scans for devices to claim remember between them.
#[derive(Default)]
struct Scans {
    /// Instance IDs of the devices the last scan saw, or `None` before the first scan.
    seen: Option<HashSet<String>>,
    /// Selected devices that could not be claimed yet, by instance ID.
    retries: HashMap<String, Retry>,
}

#[derive(Debug, Clone, Copy)]
struct Retry {
    attempts_left: u32,
    /// The device was selected by the rules for devices present at connection.
    at_connect: bool,
}

impl Retry {
    fn first(at_connect: bool) -> Self {
        Self {
            attempts_left: CLAIM_ATTEMPTS,
            at_connect,
        }
    }

    /// The next attempt, unless this was the last one.
    fn next(self) -> Option<Self> {
        let attempts_left = self.attempts_left.checked_sub(1).filter(|left| *left != 0)?;
        Some(Self { attempts_left, ..self })
    }
}

impl Scans {
    /// Record the devices a scan saw, returning how each one the rules select was selected.
    ///
    /// The first scan selects the devices present at connection; later ones, the devices plugged in
    /// since the previous scan, and those still being retried.
    fn select(
        &mut self,
        instance_ids: &[String],
        at_connect: impl Fn(usize) -> bool,
        on_hotplug: impl Fn(usize) -> bool,
    ) -> Vec<(usize, Retry)> {
        let previous = self.seen.replace(instance_ids.iter().cloned().collect());
        let mut retries = core::mem::take(&mut self.retries);
        instance_ids
            .iter()
            .enumerate()
            .filter_map(|(index, instance_id)| {
                let retry = match (retries.remove(instance_id), &previous) {
                    (Some(retry), _) => retry,
                    (None, None) => Retry::first(true),
                    (None, Some(previous)) if !previous.contains(instance_id) => Retry::first(false),
                    (None, Some(_)) => return None,
                };
                let selected = if retry.at_connect {
                    at_connect(index)
                } else {
                    on_hotplug(index)
                };
                selected.then_some((index, retry))
            })
            .collect()
    }

    /// Try again at the next scan, unless the attempts ran out.
    fn defer(&mut self, instance_id: String, retry: Retry) {
        if let Some(retry) = retry.next() {
            self.retries.insert(instance_id, retry);
        }
    }
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

    /// Give a device that is still plugged in back to its own driver.
    fn release(&self, id: &str) {
        let mut claims = self.claims();
        if claims.release(id)
            && let Err(error) = self.claim(&claims)
        {
            warn!(%error, "Failed to release a USB device from redirection");
        }
    }

    /// Forget the claim of an unplugged device, so that plugging it in again claims it.
    fn unplugged(&self, id: &str) {
        let mut claims = self.claims();
        if claims.forget(id)
            && let Err(error) = self.claim(&claims)
        {
            debug!(%error, "Failed to forget the claim of an unplugged USB device");
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
    let mut scans = Scans::default();
    let mut poll = 0u32;

    while matches!(stopped.try_recv(), Err(oneshot::error::TryRecvError::Empty)) {
        let Some(shared) = shared.upgrade() else {
            break;
        };
        if poll.is_multiple_of(HOTPLUG_SCAN_POLLS) {
            claim_devices(session, &shared, &mut scans);
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
///
/// A device that was just plugged in may not be known to the hub filter yet, so a selected device
/// that cannot be claimed is tried again at the next scans.
fn claim_devices(session: &Session, shared: &Shared, scans: &mut Scans) {
    let devices = match nusb::list_devices().wait() {
        Ok(devices) => devices.collect::<Vec<_>>(),
        Err(error) => {
            warn!(%error, "Failed to enumerate USB devices");
            return;
        }
    };
    let instance_ids: Vec<String> = devices
        .iter()
        .map(|info| info.instance_id().to_string_lossy().into_owned())
        .collect();
    let identities: Vec<_> = devices.iter().map(device::identity).collect();
    let selected = scans.select(
        &instance_ids,
        |index| {
            identities
                .get(index)
                .is_some_and(|identity| shared.config.redirect_at_connect(identity))
        },
        |index| {
            identities
                .get(index)
                .is_some_and(|identity| shared.config.redirect_on_hotplug(identity))
        },
    );
    if selected.is_empty() {
        return;
    }

    let filtered_hubs: HashSet<String> = cfgmgr::interface_paths(&GUID_DEVINTERFACE_TSUSBBUS_FILTER)
        .iter()
        .filter_map(|path| cfgmgr::interface_instance_id(path))
        .map(|instance_id| instance_id.to_uppercase())
        .collect();

    let mut claims = session.claims();
    let mut claimed = Vec::new();
    for (index, retry) in selected {
        let (Some(info), Some(instance_id), Some(identity)) =
            (devices.get(index), instance_ids.get(index), identities.get(index))
        else {
            continue;
        };

        let hub_instance_id = info.parent_instance_id().to_string_lossy();
        if !filtered_hubs.contains(&hub_instance_id.to_uppercase()) {
            debug!(%instance_id, "The RemoteFX USB filter does not filter the hub of a USB device yet");
            scans.defer(instance_id.clone(), retry);
            continue;
        }
        let Some(id) = proprietary_id(session, instance_id, &hub_instance_id) else {
            debug!(%instance_id, "The RemoteFX USB filter does not know a USB device yet");
            scans.defer(instance_id.clone(), retry);
            continue;
        };
        if !claims.may_claim(&id) {
            continue;
        }

        claims
            .devices
            .insert(id.clone(), device::describe(info, identity, true));
        claimed.push((id, instance_id.clone(), retry));
    }

    if claimed.is_empty() {
        return;
    }
    match session.claim(&claims) {
        Ok(()) => debug!(claimed = claimed.len(), "Claimed USB devices for redirection"),
        Err(error) => {
            warn!(%error, "Failed to claim USB devices for redirection");
            for (id, instance_id, retry) in claimed {
                claims.devices.remove(&id);
                scans.defer(instance_id, retry);
            }
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
            // Forgotten before the channel closes, so that releasing the device does not keep it from
            // being claimed when it is plugged in again.
            session.unplugged(id);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn scans_select_present_then_plugged_devices() {
        let mut scans = Scans::default();
        let first = scans.select(&ids(&["a", "b"]), |index| index == 1, |_| true);
        assert_eq!(first.iter().map(|(index, _)| *index).collect::<Vec<_>>(), [1]);
        assert!(first.iter().all(|(_, retry)| retry.at_connect));

        // Only the device plugged in since, by the hotplug rules.
        let second = scans.select(&ids(&["a", "b", "c"]), |_| false, |_| true);
        assert_eq!(second.iter().map(|(index, _)| *index).collect::<Vec<_>>(), [2]);
        assert!(second.iter().all(|(_, retry)| !retry.at_connect));
    }

    #[test]
    fn devices_not_ready_are_retried_a_few_times() {
        let mut scans = Scans::default();
        let _ = scans.select(&ids(&["a"]), |_| false, |_| false);
        let plugged = scans.select(&ids(&["a", "b"]), |_| false, |_| true);
        let [(1, retry)] = plugged.as_slice() else {
            panic!("expected the plugged device");
        };

        // Not claimable yet: the next scans select it again, under the rules that selected it.
        let mut retry = *retry;
        for _ in 1..CLAIM_ATTEMPTS {
            scans.defer("b".to_owned(), retry);
            let again = scans.select(&ids(&["a", "b"]), |_| false, |_| true);
            let [(1, next)] = again.as_slice() else {
                panic!("expected a retry");
            };
            assert!(!next.at_connect);
            retry = *next;
        }

        // The attempts ran out.
        scans.defer("b".to_owned(), retry);
        assert!(scans.select(&ids(&["a", "b"]), |_| false, |_| true).is_empty());
    }

    fn claimed(id: &str) -> Claims {
        let mut claims = Claims::default();
        let info = DeviceInfo {
            location: ironrdp_rdpeusb::io::device::UsbDeviceLocation {
                bus_number: 1,
                address: 2,
                port_numbers: vec![3],
            },
            descriptor: ironrdp_rdpeusb::io::device::UsbDeviceDescriptorInfo {
                vendor_id: 0x1050,
                product_id: 0x0407,
                device_version: 0x0543,
                usb_version: ironrdp_usb::BcdVersion::from_raw(0x0200),
                class_codes: ironrdp_rdpeusb::io::device::UsbClassCodes::PER_INTERFACE,
                num_configurations: 1,
            },
            active_config: None,
            speed: ironrdp_rdpeusb::io::device::UsbConnectionSpeed::Full,
            isochronous: true,
        };
        claims.devices.insert(id.to_owned(), (info, String::from("YubiKey")));
        claims
    }

    #[test]
    fn unplugged_devices_may_be_claimed_again() {
        // Unplugged: its channel closes and releases it, which must not keep it from coming back.
        let mut claims = claimed("yubikey");
        assert!(claims.forget("yubikey"));
        assert!(!claims.release("yubikey"));
        assert!(claims.may_claim("yubikey"));
    }

    #[test]
    fn released_devices_are_not_claimed_again() {
        // Retracted by the server while still plugged in.
        let mut claims = claimed("yubikey");
        assert!(!claims.may_claim("yubikey"));
        assert!(claims.release("yubikey"));
        assert!(!claims.may_claim("yubikey"));
    }

    #[test]
    fn unplugged_devices_are_selected_when_plugged_in_again() {
        let mut scans = Scans::default();
        let _ = scans.select(&ids(&["a"]), |_| true, |_| true);
        assert!(scans.select(&ids(&[]), |_| true, |_| true).is_empty());
        let replugged = scans.select(&ids(&["a"]), |_| false, |_| true);
        assert_eq!(replugged.len(), 1);
    }
}
