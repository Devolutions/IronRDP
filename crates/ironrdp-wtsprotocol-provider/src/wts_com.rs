use std::ffi::CString;
use std::fs::OpenOptions;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use ironrdp_pdu::nego;
use parking_lot::Mutex;
use tracing::{debug, info, warn};
use windows::Win32::Foundation::{E_NOTIMPL, E_POINTER, E_UNEXPECTED, HANDLE, HANDLE_PTR};
use windows::Win32::System::Com::Marshal::CoMarshalInterThreadInterfaceInStream;
use windows::Win32::System::Com::StructuredStorage::CoGetInterfaceAndReleaseStream;
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, IClassFactory, IClassFactory_Impl, IStream, COINIT_MULTITHREADED,
};
use windows::Win32::System::RemoteDesktop::{
    IWRdsProtocolConnection, IWRdsProtocolConnection_Impl, IWRdsProtocolLicenseConnection, IWRdsProtocolListener,
    IWRdsProtocolListenerCallback, IWRdsProtocolListener_Impl, IWRdsProtocolLogonErrorRedirector, IWRdsProtocolManager,
    IWRdsProtocolManager_Impl, IWRdsProtocolSettings, IWRdsProtocolShadowConnection, WTSVirtualChannelClose,
    WTSVirtualChannelOpenEx, WRDS_CONNECTION_SETTINGS, WRDS_CONNECTION_SETTING_LEVEL_1, WRDS_LISTENER_SETTINGS,
    WRDS_LISTENER_SETTING_LEVEL, WRDS_LISTENER_SETTING_LEVEL_1, WRDS_SETTINGS, WTS_CHANNEL_OPTION_DYNAMIC,
    WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH, WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW, WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED,
    WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL, WTS_CLIENT_DATA, WTS_PROPERTY_VALUE, WTS_PROTOCOL_STATUS, WTS_SERVICE_STATE,
    WTS_SESSION_ID, WTS_USER_CREDENTIAL,
};
use windows_core::{implement, Interface, BOOL, GUID, PCSTR, PCWSTR};
use windows_core::{IUnknown, HRESULT};

use crate::auth_bridge::{CredsspPolicy, CredsspServerBridge};
use crate::connection::ProtocolConnection;
use crate::listener::ProtocolListener;
use crate::manager::ProtocolManager;

const CLASS_E_NOAGGREGATION: HRESULT = HRESULT(0x80040110u32 as i32);
const CLASS_E_CLASSNOTAVAILABLE: HRESULT = HRESULT(0x80040111u32 as i32);
const E_NOINTERFACE_HRESULT: HRESULT = HRESULT(0x80004002u32 as i32);
const S_OK: HRESULT = HRESULT(0);
const S_FALSE: HRESULT = HRESULT(1);

pub const IRONRDP_PROTOCOL_MANAGER_CLSID: GUID = GUID::from_u128(0x89c7ed1e_25e5_4b15_8f52_ae6df4a5ceaf);
pub const IRONRDP_PROTOCOL_MANAGER_CLSID_STR: &str = "{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}";

static SERVER_LOCK_COUNT: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_OBJECT_COUNT: AtomicUsize = AtomicUsize::new(0);

const IRONRDP_CLIPRDR_CHANNEL_NAME: &str = "cliprdr";
const IRONRDP_RDPSND_CHANNEL_NAME: &str = "rdpsnd";
const IRONRDP_DRDYNVC_CHANNEL_NAME: &str = "drdynvc";
const IRONRDP_DISPLAYCONTROL_CHANNEL_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";
const IRONRDP_GRAPHICS_CHANNEL_NAME: &str = "Microsoft::Windows::RDS::Graphics";
const IRONRDP_AINPUT_CHANNEL_NAME: &str = "FreeRDP::Advanced::Input";
const IRONRDP_ECHO_CHANNEL_NAME: &str = "ECHO";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IronRdpVirtualChannelServer {
    Cliprdr,
    Rdpsnd,
    Drdynvc,
    DisplayControl,
    Graphics,
    AdvancedInput,
    Echo,
}

impl IronRdpVirtualChannelServer {
    fn name(self) -> &'static str {
        match self {
            Self::Cliprdr => IRONRDP_CLIPRDR_CHANNEL_NAME,
            Self::Rdpsnd => IRONRDP_RDPSND_CHANNEL_NAME,
            Self::Drdynvc => IRONRDP_DRDYNVC_CHANNEL_NAME,
            Self::DisplayControl => IRONRDP_DISPLAYCONTROL_CHANNEL_NAME,
            Self::Graphics => IRONRDP_GRAPHICS_CHANNEL_NAME,
            Self::AdvancedInput => IRONRDP_AINPUT_CHANNEL_NAME,
            Self::Echo => IRONRDP_ECHO_CHANNEL_NAME,
        }
    }

    fn requires_drdynvc_backbone(self) -> bool {
        matches!(
            self,
            Self::DisplayControl | Self::Graphics | Self::AdvancedInput | Self::Echo
        )
    }

    fn default_dynamic_priority(self) -> u32 {
        match self {
            Self::AdvancedInput => WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL,
            Self::Graphics => WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH,
            Self::DisplayControl => WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED,
            Self::Echo => WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
            Self::Cliprdr | Self::Rdpsnd | Self::Drdynvc => WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
        }
    }
}

