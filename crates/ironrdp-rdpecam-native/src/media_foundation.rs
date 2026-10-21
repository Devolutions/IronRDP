//! Media Foundation camera capture backend.

use core::ffi::c_void;
use core::fmt;
use core::time::Duration;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};

use ironrdp_rdpecam::{
    CameraBackend, ErrorCode, MediaFormat, MediaType, PropertyDescription, PropertyId, PropertyMode, PropertySet,
    PropertyValue, SampleDelivery, SampleSink, StartStreamInfo, StreamDescription,
};
use tracing::{debug, info, trace, warn};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_NOTIFY_ACTION, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_0,
    CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CM_Register_Notification, CM_Unregister_Notification, CR_SUCCESS,
    HCMNOTIFICATION,
};
use windows::Win32::Media::DirectShow::{IAMCameraControl, IAMVideoProcAmp};
use windows::Win32::Media::KernelStreaming::KSCATEGORY_VIDEO_CAMERA;
use windows::Win32::Media::MediaFoundation::{
    CLSID_MSH264EncoderMFT, CODECAPI_AVEncMPVGOPSize, CODECAPI_AVLowLatencyMode, ICodecAPI, IMF2DBuffer, IMFActivate,
    IMFAttributes, IMFMediaBuffer, IMFMediaSource, IMFMediaType, IMFSample, IMFSourceReader, IMFTransform,
    MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
    MF_E_NO_MORE_TYPES, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_MT_AVG_BITRATE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE,
    MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_MPEG2_PROFILE,
    MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_READWRITE_DISABLE_CONVERTERS, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR, MF_VERSION,
    MFCreateAttributes, MFCreateDeviceSource, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources, MFMediaType_Video, MFSTARTUP_NOSOCKET,
    MFSampleExtension_CleanPoint, MFShutdown, MFStartup, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFVideoFormat_H264,
    MFVideoFormat_I420, MFVideoFormat_IYUV, MFVideoFormat_MJPG, MFVideoFormat_NV12, MFVideoFormat_RGB24,
    MFVideoFormat_RGB32, MFVideoFormat_YUY2, MFVideoInterlace_Progressive, eAVEncH264VProfile_Base,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{GUID, Interface as _, PWSTR};

/// Index of the single capture stream exposed for each device.
const STREAM_INDEX: u8 = 0;
/// Longest time a backend call waits for the device worker thread.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest number of captured H.264 pictures kept while the server has no outstanding request.
///
/// H.264 pictures cannot be dropped individually, so they are queued; when the queue overflows,
/// it is emptied and capture resumes at the next key frame.
const MAX_QUEUED_H264_FRAMES: usize = 30;
/// Largest picture area encoded to H.264 by the software encoder.
const MAX_ENCODED_PIXELS: u32 = 1920 * 1080;

/// Quality of the H.264 stream produced when a camera only offers raw frames.
///
/// The values mirror the MSTSC `CameraRedirEncodingQuality` levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EncodingQuality {
    Low,
    #[default]
    Medium,
    High,
}

impl EncodingQuality {
    /// Average bit rate, in bits per second, for a picture size and frame rate.
    fn bit_rate(self, media_type: &MediaType) -> u32 {
        // Bits per pixel, in thousandths.
        let bits_per_pixel_milli: u64 = match self {
            Self::Low => 50,
            Self::Medium => 100,
            Self::High => 200,
        };
        let pixels = u64::from(media_type.width) * u64::from(media_type.height);
        let pixel_rate =
            pixels * u64::from(media_type.frame_rate_numerator) / u64::from(media_type.frame_rate_denominator.max(1));
        u32::try_from(pixel_rate * bits_per_pixel_milli / 1000)
            .unwrap_or(u32::MAX)
            .max(100_000)
    }
}

/// Capture options for [`MediaFoundationCamera`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraOptions {
    /// Encode raw camera frames to H.264 with the Media Foundation software encoder.
    ///
    /// Only used when the camera has no compressed media type. When disabled, or when the
    /// encoder is unavailable, raw media types are advertised. When the encoder accepts none of
    /// the camera's media types, only raw types of at most 1920x1080 are advertised.
    pub encode_h264: bool,
    pub quality: EncodingQuality,
}

impl Default for CameraOptions {
    fn default() -> Self {
        Self {
            encode_h264: true,
            quality: EncodingQuality::default(),
        }
    }
}

/// Error raised while enumerating or watching cameras.
#[derive(Debug)]
pub struct CameraError {
    context: &'static str,
    source: windows::core::Error,
}

impl CameraError {
    fn new(context: &'static str, source: windows::core::Error) -> Self {
        Self { context, source }
    }
}

impl fmt::Display for CameraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.context)
    }
}

impl core::error::Error for CameraError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// A video capture device visible to Media Foundation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDeviceInfo {
    /// Device interface path, stable across reboots for a given camera and port.
    pub symbolic_link: String,
    pub friendly_name: String,
}

impl CameraDeviceInfo {
    /// Key under which symbolic links compare equal (they are case-insensitive).
    pub fn identity_key(symbolic_link: &str) -> String {
        symbolic_link.to_lowercase()
    }
}

/// Lists the video capture devices currently attached.
pub fn enumerate_cameras() -> Result<Vec<CameraDeviceInfo>, CameraError> {
    // Media Foundation needs COM on the calling thread. A short-lived worker keeps the caller's
    // apartment (possibly a single-threaded UI apartment) untouched.
    thread::Builder::new()
        .name("rdpecam-enumerate".to_owned())
        .spawn(|| {
            let _media_foundation = MediaFoundationThread::initialize()
                .map_err(|error| CameraError::new("failed to initialize Media Foundation", error))?;
            enumerate_on_current_thread().map_err(|error| CameraError::new("failed to enumerate cameras", error))
        })
        .map_err(|error| {
            CameraError::new(
                "failed to spawn the camera enumeration thread",
                windows::core::Error::from(error),
            )
        })?
        .join()
        .unwrap_or_else(|_| {
            Err(CameraError::new(
                "camera enumeration thread panicked",
                windows::core::Error::empty(),
            ))
        })
}

/// Device arrival or removal reported by [`CameraWatcher`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraEvent {
    Added(CameraDeviceInfo),
    Removed { symbolic_link: String },
}

struct WatcherContext {
    wake: mpsc::Sender<()>,
}

/// Reports camera arrival and removal.
///
/// Configuration manager notifications for the camera interface class trigger a Media Foundation
/// re-enumeration on a dedicated thread, and the difference with the previous enumeration is
/// reported to the callback. The callback runs on that thread.
pub struct CameraWatcher {
    registration: HCMNOTIFICATION,
    context: *mut WatcherContext,
    thread: Option<JoinHandle<()>>,
}

// SAFETY: the registration handle may be unregistered from any thread, and the context is only
// shared with the notification callback, which accesses it through a `Mutex`.
unsafe impl Send for CameraWatcher {}

