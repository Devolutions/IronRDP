//! Camera redirection ([MS-RDPECAM]) configuration and session wiring.
//!
//! [MS-RDPECAM]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/

/// Quality of the H.264 stream produced for cameras that only offer raw frames.
///
/// The levels mirror MSTSC's `CameraRedirEncodingQuality`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraEncodingQuality {
    Low,
    #[default]
    Medium,
    High,
}

/// Cameras to redirect, as described by the `camerastoredirect` `.rdp` property.
///
/// The property is a `;`-separated list: `*` selects every camera, including cameras attached
/// during the session, an entry selects one camera, and an entry prefixed with `-` excludes one.
/// Entries match a camera's symbolic link or, for convenience, its friendly name, ignoring case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraConfig {
    pub redirect_all: bool,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Encode raw camera frames to H.264 when the camera has no compressed format.
    pub encode_video: bool,
    pub encoding_quality: CameraEncodingQuality,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            redirect_all: false,
            include: Vec::new(),
            exclude: Vec::new(),
            encode_video: true,
            encoding_quality: CameraEncodingQuality::default(),
        }
    }
}

impl CameraConfig {
    /// Parses a `camerastoredirect` value.
    pub fn from_rdp_value(value: &str) -> Self {
        let mut config = Self::default();
        config.apply_rdp_value(value);
        config
    }

    /// Replaces the camera selection with a `camerastoredirect` value, keeping encoding options.
    pub fn apply_rdp_value(&mut self, value: &str) {
        self.redirect_all = false;
        self.include.clear();
        self.exclude.clear();
        for entry in value.split(';').map(str::trim).filter(|entry| !entry.is_empty()) {
            if entry == "*" {
                self.redirect_all = true;
            } else if let Some(excluded) = entry.strip_prefix('-') {
                self.exclude.push(excluded.trim().to_owned());
            } else {
                self.include.push(entry.to_owned());
            }
        }
    }

    /// Returns whether any camera may be redirected.
    pub fn is_enabled(&self) -> bool {
        self.redirect_all || !self.include.is_empty()
    }

    /// Returns whether the camera with this symbolic link and friendly name is selected.
    pub fn selects(&self, symbolic_link: &str, friendly_name: &str) -> bool {
        let matches =
            |entry: &String| entry.eq_ignore_ascii_case(symbolic_link) || entry.eq_ignore_ascii_case(friendly_name);
        if self.exclude.iter().any(matches) {
            return false;
        }
        self.redirect_all || self.include.iter().any(matches)
    }
}

#[cfg(windows)]
pub use native::CameraHotplug;
#[cfg(windows)]
pub(crate) use native::{attach, process_hotplug};

#[cfg(windows)]
mod native {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use ironrdp_dvc::{DrdynvcClient, DvcMessageBatch};
    use ironrdp_pdu::pdu_other_err;
    use ironrdp_rdpecam::client::DeviceChannelListener;
    use ironrdp_rdpecam::{
        CameraRedirectionSession, DeviceDescriptor, EnumerationClient, RedirectedDevice, SampleUplink,
    };
    use ironrdp_rdpecam_native::{
        CameraDeviceInfo, CameraEvent, CameraOptions, CameraWatcher, EncodingQuality, MediaFoundationCamera,
        enumerate_cameras,
    };
    use ironrdp_session::ActiveStage;
    use tracing::{debug, info, warn};

    use super::{CameraConfig, CameraEncodingQuality};
    use crate::rdp::{RdpInputEvent, RdpInputSender};

    /// Camera attached or removed while the session runs.
    pub struct CameraHotplug(HotplugKind);

    enum HotplugKind {
        Added {
            device: RedirectedDevice,
            listener: DeviceChannelListener<MediaFoundationCamera>,
        },
        Removed {
            channel_name: String,
        },
    }