fn endpoint_name_eq(lhs: &str, rhs: &str) -> bool {
    lhs.eq_ignore_ascii_case(rhs)
}

fn ironrdp_virtual_channel_server(endpoint_name: &str, is_static: bool) -> Option<IronRdpVirtualChannelServer> {
    if is_static {
        if endpoint_name_eq(endpoint_name, IRONRDP_CLIPRDR_CHANNEL_NAME) {
            return Some(IronRdpVirtualChannelServer::Cliprdr);
        }
        if endpoint_name_eq(endpoint_name, IRONRDP_RDPSND_CHANNEL_NAME) {
            return Some(IronRdpVirtualChannelServer::Rdpsnd);
        }
        if endpoint_name_eq(endpoint_name, IRONRDP_DRDYNVC_CHANNEL_NAME) {
            return Some(IronRdpVirtualChannelServer::Drdynvc);
        }

        return None;
    }

    if endpoint_name_eq(endpoint_name, IRONRDP_DISPLAYCONTROL_CHANNEL_NAME) {
        return Some(IronRdpVirtualChannelServer::DisplayControl);
    }
    if endpoint_name_eq(endpoint_name, IRONRDP_GRAPHICS_CHANNEL_NAME) {
        return Some(IronRdpVirtualChannelServer::Graphics);
    }
    if endpoint_name_eq(endpoint_name, IRONRDP_AINPUT_CHANNEL_NAME) {
        return Some(IronRdpVirtualChannelServer::AdvancedInput);
    }
    if endpoint_name_eq(endpoint_name, IRONRDP_ECHO_CHANNEL_NAME) {
        return Some(IronRdpVirtualChannelServer::Echo);
    }

    None
}

fn is_dynamic_channel_priority(value: u32) -> bool {
    matches!(
        value,
        WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
            | WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
            | WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH
            | WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL
    )
}

fn virtual_channel_requested_priority(
    is_static: bool,
    requested_priority: u32,
    hook_target: Option<IronRdpVirtualChannelServer>,
) -> u32 {
    if is_static {
        return requested_priority;
    }

    if requested_priority == WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW {
        return hook_target
            .map(IronRdpVirtualChannelServer::default_dynamic_priority)
            .unwrap_or(requested_priority);
    }

    if is_dynamic_channel_priority(requested_priority) {
        return requested_priority;
    }

    hook_target
        .map(IronRdpVirtualChannelServer::default_dynamic_priority)
        .unwrap_or(requested_priority)
}

#[derive(Debug)]
struct ListenerWorker {
    stop_tx: mpsc::Sender<()>,
    join_handle: thread::JoinHandle<()>,
}

pub fn create_protocol_manager_com() -> IWRdsProtocolManager {
    ComProtocolManager::new().into()
}

#[implement(IClassFactory)]
struct ProtocolManagerClassFactory;

impl IClassFactory_Impl for ProtocolManagerClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: windows_core::Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> windows_core::Result<()> {
        if ppvobject.is_null() || riid.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null COM output pointer"));
        }

        if punkouter.is_some() {
            return Err(windows_core::Error::new(
                CLASS_E_NOAGGREGATION,
                "aggregation is not supported",
            ));
        }

        unsafe {
            *ppvobject = core::ptr::null_mut();
        }

        let requested_iid = unsafe { *riid };

        if requested_iid == IWRdsProtocolManager::IID {
            let manager = create_protocol_manager_com();
            unsafe {
                *ppvobject = manager.into_raw() as *mut core::ffi::c_void;
            }

            return Ok(());
        }

        if requested_iid == IUnknown::IID {
            let manager = create_protocol_manager_com();
            let unknown: IUnknown = manager.cast()?;
            unsafe {
                *ppvobject = unknown.into_raw() as *mut core::ffi::c_void;
            }

            return Ok(());
        }

        Err(windows_core::Error::new(
            E_NOINTERFACE_HRESULT,
            "requested interface is not supported",
        ))
    }

    fn LockServer(&self, flock: BOOL) -> windows_core::Result<()> {
        if flock.as_bool() {
            SERVER_LOCK_COUNT.fetch_add(1, Ordering::SeqCst);
        } else {
            let _ =
                SERVER_LOCK_COUNT.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| current.checked_sub(1));
        }

        Ok(())
    }
}