impl CameraWatcher {
    /// Starts watching. `initial` is the device list the caller already knows about.
    pub fn new<F>(initial: Vec<CameraDeviceInfo>, callback: F) -> Result<Self, CameraError>
    where
        F: Fn(CameraEvent) + Send + 'static,
    {
        let (wake_sender, wake_receiver) = mpsc::channel::<()>();
        let thread = thread::Builder::new()
            .name("rdpecam-watcher".to_owned())
            .spawn(move || {
                let mut known: BTreeMap<String, CameraDeviceInfo> = initial
                    .into_iter()
                    .map(|device| (CameraDeviceInfo::identity_key(&device.symbolic_link), device))
                    .collect();
                while wake_receiver.recv().is_ok() {
                    // Coalesce bursts of notifications into one enumeration.
                    while wake_receiver.try_recv().is_ok() {}
                    let devices = match enumerate_cameras() {
                        Ok(devices) => devices,
                        Err(error) => {
                            warn!(%error, "Failed to enumerate cameras after a device change");
                            continue;
                        }
                    };
                    let current: BTreeMap<String, CameraDeviceInfo> = devices
                        .into_iter()
                        .map(|device| (CameraDeviceInfo::identity_key(&device.symbolic_link), device))
                        .collect();
                    for (key, device) in &known {
                        if !current.contains_key(key) {
                            callback(CameraEvent::Removed {
                                symbolic_link: device.symbolic_link.clone(),
                            });
                        }
                    }
                    for (key, device) in &current {
                        if !known.contains_key(key) {
                            callback(CameraEvent::Added(device.clone()));
                        }
                    }
                    known = current;
                }
            })
            .map_err(|error| {
                CameraError::new(
                    "failed to spawn the camera watcher thread",
                    windows::core::Error::from(error),
                )
            })?;

        let filter_size = u32::try_from(size_of::<CM_NOTIFY_FILTER>()).map_err(|_| {
            CameraError::new(
                "notification filter size overflows",
                windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL),
            )
        })?;
        let rescan = wake_sender.clone();
        let context = Box::into_raw(Box::new(WatcherContext { wake: wake_sender }));
        let filter = CM_NOTIFY_FILTER {
            cbSize: filter_size,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
            u: CM_NOTIFY_FILTER_0 {
                DeviceInterface: CM_NOTIFY_FILTER_0_0 {
                    ClassGuid: KSCATEGORY_VIDEO_CAMERA,
                },
            },
            ..Default::default()
        };
        let mut registration = HCMNOTIFICATION::default();
        // SAFETY: `filter` is fully initialized, `context` stays valid until the registration is
        // removed in `Drop`, and the callback matches `PCM_NOTIFY_CALLBACK`.
        let status = unsafe {
            CM_Register_Notification(
                &filter,
                Some(context.cast_const().cast::<c_void>()),
                Some(watcher_notification),
                &mut registration,
            )
        };
        if status != CR_SUCCESS {
            // SAFETY: the registration failed, so the callback never received the pointer.
            drop(unsafe { Box::from_raw(context) });
            return Err(CameraError::new(
                "failed to register for camera notifications",
                windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL),
            ));
        }
        // Notifications only cover changes from now on: catch those since `initial` was enumerated.
        let _ = rescan.send(());
        Ok(Self {
            registration,
            context,
            thread: Some(thread),
        })
    }
}

