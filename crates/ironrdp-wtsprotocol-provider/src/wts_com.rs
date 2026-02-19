use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use ironrdp_pdu::nego;
use parking_lot::Mutex;
use tracing::{debug, info, warn};
use windows::Win32::Foundation::{E_NOTIMPL, E_POINTER, E_UNEXPECTED, HANDLE_PTR};
use windows::Win32::System::Com::Marshal::CoMarshalInterThreadInterfaceInStream;
use windows::Win32::System::Com::StructuredStorage::CoGetInterfaceAndReleaseStream;
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, IClassFactory, IClassFactory_Impl, IStream, COINIT_MULTITHREADED,
};
use windows::Win32::System::RemoteDesktop::{
    IWRdsProtocolConnection, IWRdsProtocolConnection_Impl, IWRdsProtocolLicenseConnection, IWRdsProtocolListener,
    IWRdsProtocolListenerCallback, IWRdsProtocolListener_Impl, IWRdsProtocolLogonErrorRedirector, IWRdsProtocolManager,
    IWRdsProtocolManager_Impl, IWRdsProtocolSettings, IWRdsProtocolShadowConnection, WRDS_CONNECTION_SETTINGS,
    WRDS_CONNECTION_SETTING_LEVEL_1, WRDS_LISTENER_SETTINGS, WRDS_LISTENER_SETTING_LEVEL,
    WRDS_LISTENER_SETTING_LEVEL_1, WRDS_SETTINGS, WTS_CLIENT_DATA, WTS_PROPERTY_VALUE, WTS_PROTOCOL_STATUS,
    WTS_SERVICE_STATE, WTS_SESSION_ID, WTS_USER_CREDENTIAL,
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
}

impl ComProtocolConnection {
    fn new(inner: Arc<ProtocolConnection>) -> Self {
        Self {
            inner,
            auth_bridge: CredsspServerBridge::default(),
            last_input_time: Mutex::new(0),
        }
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
        if !pkeyboardhandle.is_null() {
            unsafe {
                *pkeyboardhandle = HANDLE_PTR::default();
            }
        }

        if !pmousehandle.is_null() {
            unsafe {
                *pmousehandle = HANDLE_PTR::default();
            }
        }

        if !pbeephandle.is_null() {
            unsafe {
                *pbeephandle = HANDLE_PTR::default();
            }
        }

        Ok(())
    }

    fn GetVideoHandle(&self) -> windows_core::Result<HANDLE_PTR> {
        Ok(HANDLE_PTR::default())
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
        Ok(())
    }

    fn Close(&self) -> windows_core::Result<()> {
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
        _szendpointname: &PCSTR,
        _bstatic: BOOL,
        _requestedpriority: u32,
    ) -> windows_core::Result<usize> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "virtual channel creation is not implemented",
        ))
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