#[allow(unreachable_pub)]
#[unsafe(no_mangle)]
pub extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> HRESULT {
    let result = dll_get_class_object_impl(rclsid, riid, ppv);

    match result {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

#[allow(unreachable_pub)]
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if SERVER_LOCK_COUNT.load(Ordering::SeqCst) == 0 && ACTIVE_OBJECT_COUNT.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

fn dll_get_class_object_impl(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> windows_core::Result<()> {
    if ppv.is_null() || riid.is_null() || rclsid.is_null() {
        return Err(windows_core::Error::new(E_POINTER, "null class object pointer"));
    }

    unsafe {
        *ppv = core::ptr::null_mut();
    }

    let requested_clsid = unsafe { *rclsid };
    if requested_clsid != IRONRDP_PROTOCOL_MANAGER_CLSID {
        return Err(windows_core::Error::new(
            CLASS_E_CLASSNOTAVAILABLE,
            "unknown protocol manager CLSID",
        ));
    }

    let factory: IClassFactory = ProtocolManagerClassFactory.into();
    let requested_iid = unsafe { *riid };

    if requested_iid == IClassFactory::IID {
        unsafe {
            *ppv = factory.into_raw() as *mut core::ffi::c_void;
        }

        return Ok(());
    }

    if requested_iid == IUnknown::IID {
        let unknown: IUnknown = factory.cast()?;
        unsafe {
            *ppv = unknown.into_raw() as *mut core::ffi::c_void;
        }

        return Ok(());
    }

    Err(windows_core::Error::new(
        E_NOINTERFACE_HRESULT,
        "requested class factory interface is not supported",
    ))
}

#[implement(IWRdsProtocolManager)]
struct ComProtocolManager {
    _lifetime: ComObjectLifetime,
    inner: ProtocolManager,
}

impl ComProtocolManager {
    fn new() -> Self {
        Self {
            _lifetime: ComObjectLifetime::new(),
            inner: ProtocolManager::new(),
        }
    }
}

#[derive(Debug)]
struct ComObjectLifetime;

impl ComObjectLifetime {
    fn new() -> Self {
        ACTIVE_OBJECT_COUNT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for ComObjectLifetime {
    fn drop(&mut self) {
        ACTIVE_OBJECT_COUNT.fetch_sub(1, Ordering::SeqCst);
    }
}

impl IWRdsProtocolManager_Impl for ComProtocolManager_Impl {
    fn Initialize(
        &self,
        _piwrdssettings: windows_core::Ref<'_, IWRdsProtocolSettings>,
        _pwrdssettings: *const WRDS_SETTINGS,
    ) -> windows_core::Result<()> {
        info!("Initialized protocol manager");
        Ok(())
    }

    fn CreateListener(&self, _wszlistenername: &PCWSTR) -> windows_core::Result<IWRdsProtocolListener> {
        info!("Created protocol listener");
        Ok(ComProtocolListener::new(self.inner.create_listener()).into())
    }

    fn NotifyServiceStateChange(&self, _ptsservicestatechange: *const WTS_SERVICE_STATE) -> windows_core::Result<()> {
        info!("Received service state change notification");
        Ok(())
    }

    fn NotifySessionOfServiceStart(&self, _sessionid: *const WTS_SESSION_ID) -> windows_core::Result<()> {
        debug!("Received session service start notification");
        Ok(())
    }

    fn NotifySessionOfServiceStop(&self, _sessionid: *const WTS_SESSION_ID) -> windows_core::Result<()> {
        debug!("Received session service stop notification");
        Ok(())
    }

    fn NotifySessionStateChange(&self, _sessionid: *const WTS_SESSION_ID, eventid: u32) -> windows_core::Result<()> {
        debug!(eventid, "Received session state change notification");
        Ok(())
    }

    fn NotifySettingsChange(&self, _pwrdssettings: *const WRDS_SETTINGS) -> windows_core::Result<()> {
        info!("Received protocol settings change notification");
        Ok(())
    }

    fn Uninitialize(&self) -> windows_core::Result<()> {
        info!("Uninitialized protocol manager");
        Ok(())
    }
}

#[implement(IWRdsProtocolListener)]
struct ComProtocolListener {
    inner: Arc<ProtocolListener>,
    callback: Mutex<Option<IWRdsProtocolListenerCallback>>,
    worker: Mutex<Option<ListenerWorker>>,
}

impl ComProtocolListener {
    fn new(inner: ProtocolListener) -> Self {
        Self {
            inner: Arc::new(inner),
            callback: Mutex::new(None),
            worker: Mutex::new(None),
        }
    }
}

impl IWRdsProtocolListener_Impl for ComProtocolListener_Impl {
    fn GetSettings(
        &self,
        _wrdslistenersettinglevel: WRDS_LISTENER_SETTING_LEVEL,
    ) -> windows_core::Result<WRDS_LISTENER_SETTINGS> {
        let mut settings = WRDS_LISTENER_SETTINGS::default();
        settings.WRdsListenerSettingLevel = WRDS_LISTENER_SETTING_LEVEL_1;
        Ok(settings)
    }

    fn StartListen(&self, pcallback: windows_core::Ref<'_, IWRdsProtocolListenerCallback>) -> windows_core::Result<()> {
        let callback = pcallback
            .ok()
            .map_err(|_| windows_core::Error::new(E_POINTER, "null listener callback"))?
            .clone();

        if self.worker.lock().is_some() {
            info!("Protocol listener already started");
            return Ok(());
        }

        let (stop_tx, stop_rx) = mpsc::channel();
        let callback_stream =
            unsafe { CoMarshalInterThreadInterfaceInStream(&IWRdsProtocolListenerCallback::IID, &callback) }?;
        let callback_stream_token = callback_stream.into_raw() as usize;
        let listener = Arc::clone(&self.inner);

        let join_handle = thread::spawn(move || {
            let co_initialize = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if let Err(error) = co_initialize.ok() {
                warn!(%error, "Failed to initialize COM on listener worker thread");
                return;
            }

            let callback_stream = unsafe { IStream::from_raw(callback_stream_token as *mut core::ffi::c_void) };
            let callback_for_worker =
                unsafe { CoGetInterfaceAndReleaseStream::<_, IWRdsProtocolListenerCallback>(&callback_stream) };
            std::mem::forget(callback_stream);

            let callback_for_worker = match callback_for_worker {
                Ok(callback) => callback,
                Err(error) => {
                    warn!(%error, "Failed to unmarshal listener callback in worker thread");
                    unsafe { CoUninitialize() };
                    return;
                }
            };

            let bootstrap_connection = listener.create_connection();
            let connection: IWRdsProtocolConnection = ComProtocolConnection::new(bootstrap_connection).into();

            let mut settings = WRDS_CONNECTION_SETTINGS::default();
            settings.WRdsConnectionSettingLevel = WRDS_CONNECTION_SETTING_LEVEL_1;

            match unsafe { callback_for_worker.OnConnected(&connection, &settings) } {
                Ok(connection_callback) => {
                    if let Err(error) = unsafe { connection_callback.OnReady() } {
                        warn!(%error, "Failed to send OnReady callback");
                    }
                }
                Err(error) => {
                    warn!(%error, "Failed to dispatch OnConnected callback");
                }
            }

            let _ = stop_rx.recv();

            unsafe { CoUninitialize() };
        });

        *self.worker.lock() = Some(ListenerWorker { stop_tx, join_handle });

        *self.callback.lock() = Some(callback);

        info!("Started protocol listener");

        Ok(())
    }

    fn StopListen(&self) -> windows_core::Result<()> {
        if let Some(worker) = self.worker.lock().take() {
            if let Err(error) = worker.stop_tx.send(()) {
                warn!(%error, "Failed to signal listener worker stop");
            }

            if let Err(error) = worker.join_handle.join() {
                warn!(?error, "Listener worker thread panicked");
            }
        }

        *self.callback.lock() = None;
        info!("Stopped protocol listener");
        Ok(())
    }
}

#[implement(IWRdsProtocolConnection)]
struct ComProtocolConnection {
    inner: Arc<ProtocolConnection>,
    auth_bridge: CredsspServerBridge,
    last_input_time: Mutex<u64>,
    input_video_handles: Mutex<Option<InputVideoHandles>>,
    virtual_channels: Mutex<Vec<VirtualChannelHandle>>,
}

impl ComProtocolConnection {
    fn new(inner: Arc<ProtocolConnection>) -> Self {
        Self {
            inner,
            auth_bridge: CredsspServerBridge::default(),
            last_input_time: Mutex::new(0),
            input_video_handles: Mutex::new(None),
            virtual_channels: Mutex::new(Vec::new()),
        }
    }

    fn ensure_input_video_handles(&self) -> windows_core::Result<()> {
        let mut handles_guard = self.input_video_handles.lock();
        if handles_guard.is_none() {
            *handles_guard = Some(InputVideoHandles::open()?);
        }

        Ok(())
    }

    fn get_keyboard_mouse_handles(&self) -> windows_core::Result<(HANDLE_PTR, HANDLE_PTR)> {
        self.ensure_input_video_handles()?;

        let handles_guard = self.input_video_handles.lock();
        let handles = handles_guard
            .as_ref()
            .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "input handles are not initialized"))?;

        Ok((handles.keyboard.as_handle_ptr(), handles.mouse.as_handle_ptr()))
    }

    fn get_video_handle(&self) -> windows_core::Result<HANDLE_PTR> {
        self.ensure_input_video_handles()?;

        let handles_guard = self.input_video_handles.lock();
        let handles = handles_guard
            .as_ref()
            .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "video handle is not initialized"))?;

        Ok(handles.video.as_handle_ptr())
    }

    fn release_input_video_handles(&self) {
        let mut handles_guard = self.input_video_handles.lock();
        *handles_guard = None;
    }

    fn release_virtual_channels(&self) {
        let mut channels = self.virtual_channels.lock();
        channels.clear();
    }

    fn find_virtual_channel(&self, endpoint_name: &str, is_static: bool) -> Option<HANDLE> {
        self.virtual_channels
            .lock()
            .iter()
            .find(|channel| channel.matches(endpoint_name, is_static))
            .map(VirtualChannelHandle::raw)
    }

    fn register_virtual_channel(
        &self,
        handle: HANDLE,
        endpoint_name: Option<String>,
        is_static: bool,
    ) -> windows_core::Result<HANDLE> {
        let mut channels = self.virtual_channels.lock();
        channels.push(VirtualChannelHandle::new(handle, endpoint_name, is_static));

        channels
            .last()
            .map(VirtualChannelHandle::raw)
            .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "virtual channel storage failure"))
    }

    fn open_virtual_channel_by_name(
        &self,
        session_id: u32,
        endpoint_name: &str,
        is_static: bool,
        requested_priority: u32,
    ) -> windows_core::Result<HANDLE> {
        if let Some(existing) = self.find_virtual_channel(endpoint_name, is_static) {
            return Ok(existing);
        }

        let endpoint_name_cstring = CString::new(endpoint_name)
            .map_err(|_| windows_core::Error::new(E_UNEXPECTED, "virtual channel endpoint contains NUL byte"))?;
        let endpoint = PCSTR::from_raw(endpoint_name_cstring.as_ptr().cast::<u8>());
        let flags = virtual_channel_open_flags(is_static, requested_priority);

        let channel = unsafe { WTSVirtualChannelOpenEx(session_id, endpoint, flags) }?;

        if let Some(existing) = self.find_virtual_channel(endpoint_name, is_static) {
            if let Err(error) = unsafe { WTSVirtualChannelClose(channel) } {
                warn!(%error, "Failed to close duplicate virtual channel handle");
            }

            return Ok(existing);
        }

        self.register_virtual_channel(channel, Some(endpoint_name.to_owned()), is_static)
    }

    fn ensure_ironrdp_drdynvc_channel(&self, session_id: u32) -> windows_core::Result<HANDLE> {
        self.open_virtual_channel_by_name(session_id, IRONRDP_DRDYNVC_CHANNEL_NAME, true, 0)
    }
}

