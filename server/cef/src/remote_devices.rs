//! Supplies the remote client's camera, microphone and authenticator to the patched CEF runtime.
//!
//! The runtime always starts with these devices enabled, so pages never reach host cameras,
//! microphones or authenticators: without client input, the camera has no frames,
//! the microphone is silent, and Web Authentication requests fail.

use std::ffi::{CStr, c_char, c_float, c_int, c_void};
use std::time::Duration;

use crate::patched_runtime::Library;

/// Switches that replace capture devices with the embedder-fed camera and microphone.
///
/// The camera copies frames into shared memory; macOS otherwise expects zero-copy NV12 IOSurfaces.
pub const MEDIA_SWITCHES: [&str; 3] = [
    "use-fake-device-for-media-stream",
    "remote-media-capture",
    "disable-video-capture-use-gpu-memory-buffer",
];
/// Switch that sends every Web Authentication request to the embedder.
pub const WEBAUTHN_SWITCH: &str = "remote-webauthn";
/// The embedder feeds devices from the browser process, so capture must run there.
pub const ENABLE_FEATURES: &str = "RunVideoCaptureServiceInBrowserProcess";
/// Echo cancellation stays in the renderer: the in-process audio service would join its processing thread on the UI thread.
pub const DISABLE_FEATURES: &str = "AudioServiceOutOfProcess,ChromeWideEchoCancellation";

const INVALID: c_int = -1;
const UNKNOWN: c_int = -3;

/// Pixel layouts accepted by [`RemoteMedia::push_camera_frame`], with tightly packed planes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
#[cfg_attr(
    not(feature = "device-simulator"),
    expect(dead_code, reason = "the CEF API also accepts BGRA camera frames")
)]
pub enum PixelFormat {
    I420 = 0,
    Nv12 = 1,
    Yuy2 = 2,
    /// 32 bits per pixel, blue in the lowest address.
    Bgra = 3,
    Mjpeg = 4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraFormat {
    pub width: u32,
    pub height: u32,
    pub frame_rate: f32,
}

#[repr(C)]
struct RawAuthenticatorRequest {
    rp_id: *const c_char,
    origin: *const c_char,
    client_data_json: *const u8,
    client_data_json_size: usize,
    ctap_request: *const u8,
    ctap_request_size: usize,
    timeout_ms: u32,
    attachment: u32,
    user_verification: u32,
    attestation: u32,
    require_resident_key: c_int,
    prefer_resident_key: c_int,
}

type Enabled = unsafe extern "C" fn() -> c_int;
type GetFormat = unsafe extern "C" fn(*mut c_int, *mut c_int, *mut c_float) -> c_int;
type PushFrame = unsafe extern "C" fn(c_int, c_int, c_int, *const u8, usize, i64) -> c_int;
type PushAudio = unsafe extern "C" fn(*const i16, usize, c_int, c_int) -> c_int;
type ResetAudio = unsafe extern "C" fn();
type RequestFn = unsafe extern "C" fn(*mut c_void, u64, *const RawAuthenticatorRequest);
type CancelFn = unsafe extern "C" fn(*mut c_void, u64);
type SetHandler = unsafe extern "C" fn(Option<RequestFn>, Option<CancelFn>, *mut c_void) -> c_int;
type Respond = unsafe extern "C" fn(u64, *const u8, usize) -> c_int;
type SetPlatform = unsafe extern "C" fn(c_int) -> c_int;

/// The remote camera and microphone. Every method is callable from any thread.
#[derive(Clone, Copy)]
pub struct RemoteMedia {
    get_format: GetFormat,
    push_frame: PushFrame,
    microphone_active: Enabled,
    push_audio: PushAudio,
    reset_audio: ResetAudio,
}

impl RemoteMedia {
    pub fn load() -> anyhow::Result<Self> {
        let library = Library::open()?;
        // SAFETY: Each name is resolved to the signature the patched runtime declares for it.
        let (enabled, get_format, push_frame, microphone_active, push_audio, reset_audio) = unsafe {
            (
                library.symbol::<Enabled>(c"cef_remote_media_enabled")?,
                library.symbol::<GetFormat>(c"cef_remote_camera_get_format")?,
                library.symbol::<PushFrame>(c"cef_remote_camera_push_frame")?,
                library.symbol::<Enabled>(c"cef_remote_microphone_active")?,
                library.symbol::<PushAudio>(c"cef_remote_microphone_push")?,
                library.symbol::<ResetAudio>(c"cef_remote_microphone_reset")?,
            )
        };
        // SAFETY: The function takes no arguments and only reads the command line.
        anyhow::ensure!(
            unsafe { enabled() } == 1,
            "the CEF runtime did not enable its remote camera and microphone"
        );
        Ok(Self {
            get_format,
            push_frame,
            microphone_active,
            push_audio,
            reset_audio,
        })
    }

