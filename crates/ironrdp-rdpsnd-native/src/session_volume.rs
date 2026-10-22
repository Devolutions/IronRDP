//! Windows audio session volume for the playback stream.
//!
//! WASAPI streams opened without a session GUID join the process's default audio session on their
//! endpoint. Setting that session's volume is what the Windows volume mixer shows for this
//! application, and it is how the remote volume reaches the user without touching the device's
//! master volume or the samples themselves.

use windows::Win32::Media::Audio::{IAudioSessionManager, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::core::HSTRING;

/// Master volume of this process's default audio session on one endpoint.
///
/// COM objects are bound to the thread that created them, so this type is not `Send`.
pub(crate) struct SessionVolume {
    volume: ISimpleAudioVolume,
}

impl SessionVolume {
    /// Opens the default session volume of the endpoint with the given WASAPI endpoint ID.
    pub(crate) fn open(endpoint_id: &str) -> windows::core::Result<Self> {
        // SAFETY: initializing COM on this thread has no preconditions; a thread already initialized
        // in another apartment model reports RPC_E_CHANGED_MODE, and COM stays usable.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };

        // SAFETY: COM is initialized on this thread.
        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }?;
        // SAFETY: valid enumerator and a valid wide string ID.
        let device = unsafe { enumerator.GetDevice(&HSTRING::from(endpoint_id)) }?;
        // SAFETY: valid device; no activation parameters.
        let manager: IAudioSessionManager = unsafe { device.Activate(CLSCTX_ALL, None) }?;
        // SAFETY: valid session manager; no session GUID selects the process's default session.
        let volume = unsafe { manager.GetSimpleAudioVolume(None, 0) }?;
        Ok(Self { volume })
    }

    /// Sets the session master volume, from 0.0 (silence) to 1.0 (full).
    ///
    /// The level is audio-tapered by Windows, matching the logarithmic `dwVolume` scale of
    /// [MS-RDPEA] 2.2.2.2 "Client Audio Formats and Version PDU".
    ///
    /// [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
    pub(crate) fn set(&self, level: f32) -> windows::core::Result<()> {
        // SAFETY: a null event context is allowed.
        unsafe { self.volume.SetMasterVolume(level.clamp(0.0, 1.0), core::ptr::null()) }
    }
}