    impl core::fmt::Debug for CameraHotplug {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match &self.0 {
                HotplugKind::Added { device, .. } => f
                    .debug_struct("Added")
                    .field("device", device.descriptor())
                    .finish_non_exhaustive(),
                HotplugKind::Removed { channel_name } => {
                    f.debug_struct("Removed").field("channel_name", channel_name).finish()
                }
            }
        }
    }

    /// Selection state shared by the initial enumeration and the device watcher.
    struct CameraSelector {
        session: CameraRedirectionSession,
        config: CameraConfig,
        uplink: SampleUplink,
        /// Channel name assigned to each camera, by identity key, kept across removal and arrival.
        channel_names: BTreeMap<String, String>,
    }

    impl CameraSelector {
        fn select(
            &mut self,
            camera: &CameraDeviceInfo,
        ) -> Option<(RedirectedDevice, DeviceChannelListener<MediaFoundationCamera>)> {
            if !self.config.selects(&camera.symbolic_link, &camera.friendly_name) {
                return None;
            }
            let channel_name = self.channel_name(&camera.symbolic_link);
            let descriptor = match DeviceDescriptor::new(camera.friendly_name.clone(), channel_name) {
                Ok(descriptor) => descriptor,
                Err(error) => {
                    warn!(%error, camera = %camera.friendly_name, "Unable to describe camera");
                    return None;
                }
            };
            let device = match self.session.select_device(descriptor) {
                Ok(device) => device,
                Err(error) => {
                    warn!(%error, camera = %camera.friendly_name, "Unable to select camera");
                    return None;
                }
            };
            let symbolic_link = camera.symbolic_link.clone();
            let options = CameraOptions {
                encode_h264: self.config.encode_video,
                quality: match self.config.encoding_quality {
                    CameraEncodingQuality::Low => EncodingQuality::Low,
                    CameraEncodingQuality::Medium => EncodingQuality::Medium,
                    CameraEncodingQuality::High => EncodingQuality::High,
                },
            };
            let listener = device.listener(
                move || Some(MediaFoundationCamera::with_options(symbolic_link.clone(), options)),
                Arc::clone(&self.uplink),
            );
            info!(camera = %camera.friendly_name, channel = %device.descriptor().channel_name, "Redirecting camera");
            Some((device, listener))
        }

        fn channel_name(&mut self, symbolic_link: &str) -> String {
            let next = self.channel_names.len();
            self.channel_names
                .entry(CameraDeviceInfo::identity_key(symbolic_link))
                .or_insert_with(|| format!("RDCamera_Device_{next}"))
                .clone()
        }
    }

    /// Registers the camera enumeration channel and the selected cameras' device channels.
    pub(crate) fn attach(mut drdynvc: DrdynvcClient, config: &CameraConfig, sender: &RdpInputSender) -> DrdynvcClient {
        if !config.is_enabled() {
            return drdynvc;
        }
        let cameras = match enumerate_cameras() {
            Ok(cameras) => cameras,
            Err(error) => {
                warn!(%error, "Unable to enumerate cameras; camera redirection is disabled");
                return drdynvc;
            }
        };

        let uplink_sender = sender.clone();
        let uplink: SampleUplink = Arc::new(move |channel_id, messages| {
            uplink_sender
                .try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                .map_err(|_| pdu_other_err!("send camera samples to the event loop"))
        });
        let session = CameraRedirectionSession::new();
        let mut selector = CameraSelector {
            session: session.clone(),
            config: config.clone(),
            uplink,
            channel_names: BTreeMap::new(),
        };

        let mut devices = Vec::new();
        for camera in &cameras {
            if let Some((device, listener)) = selector.select(camera) {
                devices.push(device);
                drdynvc = drdynvc.with_listener(listener);
            }
        }
        if devices.is_empty() && !config.redirect_all {
            info!("No selected camera is attached");
        }

        if let Err(error) = session.enumeration_client(devices.clone()) {
            warn!(%error, "Unable to create the camera enumeration channel");
            return drdynvc;
        }
        // Cameras to advertise, kept current by the watcher so that a reopened enumeration channel
        // (Windows reopens it when the user desktop replaces the logon desktop) lists them all.
        let devices = Arc::new(Mutex::new(devices));

        let selector = Arc::new(Mutex::new(selector));
        let watched_devices = Arc::clone(&devices);
        let hotplug_sender = sender.clone();
        let watcher = CameraWatcher::new(cameras, move |event| {
            let mut selector = lock(&selector);
            let hotplug = match event {
                CameraEvent::Added(camera) => {
                    let Some((device, listener)) = selector.select(&camera) else {
                        return;
                    };
                    let mut devices = lock(&watched_devices);
                    if usize::from(u8::MAX) <= devices.len() {
                        warn!(camera = %camera.friendly_name, "Too many redirected cameras");
                        return;
                    }
                    devices.retain(|existing| existing.descriptor().channel_name != device.descriptor().channel_name);
                    devices.push(device.clone());
                    HotplugKind::Added { device, listener }
                }
                CameraEvent::Removed { symbolic_link } => {
                    let Some(channel_name) = selector
                        .channel_names
                        .get(&CameraDeviceInfo::identity_key(&symbolic_link))
                        .cloned()
                    else {
                        return;
                    };
                    lock(&watched_devices).retain(|device| device.descriptor().channel_name != channel_name);
                    HotplugKind::Removed { channel_name }
                }
            };
            if hotplug_sender
                .try_send(RdpInputEvent::Camera(Box::new(CameraHotplug(hotplug))))
                .is_err()
            {
                warn!("Dropping a camera change because the session input queue is full");
            }
        });
        let watcher = match watcher {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                warn!(%error, "Unable to watch for camera changes");
                None
            }
        };

        drdynvc.with_dynamic_channel_factory(ironrdp_rdpecam::ENUMERATION_CHANNEL_NAME, move || {
            // The factory keeps the watcher alive for the lifetime of the connection.
            let _watcher = &watcher;
            let advertised = lock(&devices).clone();
            session.enumeration_client(advertised).unwrap_or_else(|error| {
                warn!(%error, "Advertising no camera");
                session
                    .enumeration_client(Vec::new())
                    .unwrap_or_else(|_| unreachable!("an empty camera list is always valid"))
            })
        })
    }

    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Applies a camera change to the active session.
    pub(crate) fn process_hotplug(active_stage: &mut ActiveStage, event: CameraHotplug) -> Option<DvcMessageBatch> {
        let (device, removed) = match event.0 {
            HotplugKind::Added { device, listener } => {
                let drdynvc = active_stage.get_svc_processor_mut::<DrdynvcClient>()?;
                drdynvc.attach_listener(listener);
                (Some(device), None)
            }
            HotplugKind::Removed { channel_name } => (None, Some(channel_name)),
        };
        let mut enumeration = active_stage.get_dvc_mut::<EnumerationClient>()?;
        let channel_id = enumeration.channel_id();
        let result = match (device, removed) {
            (Some(device), _) => enumeration.processor_mut().add_device(device),
            (None, Some(channel_name)) => enumeration.processor_mut().remove_device(&channel_name),
            (None, None) => return None,
        };
        let messages = match result {
            Ok(messages) => messages,
            Err(error) => {
                debug!(%error, "Camera change not applied");
                return None;
            }
        };
        if messages.is_empty() {
            return None;
        }
        DvcMessageBatch::try_new(channel_id, messages)
            .inspect_err(|error| warn!(%error, "Unable to send camera change"))
            .ok()
    }
}