    /// Returns the format a page requested, or `None` while no page captures the camera.
    pub fn camera_format(&self) -> Option<CameraFormat> {
        let (mut width, mut height, mut frame_rate) = (0, 0, 0.0);
        // SAFETY: The outputs are writable locals.
        let capturing = unsafe { (self.get_format)(&mut width, &mut height, &mut frame_rate) } == 1;
        capturing.then_some(CameraFormat {
            width: u32::try_from(width).ok()?,
            height: u32::try_from(height).ok()?,
            frame_rate,
        })
    }

    /// Delivers one frame; returns `false` when no page is capturing.
    pub fn push_camera_frame(
        &self,
        format: PixelFormat,
        width: u32,
        height: u32,
        data: &[u8],
        timestamp: Duration,
    ) -> anyhow::Result<bool> {
        let timestamp = i64::try_from(timestamp.as_micros())?;
        // SAFETY: `data` is readable for its length.
        let status = unsafe {
            (self.push_frame)(
                format as c_int,
                c_int::try_from(width)?,
                c_int::try_from(height)?,
                data.as_ptr(),
                data.len(),
                timestamp,
            )
        };
        match status {
            0 => Ok(true),
            1 => Ok(false),
            _ => anyhow::bail!(
                "remote camera rejected a {width}x{height} {format:?} frame of {} bytes",
                data.len()
            ),
        }
    }

    /// Returns the number of open microphone streams.
    pub fn microphone_streams(&self) -> u32 {
        // SAFETY: The function takes no arguments.
        u32::try_from(unsafe { (self.microphone_active)() }).unwrap_or(0)
    }

    /// Appends interleaved samples; audio older than one second is dropped.
    pub fn push_microphone(&self, samples: &[i16], channels: u16, sample_rate: u32) -> anyhow::Result<()> {
        let channel_count = usize::from(channels);
        anyhow::ensure!(
            channel_count != 0 && samples.len().is_multiple_of(channel_count),
            "microphone samples do not fill whole frames"
        );
        // SAFETY: `samples` holds `samples.len() / channels` whole frames.
        let status = unsafe {
            (self.push_audio)(
                samples.as_ptr(),
                samples.len() / channel_count,
                c_int::from(channels),
                c_int::try_from(sample_rate)?,
            )
        };
        anyhow::ensure!(
            status == 0,
            "remote microphone rejected {channels} channels at {sample_rate} Hz"
        );
        Ok(())
    }

    pub fn reset_microphone(&self) {
        // SAFETY: The patched CEF function takes no arguments and resets its shared audio buffer.
        unsafe { (self.reset_audio)() };
    }
}

/// Parameters of one browser Web Authentication ceremony.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatorRequest {
    pub rp_id: String,
    pub origin: String,
    pub client_data_json: Vec<u8>,
    /// CTAP2 command byte (`0x01` or `0x02`) followed by its CBOR parameter map.
    pub ctap_request: Vec<u8>,
    pub timeout_ms: u32,
    pub attachment: u32,
    pub user_verification: u32,
    pub attestation: u32,
    pub require_resident_key: bool,
    pub prefer_resident_key: bool,
}

/// A message from the browser to the remote authenticator.
#[derive(Debug, PartialEq, Eq)]
pub enum AuthenticatorEvent {
    Request { id: u64, request: AuthenticatorRequest },
    Cancel { id: u64 },
}

type Handler = Box<dyn Fn(AuthenticatorEvent)>;

/// Answers authenticator requests and updates platform availability from any thread.
#[derive(Clone, Copy)]
pub struct AuthenticatorResponder {
    respond: Respond,
    set_platform: SetPlatform,
}

impl AuthenticatorResponder {
    /// Completes request `id` with a CTAP2 response: status byte, then CBOR.
    /// An empty response fails the request.
    pub fn respond(&self, id: u64, response: &[u8]) -> anyhow::Result<()> {
        // SAFETY: `response` is readable for its length.
        let status = unsafe { (self.respond)(id, response.as_ptr(), response.len()) };
        match status {
            0 | UNKNOWN => Ok(()),
            INVALID => anyhow::bail!("remote authenticator response was invalid"),
            _ => anyhow::bail!("remote authenticator response failed with {status}"),
        }
    }

    /// Sets whether pages see a user-verifying platform authenticator.
    pub fn set_platform_available(&self, available: bool) -> anyhow::Result<()> {
        // SAFETY: The function takes a plain integer and schedules UI-thread state update.
        let status = unsafe { (self.set_platform)(c_int::from(available)) };
        anyhow::ensure!(status == 0, "remote platform authenticator update failed with {status}");
        Ok(())
    }
}

/// The remote Web Authentication authenticator. Create and drop it on the CEF UI thread.
pub struct RemoteAuthenticator {
    set_handler: SetHandler,
    responder: AuthenticatorResponder,
    handler: Option<Box<Handler>>,
    _not_send: std::marker::PhantomData<*mut ()>,
}