impl IWRdsProtocolConnection_Impl for ComProtocolConnection_Impl {
    fn GetLogonErrorRedirector(&self) -> windows_core::Result<IWRdsProtocolLogonErrorRedirector> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "logon error redirector is not implemented",
        ))
    }

    fn AcceptConnection(&self) -> windows_core::Result<()> {
        self.auth_bridge.validate_security_protocol(
            CredsspPolicy::default(),
            nego::SecurityProtocol::HYBRID | nego::SecurityProtocol::HYBRID_EX,
        )?;

        self.inner.accept_connection().map_err(transition_error)?;
        Ok(())
    }

    fn GetClientData(&self, pclientdata: *mut WTS_CLIENT_DATA) -> windows_core::Result<()> {
        if pclientdata.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null client data pointer"));
        }

        unsafe {
            *pclientdata = WTS_CLIENT_DATA::default();
            (*pclientdata).fEnableWindowsKey = true;
            (*pclientdata).fInheritAutoLogon = BOOL(1);
            (*pclientdata).fNoAudioPlayback = true;
            copy_wide(&mut (*pclientdata).ProtocolName, "IRDP-WTS");
        }

        Ok(())
    }

    fn GetClientMonitorData(&self, pnummonitors: *mut u32, pprimarymonitor: *mut u32) -> windows_core::Result<()> {
        if !pnummonitors.is_null() {
            unsafe {
                *pnummonitors = 1;
            }
        }

        if !pprimarymonitor.is_null() {
            unsafe {
                *pprimarymonitor = 0;
            }
        }

        Ok(())
    }

    fn GetUserCredentials(&self, _pusercreds: *mut WTS_USER_CREDENTIAL) -> windows_core::Result<()> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "plaintext credential fallback is disabled",
        ))
    }

    fn GetLicenseConnection(&self) -> windows_core::Result<IWRdsProtocolLicenseConnection> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "license connection is not implemented",
        ))
    }

    fn AuthenticateClientToSession(&self, _sessionid: *mut WTS_SESSION_ID) -> windows_core::Result<()> {
        Ok(())
    }

    fn NotifySessionId(
        &self,
        sessionid: *const WTS_SESSION_ID,
        _sessionhandle: HANDLE_PTR,
    ) -> windows_core::Result<()> {
        if sessionid.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null session id pointer"));
        }

        let session_id = unsafe { (*sessionid).SessionId };
        self.inner.notify_session_id(session_id).map_err(transition_error)?;

        Ok(())
    }

    fn GetInputHandles(
        &self,
        pkeyboardhandle: *mut HANDLE_PTR,
        pmousehandle: *mut HANDLE_PTR,
        pbeephandle: *mut HANDLE_PTR,
    ) -> windows_core::Result<()> {
        if pkeyboardhandle.is_null() || pmousehandle.is_null() || pbeephandle.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null input handle output pointer"));
        }

        let (keyboard_handle, mouse_handle) = self.get_keyboard_mouse_handles()?;

        unsafe {
            *pkeyboardhandle = keyboard_handle;
            *pmousehandle = mouse_handle;
            *pbeephandle = HANDLE_PTR::default();
        }

        debug!("Returned protocol keyboard and mouse handles");

        Ok(())
    }

    fn GetVideoHandle(&self) -> windows_core::Result<HANDLE_PTR> {
        self.get_video_handle()
    }

    fn ConnectNotify(&self, _sessionid: u32) -> windows_core::Result<()> {
        self.inner.connect_notify().map_err(transition_error)?;
        Ok(())
    }

    fn IsUserAllowedToLogon(
        &self,
        _sessionid: u32,
        _usertoken: HANDLE_PTR,
        _pdomainname: &PCWSTR,
        _pusername: &PCWSTR,
    ) -> windows_core::Result<()> {
        Ok(())
    }

    fn SessionArbitrationEnumeration(
        &self,
        _husertoken: HANDLE_PTR,
        _bsinglesessionperuserenabled: BOOL,
        _psessionidarray: *mut u32,
        _pdwsessionidentifiercount: *mut u32,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "session arbitration uses default behavior",
        ))
    }

    fn LogonNotify(
        &self,
        _hclienttoken: HANDLE_PTR,
        _wszusername: &PCWSTR,
        _wszdomainname: &PCWSTR,
        _sessionid: *const WTS_SESSION_ID,
        _pwrdsconnectionsettings: *mut WRDS_CONNECTION_SETTINGS,
    ) -> windows_core::Result<()> {
        self.inner.logon_notify().map_err(transition_error)?;
        Ok(())
    }

    fn PreDisconnect(&self, _disconnectreason: u32) -> windows_core::Result<()> {
        Ok(())
    }

    fn DisconnectNotify(&self) -> windows_core::Result<()> {
        self.inner.disconnect_notify().map_err(transition_error)?;
        self.release_input_video_handles();
        self.release_virtual_channels();
        Ok(())
    }

    fn Close(&self) -> windows_core::Result<()> {
        self.release_virtual_channels();
        self.release_input_video_handles();
        self.inner.close().map_err(transition_error)?;
        Ok(())
    }

    fn GetProtocolStatus(&self, pprotocolstatus: *mut WTS_PROTOCOL_STATUS) -> windows_core::Result<()> {
        if pprotocolstatus.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null protocol status pointer"));
        }

        unsafe {
            *pprotocolstatus = WTS_PROTOCOL_STATUS::default();
        }

        Ok(())
    }

    fn GetLastInputTime(&self) -> windows_core::Result<u64> {
        Ok(*self.last_input_time.lock())
    }

    fn SetErrorInfo(&self, ulerror: u32) -> windows_core::Result<()> {
        warn!(ulerror, "Received protocol error info");
        Ok(())
    }

    fn CreateVirtualChannel(
        &self,
        szendpointname: &PCSTR,
        bstatic: BOOL,
        requestedpriority: u32,
    ) -> windows_core::Result<usize> {
        let session_id = self
            .inner
            .session_id()
            .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "session id is not available for virtual channel"))?;

        let endpoint = *szendpointname;
        if endpoint.is_null() {
            return Err(windows_core::Error::new(
                E_POINTER,
                "null virtual channel endpoint pointer",
            ));
        }

        let is_static = bstatic.as_bool();
        let endpoint_name = match unsafe { endpoint.to_string() } {
            Ok(name) => Some(name),
            Err(error) => {
                warn!(%error, "Failed to decode virtual channel endpoint name");
                None
            }
        };

        let hook_target = endpoint_name
            .as_deref()
            .and_then(|name| ironrdp_virtual_channel_server(name, is_static));

        let effective_priority = virtual_channel_requested_priority(is_static, requestedpriority, hook_target);

        if let Some(target) = hook_target {
            if target.requires_drdynvc_backbone() {
                if let Err(error) = self.ensure_ironrdp_drdynvc_channel(session_id) {
                    warn!(
                        %error,
                        endpoint = target.name(),
                        "Failed to pre-open DRDYNVC backbone for IronRDP dynamic channel"
                    );
                }
            }
        }

        if let Some(name) = endpoint_name.as_deref() {
            if let Some(existing_channel) = self.find_virtual_channel(name, is_static) {
                debug!(
                    session_id,
                    endpoint = name,
                    static_channel = is_static,
                    "Reusing virtual channel handle"
                );
                return Ok(existing_channel.0 as usize);
            }
        }

        let flags = virtual_channel_open_flags(is_static, effective_priority);

        let channel = if let Some(name) = endpoint_name.as_deref() {
            self.open_virtual_channel_by_name(session_id, name, is_static, effective_priority)?
        } else {
            let channel = unsafe { WTSVirtualChannelOpenEx(session_id, endpoint, flags) }?;
            self.register_virtual_channel(channel, None, is_static)?
        };

        if let Some(target) = hook_target {
            info!(
                session_id,
                endpoint = target.name(),
                static_channel = is_static,
                requestedpriority,
                effective_priority,
                flags,
                "Hooked IronRDP virtual channel server endpoint"
            );
        } else {
            debug!(
                session_id,
                requestedpriority,
                effective_priority,
                static_channel = is_static,
                flags,
                "Created virtual channel"
            );
        }

        Ok(channel.0 as usize)
    }

    fn QueryProperty(
        &self,
        _querytype: &GUID,
        _ulnumentriesin: u32,
        _ulnumentriesout: u32,
        _ppropertyentriesin: *const WTS_PROPERTY_VALUE,
        _ppropertyentriesout: *mut WTS_PROPERTY_VALUE,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::new(E_NOTIMPL, "query property is not implemented"))
    }

    fn GetShadowConnection(&self) -> windows_core::Result<IWRdsProtocolShadowConnection> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "shadow connection is not implemented",
        ))
    }

    fn NotifyCommandProcessCreated(&self, _sessionid: u32) -> windows_core::Result<()> {
        Ok(())
    }
}

