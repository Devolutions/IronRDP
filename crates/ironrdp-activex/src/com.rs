use core::ffi::c_void;
use core::panic::AssertUnwindSafe;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::panic::catch_unwind;

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_NOTIMPL, E_POINTER, FreeLibrary, HMODULE, HWND,
    S_FALSE, S_OK,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::{
    FreeLibraryAndExitThread, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleHandleExW,
};
use windows::core::PCWSTR;
use windows_core::{BOOL, GUID, HRESULT, IUnknown, Interface as _, Ref, Result, implement};

use crate::control::{CLSID_IRONRDP_ACTIVEX, Control, dispatcher_class_is_registered, is_supported_class};

static OBJECT_COUNT: AtomicU32 = AtomicU32::new(0);
static WORKER_COUNT: AtomicU32 = AtomicU32::new(0);
static SERVER_LOCK_COUNT: AtomicU32 = AtomicU32::new(0);
static OBJECT_COUNT_UNRELIABLE: AtomicBool = AtomicBool::new(false);
static WORKER_COUNT_UNRELIABLE: AtomicBool = AtomicBool::new(false);

pub(crate) fn add_object() {
    increment_count(&OBJECT_COUNT, &OBJECT_COUNT_UNRELIABLE);
}

pub(crate) fn release_object() {
    decrement_count(&OBJECT_COUNT, &OBJECT_COUNT_UNRELIABLE);
}

pub(crate) fn add_worker() {
    increment_count(&WORKER_COUNT, &WORKER_COUNT_UNRELIABLE);
}

pub(crate) fn release_worker() {
    decrement_count(&WORKER_COUNT, &WORKER_COUNT_UNRELIABLE);
}

pub(crate) fn retain_module_reference() -> Result<HMODULE> {
    let mut module = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            PCWSTR(DllGetClassObject as *const c_void as *const u16),
            &mut module,
        )
    }?;
    Ok(module)
}

pub(crate) fn retain_module_for_worker() -> Result<HMODULE> {
    retain_module_reference()
}

pub(crate) fn release_module_reference(module: HMODULE) {
    if let Err(error) = unsafe { FreeLibrary(module) } {
        tracing::error!(?error, "Unable to release ActiveX module reference");
    }
}

pub(crate) unsafe fn release_module_and_exit_worker(module: HMODULE) -> ! {
    unsafe { FreeLibraryAndExitThread(module, 0) }
}

fn can_unload() -> bool {
    counts_allow_unloading(
        OBJECT_COUNT.load(Ordering::Acquire),
        WORKER_COUNT.load(Ordering::Acquire),
        SERVER_LOCK_COUNT.load(Ordering::Acquire),
        OBJECT_COUNT_UNRELIABLE.load(Ordering::Acquire),
        WORKER_COUNT_UNRELIABLE.load(Ordering::Acquire),
    ) && !dispatcher_class_is_registered()
}

fn counts_allow_unloading(
    object_count: u32,
    worker_count: u32,
    server_lock_count: u32,
    object_count_unreliable: bool,
    worker_count_unreliable: bool,
) -> bool {
    object_count == 0
        && worker_count == 0
        && server_lock_count == 0
        && !object_count_unreliable
        && !worker_count_unreliable
}

#[implement(IClassFactory)]
struct ClassFactory {
    class_id: GUID,
}

impl ClassFactory {
    fn new(class_id: GUID) -> Self {
        add_object();
        Self { class_id }
    }
}