impl Drop for CameraWatcher {
    fn drop(&mut self) {
        // SAFETY: the handle comes from a successful registration. Unregistering waits for
        // in-flight callbacks, so the context is no longer referenced afterwards.
        let _status = unsafe { CM_Unregister_Notification(self.registration) };
        // SAFETY: the pointer was created by `Box::into_raw` and is no longer used by the callback.
        drop(unsafe { Box::from_raw(self.context) });
        // Dropping the sender above ends the watcher thread.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

unsafe extern "system" fn watcher_notification(
    _registration: HCMNOTIFICATION,
    context: *const c_void,
    action: CM_NOTIFY_ACTION,
    _event_data: *const CM_NOTIFY_EVENT_DATA,
    _event_data_size: u32,
) -> u32 {
    trace!(?action, "Camera interface notification");
    // SAFETY: `context` is the `WatcherContext` registered in `CameraWatcher::new`, which outlives
    // the registration.
    let context = unsafe { &*context.cast::<WatcherContext>() };
    let _ = context.wake.send(());
    0
}

/// Camera backend reading one Media Foundation video capture device.
///
/// The device is opened on activation and released on deactivation or shutdown.
pub struct MediaFoundationCamera {
    symbolic_link: String,
    options: CameraOptions,
    worker: Option<Worker>,
    pending: Arc<Mutex<PendingSamples>>,
}

impl MediaFoundationCamera {
    /// Creates a backend for the camera with the given symbolic link.
    pub fn new(symbolic_link: impl Into<String>) -> Self {
        Self::with_options(symbolic_link, CameraOptions::default())
    }

    pub fn with_options(symbolic_link: impl Into<String>, options: CameraOptions) -> Self {
        Self {
            symbolic_link: symbolic_link.into(),
            options,
            worker: None,
            pending: Arc::new(Mutex::new(PendingSamples::default())),
        }
    }

    pub fn symbolic_link(&self) -> &str {
        &self.symbolic_link
    }

    fn request<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Result<T, ErrorCode> {
        let worker = self.worker.as_ref().ok_or(ErrorCode::NotInitialized)?;
        let (reply, response) = mpsc::sync_channel(1);
        worker
            .commands
            .send(command(reply))
            .map_err(|_| ErrorCode::Unexpected)?;
        response.recv_timeout(COMMAND_TIMEOUT).map_err(|error| {
            warn!(%error, "Camera worker did not answer");
            ErrorCode::Unexpected
        })?
    }

    fn stop_worker(&mut self) {
        let Some(mut worker) = self.worker.take() else {
            return;
        };
        let (reply, response) = mpsc::sync_channel(1);
        if worker.commands.send(Command::Shutdown(reply)).is_ok() && response.recv_timeout(COMMAND_TIMEOUT).is_ok() {
            if let Some(thread) = worker.thread.take() {
                let _ = thread.join();
            }
        } else {
            warn!("Camera worker did not stop in time; detaching it");
        }
    }
}

impl Drop for MediaFoundationCamera {
    fn drop(&mut self) {
        self.stop_worker();
    }
}

impl CameraBackend for MediaFoundationCamera {
    fn activate(&mut self) -> Result<(), ErrorCode> {
        if self.worker.is_some() {
            return Ok(());
        }
        let (commands, command_receiver) = mpsc::channel();
        let (ready, ready_receiver) = mpsc::sync_channel(1);
        let symbolic_link = self.symbolic_link.clone();
        let options = self.options;
        let pending = Arc::clone(&self.pending);
        let thread = thread::Builder::new()
            .name("rdpecam-capture".to_owned())
            .spawn(move || worker_main(&symbolic_link, options, &command_receiver, &ready, &pending))
            .map_err(|_| ErrorCode::OutOfMemory)?;
        let mut worker = Worker {
            commands,
            thread: Some(thread),
        };
        match ready_receiver.recv_timeout(COMMAND_TIMEOUT) {
            Ok(Ok(())) => {
                info!(symbolic_link = %self.symbolic_link, "Camera activated");
                self.worker = Some(worker);
                Ok(())
            }
            Ok(Err(error)) => {
                if let Some(thread) = worker.thread.take() {
                    let _ = thread.join();
                }
                Err(error)
            }
            Err(_) => {
                warn!(symbolic_link = %self.symbolic_link, "Camera did not open in time");
                Err(ErrorCode::Unexpected)
            }
        }
    }

    fn deactivate(&mut self) -> Result<(), ErrorCode> {
        lock(&self.pending).reset();
        self.stop_worker();
        debug!(symbolic_link = %self.symbolic_link, "Camera deactivated");
        Ok(())
    }

    fn streams(&mut self) -> Result<Vec<StreamDescription>, ErrorCode> {
        Ok(vec![StreamDescription::color(true, true)])
    }

    fn media_types(&mut self, stream_index: u8) -> Result<Vec<MediaType>, ErrorCode> {
        if stream_index != STREAM_INDEX {
            return Err(ErrorCode::InvalidStreamNumber);
        }
        self.request(Command::MediaTypes)
    }

    fn current_media_type(&mut self, stream_index: u8) -> Result<MediaType, ErrorCode> {
        if stream_index != STREAM_INDEX {
            return Err(ErrorCode::InvalidStreamNumber);
        }
        self.request(Command::CurrentMediaType)
    }

    fn start_streams(&mut self, streams: &[StartStreamInfo], sink: SampleSink) -> Result<(), ErrorCode> {
        let [stream] = streams else {
            return Err(ErrorCode::InvalidRequest);
        };
        if stream.stream_index != STREAM_INDEX {
            return Err(ErrorCode::InvalidStreamNumber);
        }
        // The worker installs the sink before capturing frames of the new format.
        let media_type = stream.media_type;
        self.request(|reply| Command::Start(media_type, sink, reply))
    }

    fn stop_streams(&mut self) -> Result<(), ErrorCode> {
        self.request(Command::Stop)?;
        lock(&self.pending).reset();
        Ok(())
    }

    fn sample_requested(&mut self, _stream_index: u8) {
        lock(&self.pending).flush();
    }

    fn properties(&mut self) -> Result<Vec<PropertyDescription>, ErrorCode> {
        self.request(Command::Properties)
    }

    fn property_value(
        &mut self,
        property_set: PropertySet,
        property_id: PropertyId,
    ) -> Result<PropertyValue, ErrorCode> {
        self.request(|reply| Command::GetProperty(property_set, property_id, reply))
    }

    fn set_property_value(
        &mut self,
        property_set: PropertySet,
        property_id: PropertyId,
        value: PropertyValue,
    ) -> Result<(), ErrorCode> {
        self.request(|reply| Command::SetProperty(property_set, property_id, value, reply))
    }

    fn shutdown(&mut self) {
        lock(&self.pending).reset();
        self.stop_worker();
    }
}

type Reply<T> = mpsc::SyncSender<Result<T, ErrorCode>>;

enum Command {
    MediaTypes(Reply<Vec<MediaType>>),
    CurrentMediaType(Reply<MediaType>),
    Start(MediaType, SampleSink, Reply<()>),
    Stop(Reply<()>),
    Properties(Reply<Vec<PropertyDescription>>),
    GetProperty(PropertySet, PropertyId, Reply<PropertyValue>),
    SetProperty(PropertySet, PropertyId, PropertyValue, Reply<()>),
    Shutdown(Reply<()>),
}

struct Worker {
    commands: mpsc::Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

struct Frame {
    data: Vec<u8>,
    key_frame: bool,
}

/// Captured frames waiting for the server's Sample Requests.
#[derive(Default)]
struct PendingSamples {
    sink: Option<SampleSink>,
    frames: VecDeque<Frame>,
    /// H.264 pictures depend on each other and must all be delivered.
    keep_all: bool,
    waiting_for_key_frame: bool,
    /// The device stopped producing frames: requests fail until streaming restarts.
    failed: bool,
}

impl PendingSamples {
    fn start(&mut self, sink: SampleSink, media_type: MediaType) {
        self.sink = Some(sink);
        self.frames.clear();
        self.keep_all = media_type.format == MediaFormat::H264;
        self.waiting_for_key_frame = self.keep_all;
        self.failed = false;
    }

    fn reset(&mut self) {
        self.sink = None;
        self.frames.clear();
        self.failed = false;
    }

    fn push(&mut self, frame: Frame) {
        if self.sink.is_none() {
            return;
        }
        if !self.keep_all {
            self.frames.clear();
            self.frames.push_back(frame);
            return;
        }
        if self.waiting_for_key_frame {
            if !frame.key_frame {
                return;
            }
            self.waiting_for_key_frame = false;
        }
        if self.frames.len() == MAX_QUEUED_H264_FRAMES {
            debug!("Dropping queued H.264 camera pictures until the next key frame");
            self.frames.clear();
            if !frame.key_frame {
                self.waiting_for_key_frame = true;
                return;
            }
        }
        self.frames.push_back(frame);
    }

    /// Answers outstanding Sample Requests with queued frames, or with errors after a failure.
    fn flush(&mut self) {
        let Some(sink) = &self.sink else {
            self.frames.clear();
            return;
        };
        if self.failed {
            while sink.pending_requests(STREAM_INDEX) != 0 {
                if sink.send_error(STREAM_INDEX, ErrorCode::Unexpected) != SampleDelivery::Sent {
                    break;
                }
            }
            self.frames.clear();
            return;
        }
        while sink.pending_requests(STREAM_INDEX) != 0 {
            let Some(frame) = self.frames.pop_front() else {
                break;
            };
            // H.264 pictures depend on each other, so one the session cannot take yet is kept.
            let retry = self.keep_all.then(|| Frame {
                data: frame.data.clone(),
                key_frame: frame.key_frame,
            });
            match sink.send_sample(STREAM_INDEX, frame.data) {
                SampleDelivery::Sent | SampleDelivery::Rejected | SampleDelivery::NotRequested => {}
                SampleDelivery::Failed => {
                    // The session input queue is full: the request stays outstanding and the
                    // picture is sent again with the next captured frame.
                    if let Some(frame) = retry {
                        self.frames.push_front(frame);
                    }
                    break;
                }
                SampleDelivery::Stopped => {
                    self.frames.clear();
                    break;
                }
            }
        }
    }

    /// Drops queued frames after the reader lost some; H.264 resumes at the next key frame.
    fn resync(&mut self) {
        self.frames.clear();
        self.waiting_for_key_frame = self.keep_all;
    }

    /// Fails outstanding and later requests after the device stopped producing frames.
    fn fail(&mut self) {
        self.failed = true;
        self.flush();
    }
}

fn lock(pending: &Mutex<PendingSamples>) -> MutexGuard<'_, PendingSamples> {
    pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Initializes COM for the multithreaded apartment and starts Media Foundation on this thread.
struct MediaFoundationThread;

impl MediaFoundationThread {
    fn initialize() -> windows::core::Result<Self> {
        // SAFETY: balanced by `CoUninitialize` in `Drop` or below.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
        // SAFETY: COM is initialized on this thread.
        if let Err(error) = unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) } {
            // SAFETY: balances the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
            return Err(error);
        }
        Ok(Self)
    }
}

impl Drop for MediaFoundationThread {
    fn drop(&mut self) {
        // SAFETY: balances the successful `MFStartup` in `initialize`.
        let _ = unsafe { MFShutdown() };
        // SAFETY: balances the successful `CoInitializeEx` in `initialize`.
        unsafe { CoUninitialize() };
    }
}

/// One advertised media type and the native media type that produces it.
struct NativeMediaType {
    description: MediaType,
    native: IMFMediaType,
    /// The native frames are encoded to H.264 before being sent.
    encode: bool,
}

/// A property backed by `IAMCameraControl` or `IAMVideoProcAmp`.
#[derive(Clone, Copy)]
struct PropertyBinding {
    property_set: PropertySet,
    property_id: PropertyId,
    native: i32,
}

const PROPERTY_BINDINGS: [PropertyBinding; 11] = [
    PropertyBinding {
        property_set: PropertySet::CAMERA_CONTROL,
        property_id: PropertyId::CAMERA_CONTROL_EXPOSURE,
        native: windows::Win32::Media::DirectShow::CameraControl_Exposure.0,
    },
    PropertyBinding {
        property_set: PropertySet::CAMERA_CONTROL,
        property_id: PropertyId::CAMERA_CONTROL_FOCUS,
        native: windows::Win32::Media::DirectShow::CameraControl_Focus.0,
    },
    PropertyBinding {
        property_set: PropertySet::CAMERA_CONTROL,
        property_id: PropertyId::CAMERA_CONTROL_PAN,
        native: windows::Win32::Media::DirectShow::CameraControl_Pan.0,
    },
    PropertyBinding {
        property_set: PropertySet::CAMERA_CONTROL,
        property_id: PropertyId::CAMERA_CONTROL_ROLL,
        native: windows::Win32::Media::DirectShow::CameraControl_Roll.0,
    },
    PropertyBinding {
        property_set: PropertySet::CAMERA_CONTROL,
        property_id: PropertyId::CAMERA_CONTROL_TILT,
        native: windows::Win32::Media::DirectShow::CameraControl_Tilt.0,
    },
    PropertyBinding {
        property_set: PropertySet::CAMERA_CONTROL,
        property_id: PropertyId::CAMERA_CONTROL_ZOOM,
        native: windows::Win32::Media::DirectShow::CameraControl_Zoom.0,
    },
    PropertyBinding {
        property_set: PropertySet::VIDEO_PROC_AMP,
        property_id: PropertyId::VIDEO_PROC_AMP_BACKLIGHT_COMPENSATION,
        native: windows::Win32::Media::DirectShow::VideoProcAmp_BacklightCompensation.0,
    },
    PropertyBinding {
        property_set: PropertySet::VIDEO_PROC_AMP,
        property_id: PropertyId::VIDEO_PROC_AMP_BRIGHTNESS,
        native: windows::Win32::Media::DirectShow::VideoProcAmp_Brightness.0,
    },
    PropertyBinding {
        property_set: PropertySet::VIDEO_PROC_AMP,
        property_id: PropertyId::VIDEO_PROC_AMP_CONTRAST,
        native: windows::Win32::Media::DirectShow::VideoProcAmp_Contrast.0,
    },
    PropertyBinding {
        property_set: PropertySet::VIDEO_PROC_AMP,
        property_id: PropertyId::VIDEO_PROC_AMP_HUE,
        native: windows::Win32::Media::DirectShow::VideoProcAmp_Hue.0,
    },
    PropertyBinding {
        property_set: PropertySet::VIDEO_PROC_AMP,
        property_id: PropertyId::VIDEO_PROC_AMP_WHITE_BALANCE,
        native: windows::Win32::Media::DirectShow::VideoProcAmp_WhiteBalance.0,
    },
];

/// `CameraControl_Flags_Auto` and `VideoProcAmp_Flags_Auto` share this value.
const NATIVE_FLAGS_AUTO: i32 = 0x1;
/// `CameraControl_Flags_Manual` and `VideoProcAmp_Flags_Manual` share this value.
const NATIVE_FLAGS_MANUAL: i32 = 0x2;

/// Media Foundation objects for one opened device. Lives on the worker thread only.
struct OpenDevice {
    source: IMFMediaSource,
    reader: IMFSourceReader,
    media_types: Vec<NativeMediaType>,
    camera_control: Option<IAMCameraControl>,
    video_proc_amp: Option<IAMVideoProcAmp>,
    streaming: Option<Streaming>,
    quality: EncodingQuality,
}

struct Streaming {
    media_type: MediaType,
    /// H.264 parameter sets kept out of band by the camera or encoder, in Annex B form.
    sequence_header: Option<Vec<u8>>,
    encoder: Option<IMFTransform>,
}

impl Drop for OpenDevice {
    fn drop(&mut self) {
        // SAFETY: the source is a valid media source owned by this device.
        if let Err(error) = unsafe { self.source.Shutdown() } {
            debug!(%error, "Failed to shut down the camera media source");
        }
    }
}

fn worker_main(
    symbolic_link: &str,
    options: CameraOptions,
    commands: &mpsc::Receiver<Command>,
    ready: &mpsc::SyncSender<Result<(), ErrorCode>>,
    pending: &Mutex<PendingSamples>,
) {
    let _media_foundation = match MediaFoundationThread::initialize() {
        Ok(media_foundation) => media_foundation,
        Err(error) => {
            warn!(%error, "Failed to initialize Media Foundation for the camera");
            let _ = ready.send(Err(ErrorCode::Unexpected));
            return;
        }
    };
    let mut device = match OpenDevice::open(symbolic_link, options) {
        Ok(device) => device,
        Err(error) => {
            warn!(%error, %symbolic_link, "Failed to open the camera");
            let _ = ready.send(Err(ErrorCode::NotInitialized));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }

    loop {
        let command = if device.streaming.is_some() {
            match commands.try_recv() {
                Ok(command) => Some(command),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        } else {
            match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            }
        };
        match command {
            Some(command) => {
                if let Some(reply) = device.handle(command, pending) {
                    lock(pending).reset();
                    drop(device);
                    let _ = reply.send(Ok(()));
                    return;
                }
            }
            None => device.capture_one(pending),
        }
    }
}

impl OpenDevice {
    fn open(symbolic_link: &str, options: CameraOptions) -> windows::core::Result<Self> {
        let attributes = create_attributes(2)?;
        // SAFETY: valid attribute store and GUID pointers.
        unsafe {
            attributes.SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
        }?;
        let link = windows::core::HSTRING::from(symbolic_link);
        // SAFETY: valid attribute store; the string is copied.
        unsafe { attributes.SetString(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, &link) }?;
        // SAFETY: the attributes identify a video capture device.
        let source = unsafe { MFCreateDeviceSource(&attributes) }?;

        let reader_attributes = create_attributes(1)?;
        // SAFETY: valid attribute store. Native media types are wanted, without converters.
        unsafe { reader_attributes.SetUINT32(&MF_READWRITE_DISABLE_CONVERTERS, 1) }?;
        // SAFETY: `source` is a valid media source.
        let reader = match unsafe { MFCreateSourceReaderFromMediaSource(&source, &reader_attributes) } {
            Ok(reader) => reader,
            Err(error) => {
                // SAFETY: valid media source that is no longer used.
                let _ = unsafe { source.Shutdown() };
                return Err(error);
            }
        };

        let media_types = native_media_types(&reader, options)?;
        let camera_control = source.cast::<IAMCameraControl>().ok();
        let video_proc_amp = source.cast::<IAMVideoProcAmp>().ok();
        debug!(
            %symbolic_link,
            media_types = media_types.len(),
            camera_control = camera_control.is_some(),
            video_proc_amp = video_proc_amp.is_some(),
            "Opened camera"
        );
        Ok(Self {
            source,
            reader,
            media_types,
            camera_control,
            video_proc_amp,
            streaming: None,
            quality: options.quality,
        })
    }

    /// Runs one command. A shutdown is returned, for the worker to release the device first.
    fn handle(&mut self, command: Command, pending: &Mutex<PendingSamples>) -> Option<Reply<()>> {
        match command {
            Command::MediaTypes(reply) => {
                let media_types = self
                    .media_types
                    .iter()
                    .map(|media_type| media_type.description)
                    .collect();
                let _ = reply.send(Ok(media_types));
            }
            Command::CurrentMediaType(reply) => {
                let current = self
                    .streaming
                    .as_ref()
                    .map(|streaming| streaming.media_type)
                    .or_else(|| self.media_types.first().map(|media_type| media_type.description))
                    .ok_or(ErrorCode::InvalidMediaType);
                let _ = reply.send(current);
            }
            Command::Start(media_type, sink, reply) => {
                let _ = reply.send(self.start(media_type, sink, pending));
            }
            Command::Stop(reply) => {
                self.streaming = None;
                self.flush_reader();
                let _ = reply.send(Ok(()));
            }
            Command::Properties(reply) => {
                let _ = reply.send(Ok(self.properties()));
            }
            Command::GetProperty(property_set, property_id, reply) => {
                let _ = reply.send(self.property_value(property_set, property_id));
            }
            Command::SetProperty(property_set, property_id, value, reply) => {
                let _ = reply.send(self.set_property_value(property_set, property_id, value));
            }
            Command::Shutdown(reply) => return Some(reply),
        }
        None
    }

    /// Replaces the stream and its sink. On failure, the previous stream keeps going.
    fn start(
        &mut self,
        media_type: MediaType,
        sink: SampleSink,
        pending: &Mutex<PendingSamples>,
    ) -> Result<(), ErrorCode> {
        let native = self
            .media_types
            .iter()
            .find(|candidate| candidate.description == media_type)
            .ok_or(ErrorCode::InvalidMediaType)?;
        // Everything that can fail without touching the reader comes first.
        let (encoder, sequence_header) = if native.encode {
            let encoder = create_encoder(&native.native, &media_type, self.quality).map_err(|error| {
                warn!(%error, "Failed to configure the H.264 encoder for the camera");
                ErrorCode::InvalidMediaType
            })?;
            // SAFETY: valid, configured transform.
            let output_type = unsafe { encoder.GetOutputCurrentType(0) }.ok();
            let sequence_header = output_type.and_then(|output_type| blob(&output_type, &MF_MT_MPEG_SEQUENCE_HEADER));
            (Some(encoder), sequence_header)
        } else if media_type.format == MediaFormat::H264 {
            (None, blob(&native.native, &MF_MT_MPEG_SEQUENCE_HEADER))
        } else {
            (None, None)
        };
        // Buffered samples of the current type would otherwise hold up the change.
        self.flush_reader();
        if let Err(error) = select_media_type(&self.reader, &native.native) {
            let previous = self.streaming.as_ref().and_then(|streaming| {
                self.media_types
                    .iter()
                    .find(|candidate| candidate.description == streaming.media_type)
            });
            if let Some(previous) = previous {
                self.flush_reader();
                if select_media_type(&self.reader, &previous.native).is_ok() {
                    // The flush lost frames the queued ones may depend on.
                    lock(pending).resync();
                } else {
                    warn!("Failed to restore the previous camera media type");
                    self.streaming = None;
                    lock(pending).fail();
                }
            }
            return Err(error);
        }
        info!(?media_type, encoded = encoder.is_some(), "Camera streaming started");
        lock(pending).start(sink, media_type);
        self.streaming = Some(Streaming {
            media_type,
            sequence_header,
            encoder,
        });
        Ok(())
    }

    /// Drops the samples the reader buffered for the current stream.
    fn flush_reader(&self) {
        // SAFETY: valid source reader in synchronous mode.
        if let Err(error) = unsafe { self.reader.Flush(first_video_stream()) } {
            debug!(%error, "Failed to flush the camera source reader");
        }
    }

    /// Reads one sample and queues it for the server.
    fn capture_one(&mut self, pending: &Mutex<PendingSamples>) {
        let Some(streaming) = &self.streaming else {
            return;
        };
        let mut flags = 0u32;
        let mut sample: Option<IMFSample> = None;
        // SAFETY: valid source reader in synchronous mode; out pointers are valid locals.
        let result = unsafe {
            self.reader
                .ReadSample(first_video_stream(), 0, None, Some(&mut flags), None, Some(&mut sample))
        };
        let failed = flags & flag_bits(MF_SOURCE_READERF_ERROR.0) != 0
            || flags & flag_bits(MF_SOURCE_READERF_ENDOFSTREAM.0) != 0;
        if let Err(error) = result.as_ref() {
            warn!(%error, "Camera capture failed");
        }
        if result.is_err() || failed {
            warn!("Camera stopped producing frames");
            self.streaming = None;
            lock(pending).fail();
            return;
        }
        let Some(sample) = sample else {
            // Stream tick or gap.
            return;
        };
        let frames = match &streaming.encoder {
            Some(encoder) => encode_sample(encoder, &sample, streaming),
            None => sample_frame(&sample, streaming).map(|frame| vec![frame]),
        };
        let frames = match frames {
            Ok(frames) => frames,
            Err(error) => {
                debug!(%error, "Dropping an unreadable camera sample");
                return;
            }
        };
        let mut pending = lock(pending);
        for frame in frames {
            pending.push(frame);
        }
        pending.flush();
    }

    fn native_range(&self, binding: PropertyBinding) -> windows::core::Result<[i32; 5]> {
        let mut range = [0i32; 5];
        let [min, max, step, default, flags] = &mut range;
        if binding.property_set == PropertySet::CAMERA_CONTROL {
            let control = self.camera_control.as_ref().ok_or_else(windows::core::Error::empty)?;
            // SAFETY: valid interface; out pointers are valid locals.
            unsafe { control.GetRange(binding.native, min, max, step, default, flags) }?;
        } else {
            let control = self.video_proc_amp.as_ref().ok_or_else(windows::core::Error::empty)?;
            // SAFETY: valid interface; out pointers are valid locals.
            unsafe { control.GetRange(binding.native, min, max, step, default, flags) }?;
        }
        Ok(range)
    }

    fn native_get(&self, binding: PropertyBinding) -> windows::core::Result<(i32, i32)> {
        let mut value = 0;
        let mut flags = 0;
        if binding.property_set == PropertySet::CAMERA_CONTROL {
            let control = self.camera_control.as_ref().ok_or_else(windows::core::Error::empty)?;
            // SAFETY: valid interface; out pointers are valid locals.
            unsafe { control.Get(binding.native, &mut value, &mut flags) }?;
        } else {
            let control = self.video_proc_amp.as_ref().ok_or_else(windows::core::Error::empty)?;
            // SAFETY: valid interface; out pointers are valid locals.
            unsafe { control.Get(binding.native, &mut value, &mut flags) }?;
        }
        Ok((value, flags))
    }

    fn native_set(&self, binding: PropertyBinding, value: i32, flags: i32) -> windows::core::Result<()> {
        if binding.property_set == PropertySet::CAMERA_CONTROL {
            let control = self.camera_control.as_ref().ok_or_else(windows::core::Error::empty)?;
            // SAFETY: valid interface.
            unsafe { control.Set(binding.native, value, flags) }
        } else {
            let control = self.video_proc_amp.as_ref().ok_or_else(windows::core::Error::empty)?;
            // SAFETY: valid interface.
            unsafe { control.Set(binding.native, value, flags) }
        }
    }

    fn properties(&self) -> Vec<PropertyDescription> {
        PROPERTY_BINDINGS
            .iter()
            .filter_map(|binding| {
                let [min_value, max_value, step, default_value, flags] = self.native_range(*binding).ok()?;
                let capabilities = property_mode(flags);
                (capabilities.0 != 0 && min_value <= max_value).then_some(PropertyDescription {
                    property_set: binding.property_set,
                    property_id: binding.property_id,
                    capabilities,
                    min_value,
                    max_value,
                    step,
                    default_value,
                })
            })
            .collect()
    }

    fn property_value(&self, property_set: PropertySet, property_id: PropertyId) -> Result<PropertyValue, ErrorCode> {
        let binding = property_binding(property_set, property_id)?;
        let (value, flags) = self.native_get(binding).map_err(|error| {
            debug!(%error, ?property_set, ?property_id, "Failed to read a camera property");
            ErrorCode::ItemNotFound
        })?;
        let mode = if flags & NATIVE_FLAGS_AUTO != 0 {
            PropertyMode::AUTO
        } else {
            PropertyMode::MANUAL
        };
        Ok(PropertyValue { mode, value })
    }

    fn set_property_value(
        &self,
        property_set: PropertySet,
        property_id: PropertyId,
        value: PropertyValue,
    ) -> Result<(), ErrorCode> {
        let binding = property_binding(property_set, property_id)?;
        let (native_value, flags) = if value.mode == PropertyMode::AUTO {
            // The value is ignored in automatic mode; keep the current one.
            let (current, _) = self.native_get(binding).map_err(|_| ErrorCode::ItemNotFound)?;
            (current, NATIVE_FLAGS_AUTO)
        } else {
            (value.value, NATIVE_FLAGS_MANUAL)
        };
        self.native_set(binding, native_value, flags).map_err(|error| {
            debug!(%error, ?property_set, ?property_id, "Failed to set a camera property");
            ErrorCode::OperationNotSupported
        })
    }
}

fn property_binding(property_set: PropertySet, property_id: PropertyId) -> Result<PropertyBinding, ErrorCode> {
    if !property_set.is_known() {
        return Err(ErrorCode::SetNotFound);
    }
    PROPERTY_BINDINGS
        .iter()
        .find(|binding| binding.property_set == property_set && binding.property_id == property_id)
        .copied()
        .ok_or(ErrorCode::ItemNotFound)
}

/// Maps DirectShow capability flags to RDPECAM modes (the two use swapped bit values).
fn property_mode(native_flags: i32) -> PropertyMode {
    let mut mode = 0;
    if native_flags & NATIVE_FLAGS_MANUAL != 0 {
        mode |= PropertyMode::MANUAL.0;
    }
    if native_flags & NATIVE_FLAGS_AUTO != 0 {
        mode |= PropertyMode::AUTO.0;
    }
    PropertyMode(mode)
}

/// Lists the reader's native media types as RDPECAM media types.
///
/// When the camera produces compressed pictures, only compressed media types are kept so that the
/// server does not pick a raw format that would saturate the connection. Otherwise, when encoding
/// is enabled and the H.264 encoder is available, the raw media types it accepts are advertised as
/// H.264 instead.
fn native_media_types(reader: &IMFSourceReader, options: CameraOptions) -> windows::core::Result<Vec<NativeMediaType>> {
    let mut media_types: Vec<NativeMediaType> = Vec::new();
    for index in 0u32.. {
        // SAFETY: valid source reader; enumeration stops at `MF_E_NO_MORE_TYPES`.
        let native = match unsafe { reader.GetNativeMediaType(first_video_stream(), index) } {
            Ok(native) => native,
            Err(error) if error.code() == MF_E_NO_MORE_TYPES => break,
            Err(error) => return Err(error),
        };
        let Some(description) = describe_media_type(&native) else {
            continue;
        };
        if media_types.len() == ironrdp_rdpecam::pdu::MAX_MEDIA_TYPES {
            break;
        }
        if media_types.iter().all(|existing| existing.description != description) {
            media_types.push(NativeMediaType {
                description,
                native,
                encode: false,
            });
        }
    }
    if media_types
        .iter()
        .any(|media_type| media_type.description.is_compressed())
    {
        media_types.retain(|media_type| media_type.description.is_compressed());
        return Ok(media_types);
    }
    if options.encode_h264 && encoder_available() {
        let mut encoded: Vec<NativeMediaType> = Vec::new();
        for media_type in media_types
            .iter()
            .filter(|media_type| encodable(&media_type.description))
        {
            let description = MediaType {
                format: MediaFormat::H264,
                flags: MediaType::DECODING_REQUIRED,
                ..media_type.description
            };
            if encoded.iter().all(|existing| existing.description != description) {
                encoded.push(NativeMediaType {
                    description,
                    native: media_type.native.clone(),
                    encode: true,
                });
            }
        }
        if !encoded.is_empty() {
            return Ok(encoded);
        }
        // Nothing the encoder accepts, such as RGB only or larger than 1080p: only raw types the
        // encoder could have handled in size are offered, so that frames do not saturate the
        // connection.
        media_types.retain(|media_type| {
            media_type
                .description
                .width
                .checked_mul(media_type.description.height)
                .is_some_and(|pixels| pixels <= MAX_ENCODED_PIXELS)
        });
        if media_types.is_empty() {
            warn!("The camera has no media type small enough to send without encoding");
        }
    }
    Ok(media_types)
}

/// Makes the reader deliver `native` frames from the video stream only.
fn select_media_type(reader: &IMFSourceReader, native: &IMFMediaType) -> Result<(), ErrorCode> {
    // SAFETY: valid source reader; stream selection uses reserved stream constants.
    unsafe { reader.SetStreamSelection(all_streams(), false) }.map_err(|error| {
        warn!(%error, "Failed to deselect camera streams");
        ErrorCode::Unexpected
    })?;
    // SAFETY: as above.
    unsafe { reader.SetStreamSelection(first_video_stream(), true) }.map_err(|error| {
        warn!(%error, "Failed to select the camera video stream");
        ErrorCode::Unexpected
    })?;
    // SAFETY: `native` is one of the reader's native media types.
    unsafe { reader.SetCurrentMediaType(first_video_stream(), None, native) }.map_err(|error| {
        warn!(%error, "Failed to select the camera media type");
        ErrorCode::InvalidMediaType
    })
}

/// Returns whether the software H.264 encoder accepts frames of this media type.
fn encodable(media_type: &MediaType) -> bool {
    matches!(
        media_type.format,
        MediaFormat::Nv12 | MediaFormat::Yuy2 | MediaFormat::I420
    ) && media_type
        .width
        .checked_mul(media_type.height)
        .is_some_and(|pixels| pixels <= MAX_ENCODED_PIXELS)
        && media_type.width.is_multiple_of(2)
        && media_type.height.is_multiple_of(2)
}

fn encoder_available() -> bool {
    // SAFETY: COM is initialized on the worker thread.
    unsafe { CoCreateInstance::<_, IMFTransform>(&CLSID_MSH264EncoderMFT, None, CLSCTX_INPROC_SERVER) }.is_ok()
}

/// Creates the software H.264 encoder for frames of `native`, producing `media_type`.
fn create_encoder(
    native: &IMFMediaType,
    media_type: &MediaType,
    quality: EncodingQuality,
) -> windows::core::Result<IMFTransform> {
    // SAFETY: COM is initialized on the worker thread.
    let encoder: IMFTransform = unsafe { CoCreateInstance(&CLSID_MSH264EncoderMFT, None, CLSCTX_INPROC_SERVER) }?;
    if let Ok(codec) = encoder.cast::<ICodecAPI>() {
        let low_latency = VARIANT::from(true);
        // SAFETY: valid codec interface; the value outlives the call.
        if let Err(error) = unsafe { codec.SetValue(&CODECAPI_AVLowLatencyMode, &low_latency) } {
            debug!(%error, "H.264 encoder rejected low-latency mode");
        }
        let frames_per_second = media_type.frame_rate_numerator / media_type.frame_rate_denominator.max(1);
        let gop_size = VARIANT::from(frames_per_second.saturating_mul(2).max(1));
        // SAFETY: as above.
        if let Err(error) = unsafe { codec.SetValue(&CODECAPI_AVEncMPVGOPSize, &gop_size) } {
            debug!(%error, "H.264 encoder rejected the GOP size");
        }
    }

    let frame_size = join_u64(media_type.width, media_type.height);
    let frame_rate = join_u64(media_type.frame_rate_numerator, media_type.frame_rate_denominator);
    let pixel_aspect_ratio = join_u64(
        media_type.pixel_aspect_ratio_numerator,
        media_type.pixel_aspect_ratio_denominator,
    );
    let progressive = u32::try_from(MFVideoInterlace_Progressive.0).unwrap_or(2);
    let baseline = u32::try_from(eAVEncH264VProfile_Base.0).unwrap_or(66);

    // SAFETY: creates an empty media type.
    let output = unsafe { MFCreateMediaType() }?;
    // SAFETY: valid media type and static keys, for each call below.
    unsafe { output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) }?;
    // SAFETY: as above.
    unsafe { output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264) }?;
    // SAFETY: as above.
    unsafe { output.SetUINT32(&MF_MT_AVG_BITRATE, quality.bit_rate(media_type)) }?;
    // SAFETY: as above.
    unsafe { output.SetUINT64(&MF_MT_FRAME_SIZE, frame_size) }?;
    // SAFETY: as above.
    unsafe { output.SetUINT64(&MF_MT_FRAME_RATE, frame_rate) }?;
    // SAFETY: as above.
    unsafe { output.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pixel_aspect_ratio) }?;
    // SAFETY: as above.
    unsafe { output.SetUINT32(&MF_MT_INTERLACE_MODE, progressive) }?;
    // SAFETY: as above.
    unsafe { output.SetUINT32(&MF_MT_MPEG2_PROFILE, baseline) }?;
    // SAFETY: valid transform; the encoder needs its output type before its input type.
    unsafe { encoder.SetOutputType(0, &output, 0) }?;

    // SAFETY: creates an empty media type.
    let input = unsafe { MFCreateMediaType() }?;
    // SAFETY: valid media type and static keys, for each call below.
    unsafe { input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) }?;
    // SAFETY: as above.
    let subtype = unsafe { native.GetGUID(&MF_MT_SUBTYPE) }?;
    // SAFETY: as above.
    unsafe { input.SetGUID(&MF_MT_SUBTYPE, &subtype) }?;
    // SAFETY: as above.
    unsafe { input.SetUINT64(&MF_MT_FRAME_SIZE, frame_size) }?;
    // SAFETY: as above.
    unsafe { input.SetUINT64(&MF_MT_FRAME_RATE, frame_rate) }?;
    // SAFETY: as above.
    unsafe { input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pixel_aspect_ratio) }?;
    // SAFETY: as above.
    unsafe { input.SetUINT32(&MF_MT_INTERLACE_MODE, progressive) }?;
    // SAFETY: valid transform.
    unsafe { encoder.SetInputType(0, &input, 0) }?;

    // SAFETY: valid, configured transform.
    unsafe { encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0) }?;
    // SAFETY: as above.
    unsafe { encoder.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0) }?;
    Ok(encoder)
}