fn copy_wide<const N: usize>(target: &mut [u16; N], value: &str) {
    let mut utf16 = value.encode_utf16().take(N.saturating_sub(1));

    for (index, code_unit) in utf16.by_ref().enumerate() {
        target[index] = code_unit;
    }
}

fn transition_error(message: &'static str) -> windows_core::Error {
    windows_core::Error::new(E_UNEXPECTED, message)
}

#[derive(Debug)]
struct InputVideoHandles {
    keyboard: OpenDeviceHandle,
    mouse: OpenDeviceHandle,
    video: OpenDeviceHandle,
}

impl InputVideoHandles {
    fn open() -> windows_core::Result<Self> {
        let keyboard = OpenDeviceHandle::open_first_available(
            "keyboard",
            "IRONRDP_WTS_KEYBOARD_DEVICE",
            &[r"\\.\KeyboardClass0", r"\\.\KeyboardClass1"],
        )?;

        let mouse = OpenDeviceHandle::open_first_available(
            "mouse",
            "IRONRDP_WTS_MOUSE_DEVICE",
            &[r"\\.\PointerClass0", r"\\.\PointerClass1"],
        )?;

        let video = OpenDeviceHandle::open_first_available(
            "video",
            "IRONRDP_WTS_VIDEO_DEVICE",
            &[r"\\.\RdpVideoMiniport", r"\\.\DISPLAY"],
        )?;

        info!(
            keyboard_device = %keyboard.path,
            mouse_device = %mouse.path,
            video_device = %video.path,
            "Opened protocol input and video device handles"
        );

        Ok(Self { keyboard, mouse, video })
    }
}