impl Drop for ClassFactory {
    fn drop(&mut self) {
        release_object();
    }
}

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> Result<()> {
        if ppvobject.is_null() {
            return Err(windows_core::Error::from_hresult(E_POINTER));
        }

        // SAFETY: ppvobject was checked above and is an output pointer owned by the caller.
        unsafe {
            ppvobject.write(ptr::null_mut());
        }

        if riid.is_null() {
            return Err(windows_core::Error::from_hresult(E_POINTER));
        }

        match catch_unwind(AssertUnwindSafe(|| {
            if !punkouter.is_null() {
                return Err(windows_core::Error::from_hresult(CLASS_E_NOAGGREGATION));
            }

            let control: IUnknown = Control::new_for_class(self.class_id).into();

            // SAFETY: riid and ppvobject were checked above. QueryInterface takes ownership of the
            // reference it writes to ppvobject; control remains owned locally on failure.
            unsafe { control.query(riid, ppvobject).ok() }
        })) {
            Ok(result) => result,
            Err(_) => Err(windows_core::Error::from_hresult(E_FAIL)),
        }
    }

    fn LockServer(&self, flock: BOOL) -> Result<()> {
        if flock.as_bool() {
            let previous =
                SERVER_LOCK_COUNT.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| count.checked_add(1));

            if previous.is_err() {
                return Err(windows_core::Error::from_hresult(E_FAIL));
            }
        } else {
            let previous =
                SERVER_LOCK_COUNT.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| count.checked_sub(1));

            if previous.is_err() {
                return Err(windows_core::Error::from_hresult(E_FAIL));
            }
        }

        Ok(())
    }
}

/// Returns the class factory for a supported CLSID.
///
/// # Safety
///
/// `rclsid`, `riid`, and `ppv` must be valid non-null pointers for the duration of the call.
/// `ppv` must point to writable storage for a COM interface pointer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    match catch_unwind(AssertUnwindSafe(|| {
        if ppv.is_null() {
            return E_POINTER;
        }

        // SAFETY: ppv was checked above and is a caller-provided output pointer.
        unsafe {
            ppv.write(ptr::null_mut());
        }

        if rclsid.is_null() || riid.is_null() {
            return E_POINTER;
        }

        // SAFETY: rclsid was checked above and only read for the duration of this call.
        if !is_supported_class(unsafe { &*rclsid }) {
            return CLASS_E_CLASSNOTAVAILABLE;
        }

        let factory: IClassFactory = ClassFactory::new(unsafe { *rclsid }).into();

        // SAFETY: riid and ppv were checked above; QueryInterface initializes ppv on success.
        unsafe { factory.query(riid, ppv) }
    })) {
        Ok(result) => result,
        Err(_) => E_FAIL,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    match catch_unwind(AssertUnwindSafe(can_unload)) {
        Ok(true) => S_OK,
        Ok(false) => S_FALSE,
        Err(_) => E_FAIL,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    match catch_unwind(AssertUnwindSafe(crate::registration::register_server)) {
        Ok(Ok(())) => S_OK,
        Ok(Err(error)) => error.code(),
        Err(_) => E_FAIL,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    match catch_unwind(AssertUnwindSafe(crate::registration::unregister_server)) {
        Ok(Ok(())) => S_OK,
        Ok(Err(error)) => error.code(),
        Err(_) => E_FAIL,
    }
}

/// Returns the installed mstscax control version expected by the native mstsc host.
///
/// The value is an ABI compatibility marker; it does not indicate full mstscax binary parity.
#[unsafe(no_mangle)]
pub extern "system" fn DllGetTscCtlVer() -> u64 {
    0x0000_0000_0000_65F4
}

/// Accepts mstscax authentication properties.
///
/// IronRDP does not expose the proprietary authentication-property bitfield.
#[unsafe(no_mangle)]
pub extern "system" fn DllSetAuthProperties(_properties: u64) -> HRESULT {
    E_NOTIMPL
}

/// Retrieves an mstscax claims token.
///
/// # Safety
///
/// When non-null, `claims_token` and `actual_authority` must point to writable BSTR storage.
/// The remaining raw BSTR pointers must be valid according to the mstscax ABI for the duration
/// of the call.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClaimsToken(
    _client_address: *const u16,
    _claims_hint: *const u16,
    _username_hint: *const u16,
    _user_domain_hint: *const u16,
    _parent_window: HWND,
    claims_token: *mut *const u16,
    actual_authority: *mut *const u16,
    _logon_cert_authority: *const u16,
    _wvd_activity_id: *const u16,
) -> HRESULT {
    // A failure result still requires null output BSTRs so callers can safely invoke SysFreeString.
    if !claims_token.is_null() {
        unsafe {
            claims_token.write(ptr::null());
        }
    }
    if !actual_authority.is_null() {
        unsafe {
            actual_authority.write(ptr::null());
        }
    }
    E_NOTIMPL
}

/// Supplies a proprietary mstscax claims token.
#[unsafe(no_mangle)]
pub extern "system" fn DllSetClaimsToken(_a1: u64, _a2: u64, _refresh_token: *const u16) -> HRESULT {
    E_NOTIMPL
}

/// Logs off an mstscax claims token.
#[unsafe(no_mangle)]
pub extern "system" fn DllLogoffClaimsToken(_claims_hint: *const u16) -> HRESULT {
    E_NOTIMPL
}

/// Cancels a proprietary mstscax authentication flow.
#[unsafe(no_mangle)]
pub extern "system" fn DllCancelAuthentication() -> HRESULT {
    E_NOTIMPL
}

/// Deletes mstscax-saved credentials.
#[unsafe(no_mangle)]
pub extern "system" fn DllDeleteSavedCreds(_workspace_id: *const u16, _username: *const u16) -> HRESULT {
    E_NOTIMPL
}

pub(crate) fn own_class_id() -> GUID {
    CLSID_IRONRDP_ACTIVEX
}

fn increment_count(count: &AtomicU32, unreliable: &AtomicBool) {
    if unreliable.load(Ordering::Acquire) {
        return;
    }

    if count
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| current.checked_add(1))
        .is_err()
    {
        // An overflow makes the exact count unknowable; retaining the DLL is safer than unloading it.
        unreliable.store(true, Ordering::Release);
    }
}