/// Encodes one captured sample and returns the pictures the encoder produced.
fn encode_sample(
    encoder: &IMFTransform,
    sample: &IMFSample,
    streaming: &Streaming,
) -> windows::core::Result<Vec<Frame>> {
    // SAFETY: valid transform and sample.
    unsafe { encoder.ProcessInput(0, sample, 0) }?;
    let mut frames = Vec::new();
    loop {
        // SAFETY: valid transform.
        let info = unsafe { encoder.GetOutputStreamInfo(0) }?;
        let provides_samples = info.dwFlags & flag_bits(MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0) != 0;
        let output_sample = if provides_samples {
            None
        } else {
            // SAFETY: creates an empty sample.
            let output_sample = unsafe { MFCreateSample() }?;
            // SAFETY: creates a buffer of the size the encoder asks for.
            let buffer = unsafe { MFCreateMemoryBuffer(info.cbSize.max(1)) }?;
            // SAFETY: valid sample and buffer.
            unsafe { output_sample.AddBuffer(&buffer) }?;
            Some(output_sample)
        };
        let mut output = [MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: 0,
            pSample: core::mem::ManuallyDrop::new(output_sample),
            dwStatus: 0,
            pEvents: core::mem::ManuallyDrop::new(None),
        }];
        let mut status = 0u32;
        // SAFETY: valid transform; the output array holds one initialized buffer description.
        let result = unsafe { encoder.ProcessOutput(0, &mut output, &mut status) };
        let [output] = output;
        let picture = core::mem::ManuallyDrop::into_inner(output.pSample);
        drop(core::mem::ManuallyDrop::into_inner(output.pEvents));
        match result {
            Ok(()) => {}
            Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
            Err(error) => return Err(error),
        }
        let Some(picture) = picture else {
            break;
        };
        frames.push(sample_frame(&picture, streaming)?);
    }
    Ok(frames)
}