#[derive(Debug)]
struct OpenDeviceHandle {
    path: String,
    file: std::fs::File,
}

#[derive(Debug)]
struct VirtualChannelHandle {
    handle: HANDLE,
    endpoint_name: Option<String>,
    static_channel: bool,
}

impl VirtualChannelHandle {
    fn new(handle: HANDLE, endpoint_name: Option<String>, static_channel: bool) -> Self {
        Self {
            handle,
            endpoint_name,
            static_channel,
        }
    }

    fn raw(&self) -> HANDLE {
        self.handle
    }

    fn matches(&self, endpoint_name: &str, is_static: bool) -> bool {
        self.static_channel == is_static
            && self
                .endpoint_name
                .as_deref()
                .is_some_and(|name| endpoint_name_eq(name, endpoint_name))
    }
}

impl Drop for VirtualChannelHandle {
    fn drop(&mut self) {
        if let Err(error) = unsafe { WTSVirtualChannelClose(self.handle) } {
            warn!(%error, "Failed to close virtual channel handle");
        }
    }
}

fn virtual_channel_open_flags(is_static: bool, requested_priority: u32) -> u32 {
    if is_static {
        return 0;
    }

    let dynamic_priority = match requested_priority {
        WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        | WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
        | WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH
        | WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL => requested_priority,
        _ => WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
    };

    WTS_CHANNEL_OPTION_DYNAMIC | dynamic_priority
}

