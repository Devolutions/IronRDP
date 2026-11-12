//! Audio device enumeration and selection.

use core::fmt;
use core::str::FromStr as _;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::thread;

use cpal::traits::{DeviceTrait as _, HostTrait as _};
use tracing::{debug, info, warn};

/// How often a stream thread checks for device changes.
pub(crate) const DEVICE_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Delay before a stream thread retries a device that failed to open.
pub(crate) const DEVICE_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Direction of an audio device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Output,
    Input,
}

/// Which device to open: the system default, or a specific one.
///
/// A device given by name matches the first device whose name equals it, ignoring case, or failing
/// that, the first whose name contains it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DeviceSelector {
    /// The system default device, followed when it changes.
    #[default]
    Default,
    /// A device ID as reported by [`list_devices`].
    Id(String),
    /// A device name, or part of one.
    Name(String),
}

impl DeviceSelector {
    /// Interprets user input: `default` (or an empty string), a device ID, or a device name.
    ///
    /// Device IDs carry the host prefix (`wasapi:`, `coreaudio:`, `alsa:` and so on), so any input
    /// that parses as an ID is treated as one.
    pub fn parse(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty() || value.eq_ignore_ascii_case("default") {
            Self::Default
        } else if cpal::DeviceId::from_str(value).is_ok() {
            Self::Id(value.to_owned())
        } else {
            Self::Name(value.to_owned())
        }
    }

    /// Returns whether this selector follows the system default device.
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }
}

impl fmt::Display for DeviceSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::Id(id) => f.write_str(id),
            Self::Name(name) => f.write_str(name),
        }
    }
}

/// An audio device found on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDeviceInfo {
    /// Stable identifier, usable with [`DeviceSelector::Id`].
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Whether this is the system default device for its direction.
    pub is_default: bool,
}

/// Lists the devices of the default host for `direction`.
pub fn list_devices(direction: Direction) -> Vec<AudioDeviceInfo> {
    let host = cpal::default_host();
    let default_id = default_device(&host, direction).and_then(|device| device_id(&device));
    let devices = match direction {
        Direction::Output => host.output_devices().map(|devices| devices.collect::<Vec<_>>()),
        Direction::Input => host.input_devices().map(|devices| devices.collect::<Vec<_>>()),
    };
    let devices = match devices {
        Ok(devices) => devices,
        Err(error) => {
            warn!(%error, ?direction, "Failed to enumerate audio devices");
            return Vec::new();
        }
    };

    devices
        .iter()
        .filter_map(|device| {
            let id = device_id(device)?;
            Some(AudioDeviceInfo {
                is_default: default_id.as_deref() == Some(id.as_str()),
                name: device_name(device),
                id,
            })
        })
        .collect()
}

/// Opens the device `selector` designates, or `None` when no such device exists.
pub(crate) fn open_device(selector: &DeviceSelector, direction: Direction) -> Option<cpal::Device> {
    let host = cpal::default_host();
    match selector {
        DeviceSelector::Default => default_device(&host, direction),
        DeviceSelector::Id(id) => {
            let parsed = cpal::DeviceId::from_str(id).ok()?;
            host.device_by_id(&parsed)
        }
        DeviceSelector::Name(name) => {
            let devices: Vec<cpal::Device> = match direction {
                Direction::Output => host.output_devices().ok()?.collect(),
                Direction::Input => host.input_devices().ok()?.collect(),
            };
            let wanted = name.to_lowercase();
            let names: Vec<String> = devices
                .iter()
                .map(|device| device_name(device).to_lowercase())
                .collect();
            let index = names
                .iter()
                .position(|name| *name == wanted)
                .or_else(|| names.iter().position(|name| name.contains(&wanted)));
            debug!(%name, matched = ?index.map(|index| &names[index]), "Resolved audio device by name");
            index.map(|index| devices[index].clone())
        }
    }
}

/// Parks a stream thread until its stream has to stop: when `ended` is set, when `lost` reports
/// that the device disappeared, or, for a stream following the system default, when the default
/// device changes.
///
/// `on_wake` runs on every wake-up that does not end the stream, for the caller's periodic work.
pub(crate) fn watch_device(
    selector: &DeviceSelector,
    direction: Direction,
    device_id: Option<&str>,
    ended: &AtomicBool,
    lost: &AtomicBool,
    mut on_wake: impl FnMut(),
) {
    loop {
        thread::park_timeout(DEVICE_POLL_INTERVAL);
        if ended.load(Ordering::Relaxed) {
            return;
        }
        on_wake();
        if lost.swap(false, Ordering::Relaxed) {
            info!(?direction, device = ?device_id, "Audio device lost; reopening");
            return;
        }
        if selector.is_default() {
            let default_id = default_device_id(direction);
            if default_id.is_some() && default_id.as_deref() != device_id {
                info!(?direction, from = ?device_id, to = ?default_id, "Default audio device changed");
                return;
            }
        }
    }
}

/// Returns the ID of the current system default device for `direction`.
pub(crate) fn default_device_id(direction: Direction) -> Option<String> {
    default_device(&cpal::default_host(), direction).and_then(|device| device_id(&device))
}

pub(crate) fn device_id(device: &cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

/// Returns the platform endpoint ID (the part after the host prefix).
#[cfg_attr(not(windows), expect(dead_code, reason = "only the WASAPI session volume needs it"))]
pub(crate) fn endpoint_id(device: &cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.1)
}

fn device_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|description| description.name().to_owned())
        .unwrap_or_else(|_| "<unnamed>".to_owned())
}

fn default_device(host: &cpal::Host, direction: Direction) -> Option<cpal::Device> {
    match direction {
        Direction::Output => host.default_output_device(),
        Direction::Input => host.default_input_device(),
    }
}