fn decrement_count(count: &AtomicU32, unreliable: &AtomicBool) {
    if unreliable.load(Ordering::Acquire) {
        return;
    }

    if count
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| current.checked_sub(1))
        .is_err()
    {
        // An unmatched release makes the exact count unknowable; retaining the DLL is safer than unloading it.
        unreliable.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::IPersist;
    use windows::Win32::System::Ole::IOleObject;

    use crate::control::{CLSID_MS_RDP_CLIENT, RDM_COMPATIBILITY_CLSIDS};
    use crate::mstsc::{IMsRdpClient6, IMsRdpClient10, IRemoteDesktopClient};

    #[test]
    fn zero_tracked_references_allow_unloading() {
        assert!(counts_allow_unloading(0, 0, 0, false, false));
        assert!(!counts_allow_unloading(1, 0, 0, false, false));
        assert!(!counts_allow_unloading(0, 1, 0, false, false));
        assert!(!counts_allow_unloading(0, 0, 1, false, false));
        assert!(!counts_allow_unloading(0, 0, 0, true, false));
        assert!(!counts_allow_unloading(0, 0, 0, false, true));
    }

    #[test]
    fn unsupported_claims_token_export_clears_bstr_outputs() {
        let mut claims_token = ptr::dangling::<u16>();
        let mut authority = ptr::dangling::<u16>();

        let result = unsafe {
            DllGetClaimsToken(
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                HWND(ptr::null_mut()),
                &mut claims_token,
                &mut authority,
                ptr::null(),
                ptr::null(),
            )
        };

        assert_eq!(result, E_NOTIMPL);
        assert!(claims_token.is_null());
        assert!(authority.is_null());
    }

    #[test]
    fn class_object_export_clears_output_before_rejecting_null_inputs() {
        let mut object = ptr::NonNull::<u8>::dangling().as_ptr().cast::<c_void>();

        let result = unsafe { DllGetClassObject(ptr::null(), &IClassFactory::IID, &mut object) };
        assert_eq!(result, E_POINTER);
        assert!(object.is_null());

        object = ptr::NonNull::<u8>::dangling().as_ptr().cast::<c_void>();
        let result = unsafe { DllGetClassObject(&CLSID_IRONRDP_ACTIVEX, ptr::null(), &mut object) };
        assert_eq!(result, E_POINTER);
        assert!(object.is_null());
    }

    #[test]
    fn class_factory_clears_output_before_rejecting_null_iid() {
        let factory: IClassFactory = ClassFactory::new(CLSID_IRONRDP_ACTIVEX).into();
        let mut object = ptr::NonNull::<u8>::dangling().as_ptr().cast::<c_void>();

        let result =
            unsafe { (factory.vtable().CreateInstance)(factory.as_raw(), ptr::null_mut(), ptr::null(), &mut object) };

        assert_eq!(result, E_POINTER);
        assert!(object.is_null());
    }

    #[test]
    fn reference_counters_do_not_wrap_or_underflow() {
        let count = AtomicU32::new(u32::MAX);
        let unreliable = AtomicBool::new(false);
        increment_count(&count, &unreliable);
        assert_eq!(count.load(Ordering::Acquire), u32::MAX);
        assert!(unreliable.load(Ordering::Acquire));

        count.store(0, Ordering::Release);
        unreliable.store(false, Ordering::Release);
        decrement_count(&count, &unreliable);
        assert_eq!(count.load(Ordering::Acquire), 0);
        assert!(unreliable.load(Ordering::Acquire));
    }

    #[test]
    fn optional_mstscax_exports_are_honestly_unsupported() {
        assert_ne!(DllGetTscCtlVer(), 0);
        assert_eq!(DllSetAuthProperties(0), E_NOTIMPL);
        assert_eq!(DllSetClaimsToken(0, 0, ptr::null()), E_NOTIMPL);
        assert_eq!(DllLogoffClaimsToken(ptr::null()), E_NOTIMPL);
        assert_eq!(DllCancelAuthentication(), E_NOTIMPL);
        assert_eq!(DllDeleteSavedCreds(ptr::null(), ptr::null()), E_NOTIMPL);
    }

    #[test]
    fn class_factory_creates_the_raw_client10_contract() {
        let mut raw_factory = ptr::null_mut();
        let result = unsafe { DllGetClassObject(&CLSID_IRONRDP_ACTIVEX, &IClassFactory::IID, &mut raw_factory) };
        assert_eq!(result, S_OK);

        let factory = unsafe { IClassFactory::from_raw(raw_factory) };
        let client: IMsRdpClient10 = unsafe { factory.CreateInstance(None) }.expect("create raw IMsRdpClient10");
        assert!(client.cast::<IMsRdpClient6>().is_ok());
    }

    #[test]
    fn ironrdp_class_factory_creates_the_modern_client_contract() {
        let mut raw_factory = ptr::null_mut();
        let result = unsafe { DllGetClassObject(&CLSID_IRONRDP_ACTIVEX, &IClassFactory::IID, &mut raw_factory) };
        assert_eq!(result, S_OK);

        let factory = unsafe { IClassFactory::from_raw(raw_factory) };
        let client: IRemoteDesktopClient =
            unsafe { factory.CreateInstance(None) }.expect("create modern IRemoteDesktopClient");
        assert!(client.cast::<IMsRdpClient10>().is_ok());
    }

    #[test]
    fn class_factory_preserves_the_requested_compatibility_class_id() {
        let mut raw_factory = ptr::null_mut();
        let result = unsafe { DllGetClassObject(&CLSID_MS_RDP_CLIENT, &IClassFactory::IID, &mut raw_factory) };
        assert_eq!(result, S_OK);

        let factory = unsafe { IClassFactory::from_raw(raw_factory) };
        let client: IMsRdpClient10 = unsafe { factory.CreateInstance(None) }.expect("create compatibility client");
        let persist = client.cast::<IPersist>().expect("control supports persistence");
        assert_eq!(
            unsafe { persist.GetClassID() }.expect("retrieve compatibility class ID"),
            CLSID_MS_RDP_CLIENT
        );
        let ole_object = client
            .cast::<IOleObject>()
            .expect("control supports OLE object identity");
        assert_eq!(
            unsafe { ole_object.GetUserClassID() }.expect("retrieve compatibility user class ID"),
            CLSID_MS_RDP_CLIENT
        );
    }

    #[test]
    fn class_factory_activates_every_rdm_compatibility_class() {
        for class_id in RDM_COMPATIBILITY_CLSIDS {
            let mut raw_factory = ptr::null_mut();
            let result = unsafe { DllGetClassObject(class_id, &IClassFactory::IID, &mut raw_factory) };
            assert_eq!(result, S_OK, "create factory for {class_id:?}");

            let factory = unsafe { IClassFactory::from_raw(raw_factory) };
            let client: IMsRdpClient10 = unsafe { factory.CreateInstance(None) }
                .unwrap_or_else(|error| panic!("create {class_id:?}: {error:?}"));
            let persist = client.cast::<IPersist>().expect("control supports persistence");
            assert_eq!(
                unsafe { persist.GetClassID() }.expect("retrieve compatibility class ID"),
                *class_id
            );
        }
    }
}