#[cfg(test)]
mod tests {
    use super::{
        ironrdp_virtual_channel_server, virtual_channel_open_flags, virtual_channel_requested_priority,
        IronRdpVirtualChannelServer,
    };
    use windows::Win32::System::RemoteDesktop::{
        WTS_CHANNEL_OPTION_DYNAMIC, WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH, WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
        WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED, WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL,
    };

    #[test]
    fn static_channels_use_zero_flags() {
        assert_eq!(virtual_channel_open_flags(true, 123), 0);
    }

    #[test]
    fn dynamic_channels_map_priority_flags() {
        assert_eq!(
            virtual_channel_open_flags(false, WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
        assert_eq!(
            virtual_channel_open_flags(false, WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
        );
        assert_eq!(
            virtual_channel_open_flags(false, WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH
        );
        assert_eq!(
            virtual_channel_open_flags(false, WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL
        );
    }

    #[test]
    fn dynamic_channels_fallback_to_low_for_unknown_priority() {
        assert_eq!(
            virtual_channel_open_flags(false, 1),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
        assert_eq!(
            virtual_channel_open_flags(false, 123),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
        assert_eq!(
            virtual_channel_open_flags(false, u32::MAX),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
    }

    #[test]
    fn recognizes_ironrdp_static_virtual_channel_servers() {
        assert_eq!(
            ironrdp_virtual_channel_server("cliprdr", true),
            Some(IronRdpVirtualChannelServer::Cliprdr)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("RDPSND", true),
            Some(IronRdpVirtualChannelServer::Rdpsnd)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("DrDyNvC", true),
            Some(IronRdpVirtualChannelServer::Drdynvc)
        );
        assert_eq!(ironrdp_virtual_channel_server("cliprdr", false), None);
    }

    #[test]
    fn recognizes_ironrdp_dynamic_virtual_channel_servers() {
        assert_eq!(
            ironrdp_virtual_channel_server("Microsoft::Windows::RDS::DisplayControl", false),
            Some(IronRdpVirtualChannelServer::DisplayControl)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("microsoft::windows::rds::graphics", false),
            Some(IronRdpVirtualChannelServer::Graphics)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("FreeRDP::Advanced::Input", false),
            Some(IronRdpVirtualChannelServer::AdvancedInput)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("echo", false),
            Some(IronRdpVirtualChannelServer::Echo)
        );
    }

    #[test]
    fn ironrdp_dynamic_channels_use_recommended_priorities_when_unknown() {
        assert_eq!(
            virtual_channel_requested_priority(false, 0, Some(IronRdpVirtualChannelServer::DisplayControl)),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
        );
        assert_eq!(
            virtual_channel_requested_priority(false, 0, Some(IronRdpVirtualChannelServer::Graphics)),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH
        );
        assert_eq!(
            virtual_channel_requested_priority(false, 0, Some(IronRdpVirtualChannelServer::AdvancedInput)),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL
        );
        assert_eq!(
            virtual_channel_requested_priority(false, 0, Some(IronRdpVirtualChannelServer::Echo)),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
    }

    #[test]
    fn explicit_dynamic_priority_is_preserved() {
        assert_eq!(
            virtual_channel_requested_priority(
                false,
                WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED,
                Some(IronRdpVirtualChannelServer::AdvancedInput)
            ),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
        );
    }

    #[test]
    fn dynamic_server_backbone_requirements_are_exposed() {
        assert!(IronRdpVirtualChannelServer::DisplayControl.requires_drdynvc_backbone());
        assert!(IronRdpVirtualChannelServer::Graphics.requires_drdynvc_backbone());
        assert!(IronRdpVirtualChannelServer::AdvancedInput.requires_drdynvc_backbone());
        assert!(IronRdpVirtualChannelServer::Echo.requires_drdynvc_backbone());
        assert!(!IronRdpVirtualChannelServer::Cliprdr.requires_drdynvc_backbone());
        assert!(!IronRdpVirtualChannelServer::Rdpsnd.requires_drdynvc_backbone());
        assert!(!IronRdpVirtualChannelServer::Drdynvc.requires_drdynvc_backbone());
    }
}

impl OpenDeviceHandle {
    fn open_first_available(
        device_kind: &str,
        env_var_name: &str,
        fallback_paths: &[&str],
    ) -> windows_core::Result<Self> {
        let configured_path = std::env::var(env_var_name).ok();

        let mut candidate_paths = Vec::with_capacity(fallback_paths.len() + usize::from(configured_path.is_some()));
        if let Some(path) = configured_path {
            if !path.trim().is_empty() {
                candidate_paths.push(path);
            }
        }

        candidate_paths.extend(fallback_paths.iter().map(|path| (*path).to_owned()));

        let mut failures = Vec::new();

        for path in candidate_paths {
            let open_result = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .or_else(|_| OpenOptions::new().read(true).open(&path));

            match open_result {
                Ok(file) => {
                    return Ok(Self { path, file });
                }
                Err(error) => {
                    failures.push(format!("{}: {}", path, error));
                }
            }
        }

        let error_message = format!(
            "failed to open {} device handle; set {} to an accessible device path; attempts: {}",
            device_kind,
            env_var_name,
            failures.join(" | ")
        );

        Err(windows_core::Error::new(E_NOTIMPL, error_message))
    }

    fn as_handle_ptr(&self) -> HANDLE_PTR {
        raw_handle_to_handle_ptr(self.file.as_raw_handle())
    }
}

fn raw_handle_to_handle_ptr(raw_handle: RawHandle) -> HANDLE_PTR {
    HANDLE_PTR(raw_handle as usize)
}