fn describe_media_type(native: &IMFMediaType) -> Option<MediaType> {
    // SAFETY: valid media type; the key is a static GUID.
    let subtype = unsafe { native.GetGUID(&MF_MT_SUBTYPE) }.ok()?;
    let format = media_format(&subtype)?;
    // SAFETY: as above.
    let (width, height) = split_u64(unsafe { native.GetUINT64(&MF_MT_FRAME_SIZE) }.ok()?);
    // SAFETY: as above.
    let (frame_rate_numerator, frame_rate_denominator) =
        split_u64(unsafe { native.GetUINT64(&MF_MT_FRAME_RATE) }.ok()?);
    // SAFETY: as above.
    let (pixel_aspect_ratio_numerator, pixel_aspect_ratio_denominator) =
        split_u64(unsafe { native.GetUINT64(&MF_MT_PIXEL_ASPECT_RATIO) }.unwrap_or(0x1_0000_0001));
    let mut flags = 0;
    if matches!(format, MediaFormat::H264 | MediaFormat::Mjpg) {
        flags |= MediaType::DECODING_REQUIRED;
    }
    if matches!(format, MediaFormat::Rgb24 | MediaFormat::Rgb32) {
        // Media Foundation RGB images are bottom-up unless the stride says otherwise.
        // SAFETY: as above.
        let stride = unsafe { native.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
            .ok()
            .map(|stride| i32::from_ne_bytes(stride.to_ne_bytes()));
        if stride.is_none_or(|stride| stride < 0) {
            flags |= MediaType::BOTTOM_UP_IMAGE;
        }
    }
    MediaType::new(
        format,
        width,
        height,
        frame_rate_numerator,
        frame_rate_denominator,
        pixel_aspect_ratio_numerator,
        pixel_aspect_ratio_denominator,
        flags,
    )
    .ok()
}

fn media_format(subtype: &GUID) -> Option<MediaFormat> {
    const FORMATS: [(GUID, MediaFormat); 8] = [
        (MFVideoFormat_H264, MediaFormat::H264),
        (MFVideoFormat_MJPG, MediaFormat::Mjpg),
        (MFVideoFormat_YUY2, MediaFormat::Yuy2),
        (MFVideoFormat_NV12, MediaFormat::Nv12),
        (MFVideoFormat_I420, MediaFormat::I420),
        (MFVideoFormat_IYUV, MediaFormat::I420),
        (MFVideoFormat_RGB24, MediaFormat::Rgb24),
        (MFVideoFormat_RGB32, MediaFormat::Rgb32),
    ];
    FORMATS
        .iter()
        .find(|(guid, _)| guid == subtype)
        .map(|(_, format)| *format)
}

/// Copies one captured sample into a frame for the selected media type.
fn sample_frame(sample: &IMFSample, streaming: &Streaming) -> windows::core::Result<Frame> {
    // SAFETY: valid sample.
    let buffer = unsafe { sample.ConvertToContiguousBuffer() }?;
    let media_type = streaming.media_type;
    let mut data = match (media_type.uncompressed_sample_len(), buffer.cast::<IMF2DBuffer>()) {
        // Planar and packed formats may carry row padding; copy them without it.
        (Some(expected), Ok(two_dimensional)) => {
            // SAFETY: valid 2D buffer.
            let length = unsafe { two_dimensional.GetContiguousLength() }?;
            let length = usize::try_from(length).map_err(|_| windows::core::Error::empty())?;
            if length != expected {
                return Err(windows::core::Error::empty());
            }
            let mut data = vec![0; length];
            // SAFETY: valid 2D buffer; the destination has the contiguous length.
            unsafe { two_dimensional.ContiguousCopyTo(&mut data) }?;
            data
        }
        _ => copy_buffer(&buffer)?,
    };
    // SAFETY: valid sample; the key is a static GUID.
    let key_frame =
        unsafe { sample.GetUINT32(&MFSampleExtension_CleanPoint) }.is_ok_and(|clean_point| clean_point != 0);
    if media_type.format == MediaFormat::H264 && key_frame && !contains_sequence_parameter_set(&data) {
        if let Some(header) = &streaming.sequence_header {
            let mut with_header = Vec::with_capacity(header.len() + data.len());
            with_header.extend_from_slice(header);
            with_header.extend_from_slice(&data);
            data = with_header;
        }
    }
    Ok(Frame { data, key_frame })
}

fn copy_buffer(buffer: &IMFMediaBuffer) -> windows::core::Result<Vec<u8>> {
    let mut pointer = core::ptr::null_mut();
    let mut length = 0u32;
    // SAFETY: valid buffer; out pointers are valid locals.
    unsafe { buffer.Lock(&mut pointer, None, Some(&mut length)) }?;
    let data = usize::try_from(length)
        .ok()
        .filter(|_| !pointer.is_null())
        .map(|length| {
            // SAFETY: the locked buffer holds `length` readable bytes until `Unlock`.
            unsafe { core::slice::from_raw_parts(pointer, length) }.to_vec()
        });
    // SAFETY: balances the successful `Lock`.
    unsafe { buffer.Unlock() }?;
    data.ok_or_else(windows::core::Error::empty)
}

/// Returns whether an Annex B H.264 picture carries a sequence parameter set.
fn contains_sequence_parameter_set(data: &[u8]) -> bool {
    data.windows(4)
        .any(|window| window[..3] == [0, 0, 1] && window[3] & 0x1F == 7)
}

fn blob(attributes: &IMFMediaType, key: &GUID) -> Option<Vec<u8>> {
    // SAFETY: valid attribute store.
    let size = unsafe { attributes.GetBlobSize(key) }.ok()?;
    let mut data = vec![0; usize::try_from(size).ok()?];
    // SAFETY: valid attribute store; the destination has the blob size.
    unsafe { attributes.GetBlob(key, &mut data, None) }.ok()?;
    (!data.is_empty()).then_some(data)
}

fn enumerate_on_current_thread() -> windows::core::Result<Vec<CameraDeviceInfo>> {
    let attributes = create_attributes(1)?;
    // SAFETY: valid attribute store and GUID pointers.
    unsafe {
        attributes.SetGUID(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
        )
    }?;
    let mut activates: *mut Option<IMFActivate> = core::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: valid attribute store; out pointers are valid locals.
    unsafe { MFEnumDeviceSources(&attributes, &mut activates, &mut count) }?;
    let activates = if activates.is_null() {
        Vec::new()
    } else {
        let count = usize::try_from(count).map_err(|_| windows::core::Error::empty())?;
        // SAFETY: Media Foundation returned an array of `count` activation objects.
        let slice = unsafe { core::slice::from_raw_parts_mut(activates, count) };
        let owned: Vec<Option<IMFActivate>> = slice.iter_mut().map(Option::take).collect();
        // SAFETY: the array was allocated with `CoTaskMemAlloc`, and its elements were moved out.
        unsafe { CoTaskMemFree(Some(activates.cast_const().cast::<c_void>())) };
        owned
    };

    let mut devices = Vec::new();
    for activate in activates.into_iter().flatten() {
        let attributes: &IMFAttributes = &activate;
        let symbolic_link = match allocated_string(attributes, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK)
        {
            Ok(symbolic_link) => symbolic_link,
            Err(error) => {
                debug!(%error, "Skipping a camera without a symbolic link");
                continue;
            }
        };
        let friendly_name = allocated_string(attributes, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)
            .unwrap_or_else(|_| symbolic_link.clone());
        devices.push(CameraDeviceInfo {
            symbolic_link,
            friendly_name,
        });
    }
    Ok(devices)
}

fn allocated_string(attributes: &IMFAttributes, key: &GUID) -> windows::core::Result<String> {
    let mut value = PWSTR::null();
    let mut length = 0u32;
    // SAFETY: valid attribute store; out pointers are valid locals.
    unsafe { attributes.GetAllocatedString(key, &mut value, &mut length) }?;
    // SAFETY: Media Foundation returned a null-terminated string.
    let text = unsafe { value.to_string() };
    // SAFETY: the string was allocated with `CoTaskMemAlloc`.
    unsafe { CoTaskMemFree(Some(value.0.cast_const().cast::<c_void>())) };
    text.map_err(|_| windows::core::Error::empty())
}

fn create_attributes(initial_size: u32) -> windows::core::Result<IMFAttributes> {
    let mut attributes = None;
    // SAFETY: the out pointer is a valid local.
    unsafe { MFCreateAttributes(&mut attributes, initial_size) }?;
    attributes.ok_or_else(windows::core::Error::empty)
}

fn join_u64(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

fn split_u64(value: u64) -> (u32, u32) {
    let [high, low] = [value >> 32, value & 0xFFFF_FFFF].map(|part| u32::try_from(part).unwrap_or(0));
    (high, low)
}

fn first_video_stream() -> u32 {
    u32::from_ne_bytes(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0.to_ne_bytes())
}

fn all_streams() -> u32 {
    u32::from_ne_bytes(MF_SOURCE_READER_ALL_STREAMS.0.to_ne_bytes())
}

fn flag_bits(flag: i32) -> u32 {
    u32::from_ne_bytes(flag.to_ne_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_parameter_set_detection() {
        assert!(contains_sequence_parameter_set(&[0, 0, 0, 1, 0x67, 0x42]));
        assert!(!contains_sequence_parameter_set(&[0, 0, 0, 1, 0x65, 0x88]));
    }

    #[test]
    fn directshow_flags_map_to_swapped_rdpecam_modes() {
        assert_eq!(property_mode(NATIVE_FLAGS_AUTO), PropertyMode::AUTO);
        assert_eq!(property_mode(NATIVE_FLAGS_MANUAL), PropertyMode::MANUAL);
    }
}
