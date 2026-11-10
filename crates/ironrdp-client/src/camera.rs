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
    use core::sync::atomic::{AtomicBool, Ordering};
    use core::time::Duration;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use std::thread;

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
    use tokio::sync::mpsc::error::TrySendError;
    use tracing::{debug, info, warn};

    use super::{CameraConfig, CameraEncodingQuality};
    use crate::rdp::{RdpInputEvent, RdpInputSender};

    /// Delay between attempts to wake a session whose input queue is full.
    const HOTPLUG_RETRY_DELAY: Duration = Duration::from_millis(50);

    /// Wakes the session to apply the camera changes queued since the last wake-up.
    pub struct CameraHotplug(Arc<HotplugQueue>);

    impl core::fmt::Debug for CameraHotplug {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("CameraHotplug").finish_non_exhaustive()
        }
    }

    /// Camera changes waiting for the session, kept until it applies them.
    struct HotplugQueue {
        changes: Mutex<Vec<HotplugKind>>,
        /// A wake-up event is queued, or being retried.
        notified: AtomicBool,
        /// Cameras a reopened enumeration channel advertises. Only the session thread changes it,
        /// together with the device listeners, so every advertised camera has a listener.
        devices: Arc<Mutex<Vec<RedirectedDevice>>>,
    }

    impl HotplugQueue {
        /// Queues `change` and makes sure a wake-up event reaches the session.
        fn push(self: &Arc<Self>, change: HotplugKind, sender: &RdpInputSender) {
            lock(&self.changes).push(change);
            if self.notified.swap(true, Ordering::AcqRel) {
                return;
            }
            let wake = || RdpInputEvent::Camera(Box::new(CameraHotplug(Arc::clone(self))));
            if !matches!(sender.try_send(wake()), Err(TrySendError::Full(_))) {
                return;
            }
            // Retry without blocking the watcher; the attempts end with the session.
            debug!("Session input queue is full; retrying the camera change");
            let queue = Arc::clone(self);
            let sender = sender.clone();
            let retry = thread::Builder::new()
                .name("rdpecam-hotplug".to_owned())
                .spawn(move || {
                    loop {
                        thread::sleep(HOTPLUG_RETRY_DELAY);
                        let wake = RdpInputEvent::Camera(Box::new(CameraHotplug(Arc::clone(&queue))));
                        if !matches!(sender.try_send(wake), Err(TrySendError::Full(_))) {
                            break;
                        }
                    }
                });
            if let Err(error) = retry {
                warn!(%error, "Unable to retry a camera change; it applies with the next one");
                self.notified.store(false, Ordering::Release);
            }
        }
    }

    enum HotplugKind {
        Added {
            device: RedirectedDevice,
            listener: DeviceChannelListener<MediaFoundationCamera>,
        },
        Removed {
            channel_name: String,
        },
    }

    enum DeviceChange {
        Added(RedirectedDevice),
        Removed(String),
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
        let hotplug_sender = sender.clone();
        let hotplug_queue = Arc::new(HotplugQueue {
            changes: Mutex::new(Vec::new()),
            notified: AtomicBool::new(false),
            devices: Arc::clone(&devices),
        });
        let watcher = CameraWatcher::new(cameras, move |event| {
            let mut selector = lock(&selector);
            let hotplug = match event {
                CameraEvent::Added(camera) => {
                    let Some((device, listener)) = selector.select(&camera) else {
                        return;
                    };
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
                    HotplugKind::Removed { channel_name }
                }
            };
            hotplug_queue.push(hotplug, &hotplug_sender);
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

    /// Applies the queued camera changes to the active session.
    pub(crate) fn process_hotplug(active_stage: &mut ActiveStage, event: CameraHotplug) -> Option<DvcMessageBatch> {
        let queue = event.0;
        // Changes queued from now on need another wake-up.
        queue.notified.store(false, Ordering::Release);
        let changes = core::mem::take(&mut *lock(&queue.changes));
        if changes.is_empty() {
            return None;
        }

        // Listeners and the advertised list change even without an open enumeration channel, which
        // advertises that list when it opens.
        let drdynvc = active_stage.get_svc_processor_mut::<DrdynvcClient>()?;
        let mut device_changes = Vec::with_capacity(changes.len());
        {
            let mut devices = lock(&queue.devices);
            for change in changes {
                match change {
                    HotplugKind::Added { device, listener } => {
                        let channel_name = device.descriptor().channel_name.clone();
                        devices.retain(|existing| existing.descriptor().channel_name != channel_name);
                        if usize::from(u8::MAX) <= devices.len() {
                            warn!(camera = %device.descriptor().display_name, "Too many redirected cameras");
                            continue;
                        }
                        drdynvc.attach_listener(listener);
                        devices.push(device.clone());
                        device_changes.push(DeviceChange::Added(device));
                    }
                    HotplugKind::Removed { channel_name } => {
                        devices.retain(|device| device.descriptor().channel_name != channel_name);
                        device_changes.push(DeviceChange::Removed(channel_name));
                    }
                }
            }
        }

        let mut enumeration = active_stage.get_dvc_mut::<EnumerationClient>()?;
        let channel_id = enumeration.channel_id();
        let mut messages = Vec::new();
        for change in device_changes {
            match change {
                DeviceChange::Added(device) => {
                    let channel_name = device.descriptor().channel_name.clone();
                    match enumeration.processor_mut().add_device(device) {
                        Ok(change_messages) => messages.extend(change_messages),
                        Err(error) => {
                            warn!(%error, channel = %channel_name, "Camera not advertised");
                            // A reopened enumeration channel must not advertise it either.
                            let advertised = enumeration
                                .processor_mut()
                                .advertised_channels()
                                .any(|advertised| advertised == channel_name);
                            if !advertised {
                                lock(&queue.devices).retain(|device| device.descriptor().channel_name != channel_name);
                            }
                        }
                    }
                }
                // Failing to remove a camera means it was not advertised.
                DeviceChange::Removed(channel_name) => match enumeration.processor_mut().remove_device(&channel_name) {
                    Ok(change_messages) => messages.extend(change_messages),
                    Err(error) => debug!(%error, channel = %channel_name, "Camera removal not advertised"),
                },
            }
        }
        if messages.is_empty() {
            return None;
        }
        DvcMessageBatch::try_new(channel_id, messages)
            .inspect_err(|error| warn!(%error, "Unable to send camera changes"))
            .ok()
    }
}
