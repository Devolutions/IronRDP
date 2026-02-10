//! COM interface implementations for the DVC plugin bridge.
//!
//! We implement the "RDC client framework" side of the DVC plugin API:
//! - [`ChannelManager`] implements `IWTSVirtualChannelManager` (provides `CreateListener`)
//! - [`VirtualChannel`] implements `IWTSVirtualChannel` (provides `Write` / `Close`)
//! - [`Listener`] implements `IWTSListener` (stub `GetConfiguration`)
//!
//! The plugin DLL implements the other side:
//! - `IWTSPlugin` (lifecycle)
//! - `IWTSListenerCallback` (accept incoming channels)
//! - `IWTSVirtualChannelCallback` (receive data, close notifications)

use core::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ironrdp_dvc::encode_dvc_messages;
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, trace};
use windows::core::{Error, IUnknown, Ref, Result, PCSTR};
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, E_NOTIMPL};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag;
use windows::Win32::System::RemoteDesktop::{
    IWTSListener, IWTSListenerCallback, IWTSListener_Impl, IWTSVirtualChannel, IWTSVirtualChannelCallback,
    IWTSVirtualChannelManager, IWTSVirtualChannelManager_Impl, IWTSVirtualChannel_Impl,
};
use windows_core::implement;

/// Callback type for sending DVC messages from the COM plugin back into IronRDP's session loop.
pub(crate) type OnWriteDvc = Box<dyn Fn(u32, Vec<SvcMessage>) -> ironrdp_pdu::PduResult<()> + Send>;

// ─── IWTSVirtualChannelManager ──────────────────────────────────────────────

/// Rust implementation of `IWTSVirtualChannelManager`.
///
/// The plugin calls `CreateListener` during `IWTSPlugin::Initialize` to register
/// interest in named DVC channels. We store the channel name → listener callback
/// mapping so the worker can later dispatch `OnNewChannelConnection` when the
/// server opens a matching DVC.
#[implement(IWTSVirtualChannelManager)]
pub(crate) struct ChannelManager {
    /// channel_name → IWTSListenerCallback provided by the plugin.
    /// Shared via `Rc` so the caller can read the map after `Initialize` completes
    /// without needing an unsafe cast from the COM interface pointer.
    pub(crate) listeners: Rc<RefCell<HashMap<String, IWTSListenerCallback>>>,
}

impl ChannelManager {
    pub(crate) fn new(listeners: Rc<RefCell<HashMap<String, IWTSListenerCallback>>>) -> Self {
        Self { listeners }
    }
}

impl IWTSVirtualChannelManager_Impl for ChannelManager_Impl {
    fn CreateListener(
        &self,
        pszchannelname: &PCSTR,
        uflags: u32,
        plistenercallback: Ref<'_, IWTSListenerCallback>,
    ) -> Result<IWTSListener> {
        // SAFETY: pszchannelname is a null-terminated C string from the plugin
        let name = unsafe { pszchannelname.to_string() }
            .map_err(|e| Error::new(E_INVALIDARG, format!("invalid channel name: {e}")))?;

        debug!(channel_name = %name, flags = uflags, "Plugin registered DVC listener");

        let callback: IWTSListenerCallback = plistenercallback
            .ok()
            .map_err(|_| Error::new(E_INVALIDARG, "null listener callback"))?
            .clone();
        self.listeners.borrow_mut().insert(name.clone(), callback);

        let listener: IWTSListener = Listener { channel_name: name }.into();

        Ok(listener)
    }
}

// ─── IWTSListener ───────────────────────────────────────────────────────────

/// Stub `IWTSListener` implementation. Most plugins don't use `GetConfiguration`.
#[implement(IWTSListener)]
struct Listener {
    channel_name: String,
}

impl IWTSListener_Impl for Listener_Impl {
    fn GetConfiguration(&self) -> Result<IPropertyBag> {
        trace!(channel = %self.channel_name, "IWTSListener::GetConfiguration called (not implemented)");
        Err(Error::new(E_NOTIMPL, "GetConfiguration not implemented"))
    }
}

// ─── IWTSVirtualChannel ─────────────────────────────────────────────────────

/// Rust implementation of `IWTSVirtualChannel`.
///
/// When the plugin calls `Write()`, we encode the raw bytes as DVC data PDUs
/// and send them into IronRDP's session loop via the `on_write_dvc` callback.
#[implement(IWTSVirtualChannel)]
pub(crate) struct VirtualChannel {
    channel_id: u32,
    on_write_dvc: OnWriteDvc,
    closed: RefCell<bool>,
}

impl VirtualChannel {
    pub(crate) fn new(channel_id: u32, on_write_dvc: OnWriteDvc) -> Self {
        Self {
            channel_id,
            on_write_dvc,
            closed: RefCell::new(false),
        }
    }
}

/// A trivial DvcEncode wrapper for raw bytes going from the plugin to the server.
struct RawDvcData(Vec<u8>);

impl ironrdp_core::Encode for RawDvcData {
    fn encode(&self, dst: &mut ironrdp_core::WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "RawDvcData"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl ironrdp_dvc::DvcEncode for RawDvcData {}

impl IWTSVirtualChannel_Impl for VirtualChannel_Impl {
    fn Write(&self, cbsize: u32, pbuffer: *const u8, _preserved: Ref<'_, IUnknown>) -> Result<()> {
        if *self.closed.borrow() {
            return Err(Error::new(E_FAIL, "channel is closed"));
        }

        let size = usize::try_from(cbsize).expect("u32 fits in usize");
        if pbuffer.is_null() && size > 0 {
            return Err(Error::new(E_INVALIDARG, "null buffer"));
        }

        // SAFETY: the plugin guarantees the buffer is valid for the duration of Write()
        let data = if size > 0 {
            unsafe { core::slice::from_raw_parts(pbuffer, size) }.to_vec()
        } else {
            Vec::new()
        };

        trace!(
            channel_id = self.channel_id,
            size = data.len(),
            "IWTSVirtualChannel::Write"
        );

        let msg: ironrdp_dvc::DvcMessage = Box::new(RawDvcData(data));
        let svc_messages = encode_dvc_messages(self.channel_id, vec![msg], ChannelFlags::empty())
            .map_err(|e| Error::new(E_FAIL, format!("encode error: {e}")))?;

        (self.on_write_dvc)(self.channel_id, svc_messages)
            .map_err(|e| Error::new(E_FAIL, format!("send error: {e}")))?;

        Ok(())
    }

    fn Close(&self) -> Result<()> {
        debug!(channel_id = self.channel_id, "IWTSVirtualChannel::Close");
        *self.closed.borrow_mut() = true;
        Ok(())
    }
}

// ─── Active channel state ───────────────────────────────────────────────────

/// Per-channel state held on the COM thread. Tracks the COM objects for a single
/// open DVC channel so we can forward data from IronRDP → plugin.
pub(crate) struct ActiveChannel {
    pub(crate) callback: IWTSVirtualChannelCallback,
    pub(crate) _channel: IWTSVirtualChannel,
}