impl RemoteAuthenticator {
    pub fn load() -> anyhow::Result<Self> {
        let library = Library::open()?;
        // SAFETY: Each name is resolved to the signature the patched runtime declares for it.
        let (enabled, set_handler, respond, set_platform) = unsafe {
            (
                library.symbol::<Enabled>(c"cef_remote_webauthn_enabled")?,
                library.symbol::<SetHandler>(c"cef_remote_webauthn_set_handler")?,
                library.symbol::<Respond>(c"cef_remote_webauthn_respond")?,
                library.symbol::<SetPlatform>(c"cef_remote_webauthn_set_platform_available")?,
            )
        };
        // SAFETY: The function takes no arguments and only reads the command line.
        anyhow::ensure!(
            unsafe { enabled() } == 1,
            "the CEF runtime did not enable its remote authenticator"
        );
        Ok(Self {
            set_handler,
            responder: AuthenticatorResponder { respond, set_platform },
            handler: None,
            _not_send: std::marker::PhantomData,
        })
    }

    pub fn responder(&self) -> AuthenticatorResponder {
        self.responder
    }

    /// Delivers requests and cancellations to `handler` on the CEF UI thread.
    pub fn set_handler(&mut self, handler: impl Fn(AuthenticatorEvent) + 'static) -> anyhow::Result<()> {
        let handler: Box<Handler> = Box::new(Box::new(handler));
        let context = std::ptr::from_ref::<Handler>(&handler).cast_mut().cast::<c_void>();
        // SAFETY: On the UI thread the runtime installs the callbacks before returning, and `context`
        // stays valid until the callbacks are replaced in `clear` or a later `set_handler`.
        let status = unsafe { (self.set_handler)(Some(on_request), Some(on_cancel), context) };
        anyhow::ensure!(status == 0, "remote authenticator handler failed with {status}");
        self.handler = Some(handler);
        Ok(())
    }

    /// Removes the handler, failing pending requests.
    pub fn clear(&mut self) {
        // SAFETY: Null callbacks remove the handler before the previous context is dropped.
        unsafe { (self.set_handler)(None, None, std::ptr::null_mut()) };
        self.handler = None;
    }

    /// Sets whether pages see a user-verifying platform authenticator.
    pub fn set_platform_available(&self, available: bool) -> anyhow::Result<()> {
        self.responder.set_platform_available(available)
    }
}

impl Drop for RemoteAuthenticator {
    fn drop(&mut self) {
        if self.handler.is_some() {
            self.clear();
        }
    }
}

unsafe fn copy_bytes(ptr: *const u8, len: usize) -> Vec<u8> {
    if ptr.is_null() || len == 0 {
        Vec::new()
    } else {
        // SAFETY: Caller guarantees `ptr` points to `len` readable bytes for this call.
        unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
    }
}

unsafe fn copy_cstr(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        // SAFETY: Caller guarantees `ptr` is a valid NUL-terminated string for this call.
        unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
    }
}

unsafe extern "C" fn on_request(context: *mut c_void, id: u64, raw: *const RawAuthenticatorRequest) {
    // SAFETY: `context` is the live handler installed by `set_handler`.
    let handler = unsafe { &*context.cast::<Handler>() };
    if raw.is_null() {
        return;
    }
    // SAFETY: The runtime passes a valid struct pointer whose fields stay valid during this call.
    let raw = unsafe { &*raw };
    let request = unsafe {
        AuthenticatorRequest {
            rp_id: copy_cstr(raw.rp_id),
            origin: copy_cstr(raw.origin),
            client_data_json: copy_bytes(raw.client_data_json, raw.client_data_json_size),
            ctap_request: copy_bytes(raw.ctap_request, raw.ctap_request_size),
            timeout_ms: raw.timeout_ms,
            attachment: raw.attachment,
            user_verification: raw.user_verification,
            attestation: raw.attestation,
            require_resident_key: raw.require_resident_key != 0,
            prefer_resident_key: raw.prefer_resident_key != 0,
        }
    };
    handler(AuthenticatorEvent::Request { id, request });
}

unsafe extern "C" fn on_cancel(context: *mut c_void, id: u64) {
    // SAFETY: `context` is the live handler installed by `set_handler`.
    let handler = unsafe { &*context.cast::<Handler>() };
    handler(AuthenticatorEvent::Cancel { id });
}

#[cfg(test)]
unsafe extern "C" fn no_op_reset_audio() {}

#[cfg(test)]
impl RemoteMedia {
    pub(crate) fn for_test(
        get_format: GetFormat,
        push_frame: PushFrame,
        microphone_active: Enabled,
        push_audio: PushAudio,
    ) -> Self {
        Self {
            get_format,
            push_frame,
            microphone_active,
            push_audio,
            reset_audio: no_op_reset_audio,
        }
    }

    pub(crate) fn for_test_with_reset(
        get_format: GetFormat,
        push_frame: PushFrame,
        microphone_active: Enabled,
        push_audio: PushAudio,
        reset_audio: ResetAudio,
    ) -> Self {
        Self {
            get_format,
            push_frame,
            microphone_active,
            push_audio,
            reset_audio,
        }
    }
}

#[cfg(test)]
impl AuthenticatorResponder {
    pub(crate) fn for_test(respond: Respond, set_platform: SetPlatform) -> Self {
        Self { respond, set_platform }
    }
}
