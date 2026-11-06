#![expect(
    clippy::as_conversions,
    clippy::as_pointer_underscore,
    clippy::inline_always,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks,
    reason = "hand-written COM and WTS ABI glue mirrors Windows header pointer and integer representations"
)]

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use core::time::Duration;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write};
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::{AsRawHandle as _, IntoRawHandle as _};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread;

use ironrdp_wtsprotocol_ipc::{
    DEFAULT_MAX_FRAME_SIZE, ProviderCommand, RemoteAppSettings, ServiceEvent, default_pipe_name, pipe_path,
    read_json_message, resolve_pipe_name_from_env, write_json_message,
};
use parking_lot::Mutex;
use tracing::{debug, info, warn};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_ALLCLASSES, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SETUP_DI_REGISTRY_PROPERTY, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA, SPDRP_PHYSICAL_DEVICE_OBJECT_NAME,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceInterfaceDetailW, SetupDiGetDeviceRegistryPropertyW,
};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, CloseHandle, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL,
    E_OUTOFMEMORY as E_OUTOFMEMORY_WIN32, E_POINTER, E_UNEXPECTED, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND,
    ERROR_INSUFFICIENT_BUFFER, ERROR_NO_DATA, ERROR_NO_MORE_ITEMS, ERROR_NO_TOKEN, ERROR_PATH_NOT_FOUND,
    ERROR_SEM_TIMEOUT, GetLastError, HANDLE, HANDLE_PTR, HLOCAL, LocalFree, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    DuplicateTokenEx, IsValidSecurityDescriptor, LookupAccountNameW, PSECURITY_DESCRIPTOR, PSID, RevertToSelf,
    SID_NAME_USE, SecurityImpersonation, TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_QUERY, TokenImpersonation,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    QueryDosDeviceW, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Com::{
    COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemAlloc, CoTaskMemFree, CoUninitialize, IAgileObject,
    IAgileObject_Impl, IClassFactory, IClassFactory_Impl,
};
use windows::Win32::System::Memory::{LMEM_FIXED, LocalAlloc};
use windows::Win32::System::Pipes::{PIPE_READMODE_MESSAGE, PeekNamedPipe, SetNamedPipeHandleState};
use windows::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, RRF_RT_REG_MULTI_SZ, RRF_RT_REG_SZ, RRF_ZEROONFAILURE, RegGetValueW,
};
use windows::Win32::System::RemoteDesktop::{
    IWRdsProtocolConnection, IWRdsProtocolConnection_Impl, IWRdsProtocolConnection_Vtbl,
    IWRdsProtocolConnectionCallback, IWRdsProtocolConnectionSettings_Impl, IWRdsProtocolLicenseConnection,
    IWRdsProtocolLicenseConnection_Impl, IWRdsProtocolListener, IWRdsProtocolListener_Impl,
    IWRdsProtocolListenerCallback, IWRdsProtocolLogonErrorRedirector, IWRdsProtocolLogonErrorRedirector_Impl,
    IWRdsProtocolManager, IWRdsProtocolManager_Impl, IWRdsProtocolSettings, IWRdsProtocolShadowConnection,
    IWRdsWddmIddProps, IWRdsWddmIddProps_Impl, IWRdsWddmIddProps1, IWRdsWddmIddProps1_Impl,
    WRDS_CONNECTION_SETTING_LEVEL, WRDS_CONNECTION_SETTINGS, WRDS_LISTENER_SETTING_LEVEL, WRDS_LISTENER_SETTINGS,
    WRDS_LISTENER_SETTINGS_1, WRDS_SETTINGS, WTS_CERT_TYPE_INVALID, WTS_CLIENT_DATA, WTS_KEY_EXCHANGE_ALG_RSA,
    WTS_LICENSE_CAPABILITIES, WTS_LICENSE_PREAMBLE_VERSION, WTS_LOGON_ERR_NOT_HANDLED,
    WTS_LOGON_ERROR_REDIRECTOR_RESPONSE, WTS_PROCESS_INFOW, WTS_PROPERTY_VALUE, WTS_PROTOCOL_STATUS, WTS_SERVICE_STATE,
    WTS_SESSION_ID, WTS_USER_CREDENTIAL, WTSEnumerateProcessesW, WTSFreeMemory, WTSQueryUserToken,
};
use windows::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS,
    SERVICE_RUNNING, SERVICE_START, SERVICE_START_PENDING, SERVICE_STATUS, StartServiceW,
};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::Threading::{GetCurrentThread, OpenThreadToken, SetThreadToken};
use windows::core::AgileReference;
use windows_core::{BOOL, GUID, HRESULT, IUnknown, IUnknownImpl, Interface as _, PCSTR, PCWSTR, PWSTR, implement};

use crate::connection::{ConnectionLifecycleState, ProtocolConnection};
use crate::listener::ProtocolListener;
use crate::manager::ProtocolManager;
use crate::published_apps::PublishedApplications;

const S_OK: HRESULT = HRESULT(0);
const S_FALSE: HRESULT = HRESULT(1);
// HRESULT_FROM_WIN32(ERROR_OUTOFMEMORY=14) == 0x8007000E
const E_OUTOFMEMORY: HRESULT = HRESULT(-2147024882);
// CRED_PACK_PROTECTED_CREDENTIALS would encrypt the password for TermService's logon session,
// which Winlogon cannot decrypt, so the serialized credential is packed unprotected.
const CRED_PACK_UNPROTECTED: u32 = 0;

#[link(name = "credui")]
unsafe extern "system" {
    fn CredPackAuthenticationBufferW(
        dwflags: u32,
        pszusername: PWSTR,
        pszpassword: PWSTR,
        ppackedcredentials: *mut u8,
        pcbpackedcredentials: *mut u32,
    ) -> BOOL;
}

#[repr(C)]
struct WRDS_SERIALIZED_USER_CREDENTIAL {
    serialization_length: u32,
    serialization: *mut u8,
}

#[repr(transparent)]
#[derive(Clone, PartialEq, Eq)]
struct IWRdsProtocolConnection2(IUnknown);

unsafe impl windows_core::Interface for IWRdsProtocolConnection2 {
    type Vtable = IWRdsProtocolConnection2_Vtbl;
    const IID: GUID = GUID::from_u128(0xc2bd9b66_4a76_4701_b6a3_bfafc1482169);
}

impl core::ops::Deref for IWRdsProtocolConnection2 {
    type Target = IWRdsProtocolConnection;

    fn deref(&self) -> &Self::Target {
        // SAFETY: `IWRdsProtocolConnection2` extends `IWRdsProtocolConnection`.
        unsafe { &*core::ptr::from_ref(self).cast::<IWRdsProtocolConnection>() }
    }
}

#[repr(C)]
struct IWRdsProtocolConnection2_Vtbl {
    base__: IWRdsProtocolConnection_Vtbl,
    get_serialized_user_credential: unsafe extern "system" fn(
        this: *mut c_void,
        usercredential: *mut *mut WRDS_SERIALIZED_USER_CREDENTIAL,
    ) -> HRESULT,
}

#[expect(
    non_camel_case_types,
    non_snake_case,
    reason = "custom COM interface names mirror the Windows ABI"
)]
trait IWRdsProtocolConnection2_Impl: IWRdsProtocolConnection_Impl + IUnknownImpl {
    fn GetSerializedUserCredential(
        &self,
        usercredential: *mut *mut WRDS_SERIALIZED_USER_CREDENTIAL,
    ) -> windows_core::Result<()>;
}

impl IWRdsProtocolConnection2_Vtbl {
    const fn new<Identity: IWRdsProtocolConnection2_Impl, const OFFSET: isize>() -> Self {
        Self {
            base__: IWRdsProtocolConnection_Vtbl::new::<Identity, OFFSET>(),
            get_serialized_user_credential: Self::get_serialized_user_credential::<Identity, OFFSET>,
        }
    }

    fn matches(iid: &GUID) -> bool {
        let is_match = iid == &<IWRdsProtocolConnection2 as windows_core::Interface>::IID
            || IWRdsProtocolConnection_Vtbl::matches(iid);

        debug_log_line(&format!(
            "IWRdsProtocolConnection2::QueryInterfaceMatch iid={iid:?} matched={is_match}",
        ));

        is_match
    }

    unsafe extern "system" fn get_serialized_user_credential<
        Identity: IWRdsProtocolConnection2_Impl,
        const OFFSET: isize,
    >(
        this: *mut c_void,
        usercredential: *mut *mut WRDS_SERIALIZED_USER_CREDENTIAL,
    ) -> HRESULT {
        // SAFETY: `this` is the COM identity pointer for `Identity`.
        let this = unsafe { (this as *mut *mut c_void).offset(OFFSET) as *mut Identity };
        unsafe { (*this).GetSerializedUserCredential(usercredential) }.into()
    }
}

#[repr(transparent)]
#[derive(Clone)]
struct IWRdsEnhancedFastReconnectArbitrator(IUnknown);

unsafe impl windows_core::Interface for IWRdsEnhancedFastReconnectArbitrator {
    type Vtable = IWRdsEnhancedFastReconnectArbitrator_Vtbl;

    const IID: GUID = GUID::from_u128(0x5718_ae9b_47f2_499f_b634_d817_5bd5_1131);
}

impl core::ops::Deref for IWRdsEnhancedFastReconnectArbitrator {
    type Target = IUnknown;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[repr(C)]
struct IWRdsEnhancedFastReconnectArbitrator_Vtbl {
    base__: windows_core::IUnknown_Vtbl,
    get_session_for_enhanced_fast_reconnect: unsafe extern "system" fn(
        this: *mut c_void,
        psessionidarray: *const i32,
        dwsessioncount: u32,
        presultsessionid: *mut i32,
    ) -> HRESULT,
}

#[expect(
    non_camel_case_types,
    non_snake_case,
    reason = "custom COM interface names mirror the Windows ABI"
)]
trait IWRdsEnhancedFastReconnectArbitrator_Impl: IUnknownImpl {
    fn GetSessionForEnhancedFastReconnect(
        &self,
        psessionidarray: *const i32,
        dwsessioncount: u32,
        presultsessionid: *mut i32,
    ) -> windows_core::Result<()>;
}

impl IWRdsEnhancedFastReconnectArbitrator_Vtbl {
    const fn new<Identity: IWRdsEnhancedFastReconnectArbitrator_Impl, const OFFSET: isize>() -> Self {
        Self {
            base__: windows_core::IUnknown_Vtbl::new::<Identity, OFFSET>(),
            get_session_for_enhanced_fast_reconnect: Self::get_session_for_enhanced_fast_reconnect::<Identity, OFFSET>,
        }
    }

    fn matches(iid: &GUID) -> bool {
        let is_match = iid == &<IWRdsEnhancedFastReconnectArbitrator as windows_core::Interface>::IID;

        debug_log_line(&format!(
            "IWRdsEnhancedFastReconnectArbitrator::QueryInterfaceMatch iid={iid:?} matched={is_match}",
        ));

        is_match
    }

    unsafe extern "system" fn get_session_for_enhanced_fast_reconnect<
        Identity: IWRdsEnhancedFastReconnectArbitrator_Impl,
        const OFFSET: isize,
    >(
        this: *mut c_void,
        psessionidarray: *const i32,
        dwsessioncount: u32,
        presultsessionid: *mut i32,
    ) -> HRESULT {
        let this = unsafe { (this as *mut *mut c_void).offset(OFFSET) as *mut Identity };
        unsafe { (*this).GetSessionForEnhancedFastReconnect(psessionidarray, dwsessioncount, presultsessionid) }.into()
    }
}

// The custom IronRDP IDD is the primary strict-mode path. Provider-side fallback
// probing remains available for diagnostics, but strict e2e must prove that the
// active hardware ID and selected video source are both using the custom driver.
const IRONRDP_IDD_HARDWARE_ID: &str = "IronRdpIdd";
const IRONRDP_IDD_DEVICE_MARKERS: [&str; 2] = ["IronRdpIdd", "RdpIdd_IndirectDisplay"];
const GUID_DEVINTERFACE_IRONRDP_IDD_VIDEO: GUID = GUID::from_u128(0x1ea642e3_6a78_4f4b_9a19_2eb4f0f33b82);
const IRONRDP_IDD_RUNTIME_ROOT: &str = r"C:\ProgramData\IronRDP\Idd";
const IRONRDP_IDD_RUNTIME_STATE_FILE: &str = r"C:\ProgramData\IronRDP\Idd\runtime-config.txt";
const WTS_PROTOCOL_TYPE_NON_RDP: u16 = 2;

const WTS_VALUE_TYPE_ULONG: u16 = 1;
const WTS_VALUE_TYPE_STRING: u16 = 2;
const WTS_VALUE_TYPE_BINARY: u16 = 3;

// From Windows SDK `wtsdefs.h`.
const PROPERTY_TYPE_GET_FAST_RECONNECT: GUID = GUID::from_u128(0x6212_d757_0043_4862_99c3_9f30_59ac_2a3b);
const PROPERTY_TYPE_GET_FAST_RECONNECT_USER_SID: GUID = GUID::from_u128(0x197c_427a_0135_4b6d_9c5e_e657_9a0a_b625);
const PROPERTY_TYPE_CONNECTION_GUID: GUID = GUID::from_u128(0x9eaa_04f6_5b9d_4ba5_be9d_3748_ad6d_8af7);
const PROPERTY_TYPE_SUPPRESS_LOGON_UI: GUID = GUID::from_u128(0x846b_20bb_6254_430e_952f_b0c7_ca08_1915);
const PROPERTY_TYPE_CAPTURE_PROTECTED_CONTENT: GUID = GUID::from_u128(0x2918_db60_6cae_42a8_9945_8128_d7dd_8e71);
const PROPERTY_TYPE_LICENSE_GUID: GUID = GUID::from_u128(0x4daa_5ab8_8b6a_49cf_9c85_8add_504c_d1f7);

// From Windows SDK `wtsdefs.h`.
const WTS_QUERY_AUDIOENUM_DLL: GUID = GUID::from_u128(0x9bf4_fa97_c883_4c2a_80ab_5a39_c9af_00db);
const PROPERTY_TYPE_ENABLE_UNIVERSAL_APPS_FOR_CUSTOM_SHELL: GUID =
    GUID::from_u128(0xed2c_3fda_338d_4d3f_81a3_e767_310d_908e);
const WTS_QUERY_ALLOWED_INITIAL_APP: GUID = GUID::from_u128(0xc77d1b30_5be1_4c6b_a0e1_bd6d2e5c9fcc);

// Native RemoteApp shell properties use the WTS QueryProperty ABI.
// UNTESTED: These private GUIDs must remain compatible across Windows builds; check native shell startup on upgrades.
const PROPERTY_TYPE_RAIL_CLIENT_CAPABILITIES: GUID = GUID::from_u128(0xb8ce54c6_8a2d_4612_9c74_e715fc8bedea);
const PROPERTY_TYPE_WINDOW_CAPABILITYSET: GUID = GUID::from_u128(0x4042c6ed_aad1_4eac_b3c1_87b726726cd8);
const PROPERTY_TYPE_GET_REMOTEAPP_FEATURE_FLAGS: GUID = GUID::from_u128(0xe26e0aca_a72a_4b8e_ae5c_49bdfd467c4f);
const PROPERTY_TYPE_GET_REMOTEAPP_HD_SUPPORT_LEVEL: GUID = GUID::from_u128(0xae3c6a8e_87b2_472d_d0be_915c27db937c);
const TERMINAL_TYPE_RDP_REMOTEAPP: GUID = GUID::from_u128(0xeddcc3ce_6e7e_4f4b_8439_3d9ad4c9440f);

const FAST_RECONNECT_DISABLED: u32 = 0;
const FAST_RECONNECT_BASIC: u32 = 1;
const FAST_RECONNECT_ENHANCED: u32 = 2;

fn remote_app_initial_program() -> windows_core::Result<(String, String)> {
    let mut buffer = vec![0; 32_768];
    // SAFETY: The buffer is writable and its length is passed by the generated binding.
    let length = unsafe { GetSystemDirectoryW(Some(&mut buffer)) };
    if length == 0 {
        return Err(windows_core::Error::from_thread());
    }
    let length = usize::try_from(length)
        .map_err(|_| windows_core::Error::new(E_INVALIDARG, "system directory length overflow"))?;
    if buffer.len() <= length {
        return Err(windows_core::Error::new(E_INVALIDARG, "system directory is too long"));
    }
    let directory = String::from_utf16(&buffer[..length])
        .map_err(|_| windows_core::Error::new(E_INVALIDARG, "system directory is not valid UTF-16"))?;
    let program = format!(r"{directory}\rdpinit.exe");
    Ok((directory, program))
}

fn property_string(value: &WTS_PROPERTY_VALUE) -> windows_core::Result<String> {
    if value.Type != WTS_VALUE_TYPE_STRING {
        return Err(windows_core::Error::new(E_INVALIDARG, "expected a string property"));
    }
    // SAFETY: Type identifies the active union member.
    let value = unsafe { value.u.strVal };
    let byte_length = usize::try_from(value.size)
        .map_err(|_| windows_core::Error::new(E_INVALIDARG, "property string length overflow"))?;
    if byte_length == 0 {
        return Ok(String::new());
    }
    if 0x8000 < byte_length || byte_length % 2 != 0 || value.pstrVal.is_null() {
        return Err(windows_core::Error::new(E_INVALIDARG, "invalid property string buffer"));
    }
    // SAFETY: Native initial-application properties describe their UTF-16 buffers in bytes.
    let buffer = unsafe { core::slice::from_raw_parts(value.pstrVal.0, byte_length / 2) };
    let length = buffer.iter().position(|v| *v == 0).unwrap_or(buffer.len());
    String::from_utf16(&buffer[..length]).map_err(|_| windows_core::Error::new(E_INVALIDARG, "invalid property UTF-16"))
}

fn write_property_string(property: &mut WTS_PROPERTY_VALUE, value: &str) -> windows_core::Result<()> {
    if property.Type != WTS_VALUE_TYPE_STRING {
        return Err(windows_core::Error::new(
            E_INVALIDARG,
            "expected a string output property",
        ));
    }
    // SAFETY: Type identifies the active union member.
    let buffer = unsafe { property.u.strVal };
    if 0x8000 < buffer.size || buffer.size % 2 != 0 || buffer.pstrVal.is_null() {
        return Err(windows_core::Error::new(E_INVALIDARG, "invalid string output buffer"));
    }
    let wide: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let capacity = usize::try_from(buffer.size)
        .map_err(|_| windows_core::Error::new(E_INVALIDARG, "property string capacity overflow"))?;
    if capacity < size_of_val(wide.as_slice()) {
        return Err(windows_core::Error::new(
            ERROR_INSUFFICIENT_BUFFER.to_hresult(),
            "initial-application output buffer is too small",
        ));
    }
    // SAFETY: The caller owns this writable buffer and its byte capacity includes the terminator.
    unsafe { core::ptr::copy_nonoverlapping(wide.as_ptr(), buffer.pstrVal.0, wide.len()) };
    Ok(())
}

fn session_has_user_token(session_id: u32) -> bool {
    let mut token = HANDLE::default();
    // SAFETY: `WTSQueryUserToken` writes a token handle into `token` on success.
    let result = unsafe { WTSQueryUserToken(session_id, &mut token) };
    if result.is_ok() {
        // SAFETY: close token handle returned by `WTSQueryUserToken`.
        unsafe {
            let _ = CloseHandle(token);
        }
        true
    } else {
        false
    }
}

fn session_has_process(session_id: u32, process_name: &str) -> bool {
    let mut process_info_ptr: *mut WTS_PROCESS_INFOW = core::ptr::null_mut();
    let mut process_count = 0u32;

    // SAFETY: WTSEnumerateProcessesW writes a buffer pointer/count pair on success.
    let enumerate_result = unsafe { WTSEnumerateProcessesW(None, 0, 1, &mut process_info_ptr, &mut process_count) };
    if enumerate_result.is_err() || process_info_ptr.is_null() || process_count == 0 {
        return false;
    }

    let mut found = false;

    if let Ok(process_count_usize) = usize::try_from(process_count) {
        // SAFETY: `process_info_ptr` references `process_count_usize` entries on success.
        let processes = unsafe { core::slice::from_raw_parts(process_info_ptr, process_count_usize) };

        for entry in processes {
            if entry.SessionId != session_id {
                continue;
            }

            // SAFETY: pProcessName is a NUL-terminated string owned by the WTS buffer.
            let name = unsafe { PCWSTR(entry.pProcessName.0).to_string() }.unwrap_or_default();
            if name.eq_ignore_ascii_case(process_name) {
                found = true;
                break;
            }
        }
    }

    // SAFETY: free memory allocated by WTSEnumerateProcessesW.
    unsafe {
        WTSFreeMemory(process_info_ptr.cast());
    }

    found
}

fn session_is_interactive(session_id: u32) -> bool {
    session_has_user_token(session_id) && session_has_process(session_id, "explorer.exe")
}

fn canonicalize_enumerated_session_id(session_id: u32) -> u32 {
    // Some hosts expose high-bit encoded values via WTSEnumerateSessionsW (for example 0x0001_0002),
    // while token/process APIs expect the canonical low-word session id (2).
    if session_id > u32::from(u16::MAX) {
        let low_word = session_id & u32::from(u16::MAX);
        if low_word != 0 && low_word != u32::from(u16::MAX) {
            return low_word;
        }

        // Values with an empty or all-ones low word (for example 0x0001_0000) are not
        // valid user sessions for token/process APIs.
        return u32::MAX;
    }

    session_id
}

fn lookup_account_sid_string(username: &str, domain: &str) -> windows_core::Result<String> {
    let username = username.trim();
    let domain = domain.trim();

    let account = if domain.is_empty() {
        username.to_owned()
    } else if username.contains('@') {
        // Already a UPN.
        username.to_owned()
    } else if domain.contains('.') {
        // AD often doesn't resolve DNS-style domains as down-level `DOMAIN\\user`.
        // Use UPN to make LookupAccountNameW succeed without relying on NetBIOS mapping.
        format!("{username}@{domain}")
    } else {
        format!("{domain}\\{username}")
    };

    let account_w = windows_core::HSTRING::from(account);

    let mut sid_size = 0u32;
    let mut domain_size = 0u32;
    let mut use_type = SID_NAME_USE(0);

    // First call to obtain required buffer sizes.
    // SAFETY: sizes/use_type are valid out-params.
    unsafe {
        let _ = LookupAccountNameW(
            None,
            PCWSTR(account_w.as_ptr()),
            None,
            &mut sid_size,
            None,
            &mut domain_size,
            &mut use_type,
        );
    }

    if sid_size == 0 {
        return Err(windows_core::Error::new(
            E_UNEXPECTED,
            "LookupAccountNameW returned zero SID size",
        ));
    }

    let sid_len = usize::try_from(sid_size)
        .map_err(|_| windows_core::Error::new(E_UNEXPECTED, "SID size does not fit in usize"))?;
    let domain_len = usize::try_from(domain_size)
        .map_err(|_| windows_core::Error::new(E_UNEXPECTED, "domain size does not fit in usize"))?;

    let mut sid_buf = vec![0u8; sid_len];
    let mut domain_buf = vec![0u16; domain_len];

    // SAFETY: buffers are allocated according to `sid_size` and `domain_size` returned above.
    unsafe {
        LookupAccountNameW(
            None,
            PCWSTR(account_w.as_ptr()),
            Some(PSID(sid_buf.as_mut_ptr().cast())),
            &mut sid_size,
            Some(PWSTR(domain_buf.as_mut_ptr())),
            &mut domain_size,
            &mut use_type,
        )?;
    }

    let mut sid_string = PWSTR::null();
    // SAFETY: `sid_buf` contains a valid SID and `sid_string` is a valid out-param.
    unsafe { ConvertSidToStringSidW(PSID(sid_buf.as_mut_ptr().cast()), &mut sid_string)? };

    // SAFETY: `sid_string` is NUL-terminated on success.
    let sid = unsafe { sid_string.to_string() }.map_err(|error| {
        windows_core::Error::new(
            E_UNEXPECTED,
            format!("failed to decode SID string returned by ConvertSidToStringSidW: {error}"),
        )
    })?;

    // SAFETY: ConvertSidToStringSidW allocates with LocalAlloc; free with LocalFree.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sid_string.0.cast())));
    }

    Ok(sid)
}

const DEBUG_LOG_PATH_ENV: &str = "IRONRDP_WTS_PROVIDER_DEBUG_LOG";
static DEBUG_LOG_FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();

fn normalize_winlogon_credentials(username: &str, domain: &str) -> (String, String) {
    let username = username.trim();
    let domain = domain.trim();

    if domain.is_empty() {
        if let Some((dom, user)) = username.split_once('\\') {
            let user = user.trim();
            let dom = dom.trim();
            if !user.is_empty() {
                return (user.to_owned(), dom.to_owned());
            }
        }
    }

    if username.contains('@') {
        // Preserve UPN formatting exactly as provided: the full UPN must stay
        // in the username field with an empty domain field.
        return (username.to_owned(), String::new());
    }

    (username.to_owned(), domain.to_owned())
}

fn debug_log_line(message: &str) {
    let Some(file) = DEBUG_LOG_FILE
        .get_or_init(|| {
            let path = std::env::var(DEBUG_LOG_PATH_ENV)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())?;
            let parent = std::path::Path::new(&path).parent()?;
            std::fs::create_dir_all(parent).ok()?;
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
                .map(Mutex::new)
        })
        .as_ref()
    else {
        return;
    };

    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();

    let line = format!("{timestamp_ms} {message}\n");
    let _ = file.lock().write_all(line.as_bytes());
}

fn load_idd_runtime_state() -> HashMap<String, String> {
    let Ok(content) = std::fs::read_to_string(IRONRDP_IDD_RUNTIME_STATE_FILE) else {
        return HashMap::new();
    };

    let mut values = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };

        let key = key.trim();
        if key.is_empty() {
            continue;
        }

        values.insert(key.to_owned(), value.trim().to_owned());
    }

    values
}

fn update_idd_runtime_state(updates: &[(&str, String)], source: &str) {
    let mut state = load_idd_runtime_state();
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();

    state.insert("updated_source".to_owned(), source.to_owned());
    state.insert("updated_at_unix_ms".to_owned(), timestamp_ms.to_string());

    for (key, value) in updates {
        state.insert((*key).to_owned(), value.clone());
    }

    let mut lines: Vec<String> = state.into_iter().map(|(key, value)| format!("{key}={value}")).collect();
    lines.sort();
    lines.push(String::new());

    if let Err(error) = std::fs::create_dir_all(IRONRDP_IDD_RUNTIME_ROOT) {
        debug_log_line(&format!(
            "update_idd_runtime_state source={source} create_dir_all('{}') failed: {error}",
            IRONRDP_IDD_RUNTIME_ROOT
        ));
        return;
    }

    match std::fs::write(IRONRDP_IDD_RUNTIME_STATE_FILE, lines.join("\n")) {
        Ok(()) => debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_IDD_RUNTIME_STATE_UPDATED source={source} path={} keys={}",
            IRONRDP_IDD_RUNTIME_STATE_FILE,
            updates.iter().map(|(key, _)| *key).collect::<Vec<_>>().join(",")
        )),
        Err(error) => debug_log_line(&format!(
            "update_idd_runtime_state source={source} write('{}') failed: {error}",
            IRONRDP_IDD_RUNTIME_STATE_FILE
        )),
    }
}

fn command_kind(command: &ProviderCommand) -> &'static str {
    match command {
        ProviderCommand::StartListen { .. } => "start_listen",
        ProviderCommand::StopListen { .. } => "stop_listen",
        ProviderCommand::WaitForIncoming { .. } => "wait_for_incoming",
        ProviderCommand::AcceptConnection { .. } => "accept_connection",
        ProviderCommand::CloseConnection { .. } => "close_connection",
        ProviderCommand::GetConnectionCredentials { .. } => "get_connection_credentials",
        ProviderCommand::GetRemoteAppSettings { .. } => "get_remote_app_settings",
        ProviderCommand::SetCaptureSessionId { .. } => "set_capture_session_id",
        ProviderCommand::NotifyIddDriverLoaded { .. } => "notify_idd_driver_loaded",
        ProviderCommand::OpenVirtualChannel { .. } => "open_virtual_channel",
    }
}

/// Ensures the WUDFRd (UMDF kernel reflector) service is running.
///
/// WUDFRd is required for UMDF IDD drivers to load. It auto-stops when idle,
/// so we start it proactively before TermService creates the SWD device.
/// This is called from `GetHardwareId` so WUDFRd is ready when PnP tries
/// to start the IDD driver.
fn ensure_wudfrd_running() {
    let service_name_wide: Vec<u16> = "WUDFRd\0".encode_utf16().collect();
    let service_name = PCWSTR(service_name_wide.as_ptr());

    // SAFETY: OpenSCManagerW with SC_MANAGER_CONNECT is safe for querying/starting services.
    let scm = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) };
    let Ok(scm) = scm else {
        let error = std::io::Error::last_os_error();
        debug_log_line(&format!("ensure_wudfrd_running: OpenSCManagerW failed: {error}"));
        // Fallback: try via sc.exe
        ensure_wudfrd_running_via_sc_exe();
        return;
    };

    // SAFETY: scm is a valid SC_HANDLE from OpenSCManagerW.
    let svc = unsafe { OpenServiceW(scm, service_name, SERVICE_QUERY_STATUS | SERVICE_START) };
    let Ok(svc) = svc else {
        let error = std::io::Error::last_os_error();
        debug_log_line(&format!("ensure_wudfrd_running: OpenServiceW(WUDFRd) failed: {error}"));
        unsafe {
            let _ = CloseServiceHandle(scm);
        }
        // Fallback: try via sc.exe
        ensure_wudfrd_running_via_sc_exe();
        return;
    };

    let mut status = SERVICE_STATUS::default();
    // SAFETY: svc is a valid service handle.
    let query_ok = unsafe { QueryServiceStatus(svc, &mut status) };
    if query_ok.is_err() {
        let error = std::io::Error::last_os_error();
        debug_log_line(&format!("ensure_wudfrd_running: QueryServiceStatus failed: {error}"));
        unsafe {
            let _ = CloseServiceHandle(svc);
            let _ = CloseServiceHandle(scm);
        }
        return;
    }

    if status.dwCurrentState == SERVICE_RUNNING || status.dwCurrentState == SERVICE_START_PENDING {
        debug_log_line(&format!(
            "ensure_wudfrd_running: WUDFRd already in state {:?}",
            status.dwCurrentState
        ));
        unsafe {
            let _ = CloseServiceHandle(svc);
            let _ = CloseServiceHandle(scm);
        }
        return;
    }

    debug_log_line(&format!(
        "ensure_wudfrd_running: WUDFRd in state {:?}, starting...",
        status.dwCurrentState
    ));

    // SAFETY: svc is a valid service handle with SERVICE_START access.
    let start_result = unsafe { StartServiceW(svc, None) };
    match start_result {
        Ok(()) => {
            debug_log_line("ensure_wudfrd_running: StartServiceW(WUDFRd) succeeded");

            // Wait up to 5 seconds for WUDFRd to reach RUNNING state.
            for attempt in 0..50 {
                thread::sleep(Duration::from_millis(100));
                let mut poll_status = SERVICE_STATUS::default();
                // SAFETY: svc is still a valid handle.
                if unsafe { QueryServiceStatus(svc, &mut poll_status) }.is_ok()
                    && poll_status.dwCurrentState == SERVICE_RUNNING
                {
                    debug_log_line(&format!(
                        "ensure_wudfrd_running: WUDFRd running after {} ms",
                        (attempt + 1) * 100
                    ));
                    break;
                }
            }
        }
        Err(error) => {
            debug_log_line(&format!("ensure_wudfrd_running: StartServiceW failed: {error}"));
        }
    }

    unsafe {
        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);
    }
}

/// Fallback: start WUDFRd by spawning `sc.exe start WUDFRd`.
/// Used when the SCM API fails (e.g., insufficient permissions for NETWORK SERVICE).
fn ensure_wudfrd_running_via_sc_exe() {
    debug_log_line("ensure_wudfrd_running_via_sc_exe: attempting sc.exe start WUDFRd");
    match std::process::Command::new("sc.exe").args(["start", "WUDFRd"]).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug_log_line(&format!(
                "ensure_wudfrd_running_via_sc_exe: exit={} stdout={} stderr={}",
                output.status.code().unwrap_or(-1),
                stdout.trim(),
                stderr.trim()
            ));
            if output.status.success() {
                // Wait up to 5 seconds for WUDFRd to start.
                for attempt in 0..50 {
                    thread::sleep(Duration::from_millis(100));
                    if let Ok(check) = std::process::Command::new("sc.exe").args(["query", "WUDFRd"]).output() {
                        let query_out = String::from_utf8_lossy(&check.stdout);
                        if query_out.contains("RUNNING") {
                            debug_log_line(&format!(
                                "ensure_wudfrd_running_via_sc_exe: WUDFRd running after {} ms",
                                (attempt + 1) * 100
                            ));
                            return;
                        }
                    }
                }
                debug_log_line("ensure_wudfrd_running_via_sc_exe: WUDFRd did not reach RUNNING in 5s");
            }
        }
        Err(error) => {
            debug_log_line(&format!(
                "ensure_wudfrd_running_via_sc_exe: failed to spawn sc.exe: {error}"
            ));
        }
    }
}

fn event_kind(event: &ServiceEvent) -> &'static str {
    match event {
        ServiceEvent::Ack => "ack",
        ServiceEvent::ListenerStarted { .. } => "listener_started",
        ServiceEvent::ListenerStopped { .. } => "listener_stopped",
        ServiceEvent::IncomingConnection { .. } => "incoming_connection",
        ServiceEvent::NoIncoming => "no_incoming",
        ServiceEvent::ConnectionReady { .. } => "connection_ready",
        ServiceEvent::RemoteAppSettings { .. } => "remote_app_settings",
        ServiceEvent::ConnectionBroken { .. } => "connection_broken",
        ServiceEvent::ConnectionCredentials { .. } => "connection_credentials",
        ServiceEvent::NoCredentials { .. } => "no_credentials",
        ServiceEvent::VirtualChannelReady { .. } => "virtual_channel_ready",
        ServiceEvent::Error { .. } => "error",
    }
}

pub const IRONRDP_PROTOCOL_MANAGER_CLSID: GUID = GUID::from_u128(0x89c7ed1e_25e5_4b15_8f52_ae6df4a5ceaf);
pub const IRONRDP_PROTOCOL_MANAGER_CLSID_STR: &str = "{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}";

static SERVER_LOCK_COUNT: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_OBJECT_COUNT: AtomicUsize = AtomicUsize::new(0);

const VIRTUAL_CHANNEL_PIPE_BRIDGE_ENV: &str = "IRONRDP_WTS_VC_BRIDGE_PIPE_PREFIX";
const VIRTUAL_CHANNEL_PIPE_BRIDGE_QUEUE_SIZE: usize = 200;
const VIRTUAL_CHANNEL_PIPE_BRIDGE_RECONNECT_DELAY: Duration = Duration::from_millis(500);
const VIRTUAL_CHANNEL_PIPE_BRIDGE_SEND_TIMEOUT: Duration = Duration::from_millis(100);
const CONTROL_PIPE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE: usize = 1024 * 1024;

type SharedVirtualChannelBridgeHandler = Arc<dyn VirtualChannelBridgeHandler>;

static VIRTUAL_CHANNEL_BRIDGE_HANDLER: OnceLock<Mutex<Option<SharedVirtualChannelBridgeHandler>>> = OnceLock::new();

struct ComInitGuard {
    initialized: bool,
}

impl ComInitGuard {
    fn new() -> Self {
        // SAFETY: COM initialization is per-thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        match hr.ok() {
            Ok(()) => Self { initialized: true },
            Err(error) => {
                debug_log_line(&format!("CoInitializeEx failed in StartListen path: {error}"));
                Self { initialized: false }
            }
        }
    }
}

impl Drop for ComInitGuard {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: paired with a successful CoInitializeEx in this guard.
            unsafe { CoUninitialize() };
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualChannelRouteKind {
    Unknown,
    IronRdpStatic,
    IronRdpDynamicBackbone,
    IronRdpDynamicEndpoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualChannelBridgeEndpoint {
    pub endpoint_name: String,
    pub static_channel: bool,
    pub route_kind: VirtualChannelRouteKind,
}

#[derive(Clone)]
pub struct VirtualChannelBridgeTx {
    endpoint: VirtualChannelBridgeEndpoint,
    outbound_tx: mpsc::SyncSender<Vec<u8>>,
}

impl VirtualChannelBridgeTx {
    pub fn endpoint(&self) -> &VirtualChannelBridgeEndpoint {
        &self.endpoint
    }

    pub fn send(&self, payload: Vec<u8>) -> windows_core::Result<()> {
        self.outbound_tx
            .send(payload)
            .map_err(|_| windows_core::Error::new(E_UNEXPECTED, "virtual channel bridge sender is closed"))
    }
}

pub trait VirtualChannelBridgeHandler: Send + Sync {
    fn on_channel_opened(&self, endpoint: &VirtualChannelBridgeEndpoint, tx: VirtualChannelBridgeTx);

    fn on_channel_data(&self, endpoint: &VirtualChannelBridgeEndpoint, data: &[u8]);

    fn on_channel_closed(&self, endpoint: &VirtualChannelBridgeEndpoint);
}

pub fn set_virtual_channel_bridge_handler(handler: Option<Arc<dyn VirtualChannelBridgeHandler>>) {
    *virtual_channel_bridge_handler_slot().lock() = handler;
}

fn virtual_channel_bridge_handler_slot() -> &'static Mutex<Option<SharedVirtualChannelBridgeHandler>> {
    VIRTUAL_CHANNEL_BRIDGE_HANDLER.get_or_init(|| Mutex::new(None))
}

fn get_virtual_channel_bridge_handler() -> Option<SharedVirtualChannelBridgeHandler> {
    virtual_channel_bridge_handler_slot().lock().clone()
}

#[derive(Debug)]
struct ListenerWorker {
    stop_tx: mpsc::Sender<()>,
    join_handle: thread::JoinHandle<()>,
}

#[derive(Debug, Clone)]
struct ProviderControlBridge {
    pipe_name: Option<String>,
    optional_connection: bool,
}

#[derive(Debug, Clone)]
struct IncomingConnection {
    connection_id: u32,
    peer_addr: Option<String>,
}

enum WaitForIncomingEvent {
    Incoming(IncomingConnection),
    ConnectionBroken { connection_id: u32, reason: String },
}

impl ProviderControlBridge {
    fn from_env() -> Self {
        if let Some(pipe_name) = resolve_pipe_name_from_env() {
            return Self {
                pipe_name: Some(pipe_name),
                optional_connection: false,
            };
        }

        Self {
            pipe_name: Some(default_pipe_name()),
            optional_connection: true,
        }
    }

    fn start_listen(&self, listener_name: &str) -> windows_core::Result<bool> {
        // Use retries: if the pipe is momentarily busy (e.g., occupied by an orphaned worker
        // thread from a previous TermService session still polling WaitForIncoming), we want
        // to keep retrying rather than returning an error that causes a "Catastrophic failure".
        let Some(event) = self.send_command_retried(&ProviderCommand::StartListen {
            listener_name: listener_name.to_owned(),
        })?
        else {
            // When running under TermService, the companion service is typically started via a
            // Scheduled Task. On boot, TermService can call StartListen before that task has
            // created the named pipe. If we return "disabled" here, we never enter the
            // WaitForIncoming polling loop, so the companion never auto-starts its TCP listener.
            //
            // Treat the bridge as enabled and let the worker thread keep polling; once the pipe
            // becomes available, WaitForIncoming will auto-start the TCP listener.
            debug_log_line(&format!(
                "ProviderControlBridge::start_listen control pipe not available yet; enabling deferred StartListen listener_name={listener_name}",
            ));
            return Ok(true);
        };

        match event {
            ServiceEvent::ListenerStarted {
                listener_name: started_listener,
            } if started_listener == listener_name => Ok(true),
            ServiceEvent::Ack => Ok(true),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            other => Err(windows_core::Error::new(
                E_UNEXPECTED,
                format!("unexpected service event on start listen: {other:?}"),
            )),
        }
    }

    fn stop_listen(&self, listener_name: &str) -> windows_core::Result<()> {
        let Some(event) = self.send_command(&ProviderCommand::StopListen {
            listener_name: listener_name.to_owned(),
        })?
        else {
            return Ok(());
        };

        match event {
            ServiceEvent::ListenerStopped {
                listener_name: stopped_listener,
            } if stopped_listener == listener_name => Ok(()),
            ServiceEvent::Ack => Ok(()),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            other => Err(windows_core::Error::new(
                E_UNEXPECTED,
                format!("unexpected service event on stop listen: {other:?}"),
            )),
        }
    }

    fn set_capture_session_id_retried(
        &self,
        connection_id: u32,
        session_id: u32,
        source: &'static str,
    ) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_SEND source={source} connection_id={connection_id} session_id={session_id}",
        ));

        let Some(event) = self.send_command_retried(&ProviderCommand::SetCaptureSessionId {
            connection_id,
            session_id,
        })?
        else {
            debug_log_line(&format!(
                "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_PIPE_UNAVAILABLE source={source} connection_id={connection_id} session_id={session_id}",
            ));
            return Ok(());
        };

        match event {
            ServiceEvent::Ack => {
                debug_log_line(&format!(
                    "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ACK source={source} connection_id={connection_id} session_id={session_id}",
                ));
                Ok(())
            }
            ServiceEvent::Error { message } => {
                debug_log_line(&format!(
                    "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ERROR source={source} connection_id={connection_id} session_id={session_id} message={message}",
                ));
                Err(windows_core::Error::new(E_UNEXPECTED, message))
            }
            other => {
                debug_log_line(&format!(
                    "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_UNEXPECTED source={source} connection_id={connection_id} session_id={session_id} event={other:?}",
                ));
                Err(windows_core::Error::new(
                    E_UNEXPECTED,
                    format!("unexpected service event on set capture session id (retried): {other:?}"),
                ))
            }
        }
    }

    fn set_capture_session_id_async(&self, connection_id: u32, session_id: u32, source: &'static str) {
        let bridge = self.clone();
        if let Err(error) = thread::Builder::new()
            .name(format!("irdp-set-capture-{connection_id}"))
            .spawn(move || {
                debug_log_line(&format!(
                    "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ASYNC_BEGIN source={source} connection_id={connection_id} session_id={session_id}",
                ));
                if let Err(error) = bridge.set_capture_session_id_retried(connection_id, session_id, source) {
                    debug_log_line(&format!(
                        "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ASYNC_ERROR source={source} connection_id={connection_id} session_id={session_id} error={error}",
                    ));
                } else {
                    debug_log_line(&format!(
                        "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ASYNC_OK source={source} connection_id={connection_id} session_id={session_id}",
                    ));
                }
            })
        {
            debug_log_line(&format!(
                "SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ASYNC_SPAWN_ERROR source={source} connection_id={connection_id} session_id={session_id} error={error}",
            ));
        }
    }

    fn notify_idd_driver_loaded(&self, session_id: u32) -> windows_core::Result<()> {
        let Some(event) = self.send_command(&ProviderCommand::NotifyIddDriverLoaded { session_id })? else {
            return Ok(());
        };

        match event {
            ServiceEvent::Ack => Ok(()),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            _ => Ok(()),
        }
    }

    fn accept_connection(&self, connection_id: u32) -> windows_core::Result<()> {
        let Some(event) = self.send_command_retried(&ProviderCommand::AcceptConnection { connection_id })? else {
            return Ok(());
        };

        match event {
            ServiceEvent::ConnectionReady {
                connection_id: ready_connection_id,
            } if ready_connection_id == connection_id => Ok(()),
            ServiceEvent::Ack => Ok(()),
            ServiceEvent::Error { message } => {
                debug_log_line(&format!(
                    "accept_connection error connection_id={connection_id} message={message}"
                ));
                Err(windows_core::Error::new(E_UNEXPECTED, message))
            }
            other => {
                debug_log_line(&format!(
                    "accept_connection unexpected_event connection_id={connection_id} event={other:?}"
                ));
                Err(windows_core::Error::new(
                    E_UNEXPECTED,
                    format!("unexpected service event on accept connection: {other:?}"),
                ))
            }
        }
    }

    fn close_connection(&self, connection_id: u32) -> windows_core::Result<()> {
        let Some(event) = self.send_command_retried(&ProviderCommand::CloseConnection { connection_id })? else {
            return Ok(());
        };

        match event {
            ServiceEvent::Ack => Ok(()),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            _ => Ok(()),
        }
    }

    /// Retrieve the client-derived CredSSP plaintext credentials for a connection from the companion service.
    ///
    /// Polls up to ~15 seconds waiting for the NLA/CredSSP handshake to complete.
    /// Tolerates transient pipe I/O errors (the companion service may restart its pipe server).
    /// Returns `(username, domain, password)` on success, or `None` when no credentials are
    /// available after the timeout.
    fn get_connection_credentials(&self, connection_id: u32) -> windows_core::Result<Option<(String, String, String)>> {
        let mut last_pipe_error: Option<windows_core::Error> = None;

        for poll in 0..60 {
            if poll > 0 {
                thread::sleep(Duration::from_millis(250));
            }

            let event = match self.send_command_retried(&ProviderCommand::GetConnectionCredentials { connection_id }) {
                Ok(Some(event)) => event,
                Ok(None) => {
                    // Pipe transiently unavailable (companion pipe server between iterations).
                    // Treat this like NoCredentials: continue polling rather than giving up.
                    continue;
                }
                Err(error) => {
                    if poll == 0 {
                        debug_log_line(&format!(
                            "get_connection_credentials pipe error connection_id={connection_id}; retrying... error={error}"
                        ));
                    }
                    last_pipe_error = Some(error);
                    continue;
                }
            };

            match event {
                ServiceEvent::ConnectionCredentials {
                    connection_id: cid,
                    username,
                    domain,
                    password,
                } if cid == connection_id => {
                    debug_log_line(&format!(
                        "get_connection_credentials ok connection_id={connection_id} poll={poll} username={username} domain={domain}"
                    ));
                    return Ok(Some((username, domain, password)));
                }
                ServiceEvent::NoCredentials { .. } => {
                    if poll == 0 {
                        debug_log_line(&format!(
                            "get_connection_credentials no_credentials yet connection_id={connection_id}; polling..."
                        ));
                    }
                    continue;
                }
                ServiceEvent::Error { message } => {
                    debug_log_line(&format!(
                        "get_connection_credentials service_error connection_id={connection_id} message={message}"
                    ));
                    last_pipe_error = Some(windows_core::Error::new(E_UNEXPECTED, message));
                    continue;
                }
                other => {
                    warn!(
                        connection_id,
                        ?other,
                        "Unexpected service event on get_connection_credentials"
                    );
                    return Ok(None);
                }
            }
        }

        debug_log_line(&format!(
            "get_connection_credentials timed out connection_id={connection_id} last_error={last_pipe_error:?}"
        ));
        Ok(None)
    }

    fn remote_app_settings(&self, connection_id: u32) -> windows_core::Result<Option<RemoteAppSettings>> {
        let event = with_provider_identity(|| {
            self.send_command_retried(&ProviderCommand::GetRemoteAppSettings { connection_id })
        })?;
        match event {
            Some(ServiceEvent::RemoteAppSettings {
                connection_id: returned,
                settings,
            }) if returned == connection_id => Ok(settings),
            Some(ServiceEvent::Error { message }) => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            _ => Err(windows_core::Error::new(
                E_UNEXPECTED,
                "invalid RemoteApp settings response",
            )),
        }
    }

    fn wait_for_incoming(
        &self,
        listener_name: &str,
        timeout_ms: u32,
    ) -> windows_core::Result<Option<WaitForIncomingEvent>> {
        let Some(event) = self.send_command_retried(&ProviderCommand::WaitForIncoming {
            listener_name: listener_name.to_owned(),
            timeout_ms,
        })?
        else {
            return Ok(None);
        };

        match event {
            ServiceEvent::IncomingConnection {
                listener_name: service_listener_name,
                connection_id,
                peer_addr,
            } => {
                if service_listener_name != listener_name {
                    return Err(windows_core::Error::new(
                        E_UNEXPECTED,
                        format!(
                            "incoming connection listener mismatch: expected {listener_name} got {service_listener_name}"
                        ),
                    ));
                }

                Ok(Some(WaitForIncomingEvent::Incoming(IncomingConnection {
                    connection_id,
                    peer_addr,
                })))
            }
            ServiceEvent::ConnectionBroken { connection_id, reason } => {
                Ok(Some(WaitForIncomingEvent::ConnectionBroken { connection_id, reason }))
            }
            ServiceEvent::NoIncoming | ServiceEvent::Ack => Ok(None),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            other => Err(windows_core::Error::new(
                E_UNEXPECTED,
                format!("unexpected service event on wait incoming: {other:?}"),
            )),
        }
    }

    fn send_command(&self, command: &ProviderCommand) -> windows_core::Result<Option<ServiceEvent>> {
        self.send_command_with_retries(command, 1)
    }

    fn send_command_retried(&self, command: &ProviderCommand) -> windows_core::Result<Option<ServiceEvent>> {
        self.send_command_with_retries(command, 10)
    }

    fn send_command_with_retries(
        &self,
        command: &ProviderCommand,
        max_attempts: u32,
    ) -> windows_core::Result<Option<ServiceEvent>> {
        let Some(pipe_name) = self.pipe_name.as_ref() else {
            return Ok(None);
        };

        let full_pipe_name = pipe_path(pipe_name);

        let mut last_error = None;
        for attempt in 0..max_attempts {
            if attempt > 0 {
                thread::sleep(Duration::from_millis(50 + u64::from(attempt) * 30));
            }

            debug_log_line(&format!(
                "ProviderControlBridge::send_command open pipe={} command={}{}",
                full_pipe_name,
                command_kind(command),
                if attempt > 0 {
                    format!(" retry={attempt}")
                } else {
                    String::new()
                }
            ));

            let pipe_result = OpenOptions::new().read(true).write(true).open(&full_pipe_name);

            let mut pipe = match pipe_result {
                Ok(pipe) => pipe,
                Err(error)
                    if is_optional_control_pipe_error(&error)
                        && (self.optional_connection
                            || matches!(
                                command,
                                ProviderCommand::StartListen { .. }
                                    | ProviderCommand::WaitForIncoming { .. }
                                    | ProviderCommand::NotifyIddDriverLoaded { .. }
                            )) =>
                {
                    debug!(
                        %error,
                        pipe = %full_pipe_name,
                        command = command_kind(command),
                        "Companion control pipe not available yet; deferring provider IPC"
                    );
                    return Ok(None);
                }
                Err(error) if error.raw_os_error() == Some(231) && attempt + 1 < max_attempts => {
                    debug_log_line(&format!(
                        "ProviderControlBridge::send_command pipe_busy pipe={} command={} attempt={}",
                        full_pipe_name,
                        command_kind(command),
                        attempt
                    ));
                    last_error = Some(error);
                    continue;
                }
                Err(error) => {
                    debug_log_line(&format!(
                        "ProviderControlBridge::send_command open_failed pipe={} command={} error={}",
                        full_pipe_name,
                        command_kind(command),
                        error
                    ));
                    return Err(io_error_to_windows_error(error, "failed to connect to control pipe"));
                }
            };

            // The companion impersonates the client before reading, which only works once data is
            // in the pipe, so the command is written here; a small frame never blocks on the pipe buffer.
            write_json_message(&mut pipe, command)
                .map_err(|error| io_error_to_windows_error(error, "failed to send control command"))?;

            // TermService calls the provider while holding its own locks, and the companion may be
            // blocked in a WTS RPC into TermService. Waiting unboundedly for the reply would deadlock
            // both processes, so the read runs on a worker thread and is abandoned after a deadline.
            // An abandoned worker exits once the companion answers or the pipe breaks.
            let response_timeout = match command {
                ProviderCommand::WaitForIncoming { timeout_ms, .. } => {
                    Duration::from_millis(u64::from(*timeout_ms)) + CONTROL_PIPE_RESPONSE_TIMEOUT
                }
                _ => CONTROL_PIPE_RESPONSE_TIMEOUT,
            };
            let (response_tx, response_rx) = mpsc::sync_channel(1);
            thread::Builder::new()
                .name("ironrdp-control-pipe-read".to_owned())
                .spawn(move || {
                    let result = read_json_message::<ServiceEvent>(&mut pipe, DEFAULT_MAX_FRAME_SIZE)
                        .map_err(|error| io_error_to_windows_error(error, "failed to read control response"));
                    let _ = response_tx.send(result);
                })
                .map_err(|error| io_error_to_windows_error(error, "failed to spawn control pipe read thread"))?;

            let event = match response_rx.recv_timeout(response_timeout) {
                Ok(result) => result?,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    debug_log_line(&format!(
                        "ProviderControlBridge::send_command timed_out command={} timeout_ms={}",
                        command_kind(command),
                        response_timeout.as_millis()
                    ));
                    return Err(windows_core::Error::new(
                        HRESULT::from_win32(ERROR_SEM_TIMEOUT.0),
                        "companion did not answer the control command in time",
                    ));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(windows_core::Error::new(
                        E_UNEXPECTED,
                        "control pipe read thread ended without a response",
                    ));
                }
            };

            debug_log_line(&format!(
                "ProviderControlBridge::send_command ok command={} event={}",
                command_kind(command),
                event_kind(&event)
            ));

            return Ok(Some(event));
        }

        let error = last_error.unwrap_or_else(|| std::io::Error::other("max retries exceeded"));
        Err(io_error_to_windows_error(
            error,
            "failed to connect to control pipe after retries",
        ))
    }
}

fn is_optional_control_pipe_error(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;

    matches!(
        error.kind(),
        ErrorKind::NotFound
            | ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::TimedOut
            | ErrorKind::BrokenPipe
            | ErrorKind::WouldBlock
    )
}

/// TermService session arbitration treats only `0xFFFFFFFF` as "no limit";
/// a zero `MaxProtocolListenerConnectionCount` rejects every non-admin logon
/// with `ERROR_CTX_LICENSE_NOT_AVAILABLE` ("Exceed listener max instance count").
const UNLIMITED_LISTENER_CONNECTIONS: u32 = u32::MAX;

fn default_connection_settings(listener_name: &str) -> WRDS_CONNECTION_SETTINGS {
    // SAFETY: `WRDS_CONNECTION_SETTINGS` is a plain C struct; a zeroed value is a valid baseline.
    let mut settings: WRDS_CONNECTION_SETTINGS = unsafe { core::mem::zeroed() };
    settings.WRdsConnectionSettingLevel = WRDS_CONNECTION_SETTING_LEVEL(1);
    // SAFETY: WRDS_CONNECTION_SETTINGS contains a union; accessing the level-1 view is valid because we set the level.
    unsafe {
        // A/B mode: advertise a non-RDP protocol type.
        // WTS_PROTOCOL_TYPE_* values in the Windows SDK (WtsApi32.h):
        //   CONSOLE = 0, ICA = 1, RDP = 2
        let s1 = &mut settings.WRdsConnectionSetting.WRdsConnectionSettings1;
        s1.ProtocolType = WTS_PROTOCOL_TYPE_NON_RDP;
        copy_wide(&mut s1.ProtocolName, "RDP");

        // Do not advertise auto-logon at the listener level. For NLA/CredSSP, the per-connection
        // callbacks are responsible for exposing user credentials only after the handshake has
        // actually completed.
        s1.fInheritAutoLogon = false;
        s1.fUsingSavedCreds = false;
        s1.fPromptForPassword = false;
        s1.fEnableWindowsKey = true;
        s1.fDisableCtrlAltDel = true;
        s1.fMouse = true;

        let ls = &mut s1.WRdsListenerSettings;
        ls.WRdsListenerSettingLevel = WRDS_LISTENER_SETTING_LEVEL(1);
        ls.WRdsListenerSetting
            .WRdsListenerSettings1
            .MaxProtocolListenerConnectionCount = UNLIMITED_LISTENER_CONNECTIONS;
        ls.WRdsListenerSetting.WRdsListenerSettings1.pSecurityDescriptor = core::ptr::null_mut();
    }

    debug_log_line(&format!(
        "default_connection_settings listener_name={listener_name} (matching MS sample: NULL SD)",
    ));

    settings
}

fn enable_connection_autologon(settings: &mut WRDS_CONNECTION_SETTINGS) {
    // SAFETY: `settings` is initialized for the level-1 view by `default_connection_settings`.
    unsafe {
        let s1 = &mut settings.WRdsConnectionSetting.WRdsConnectionSettings1;
        s1.fInheritAutoLogon = true;
        s1.fUsingSavedCreds = true;
        s1.fPromptForPassword = false;
        // TermService limits these fields to 20/17/14 UTF-16 units through legacy USERCONFIGW buffers.
        // Keep credentials in the full-width GetClientData and GetUserCredentials callbacks instead.
    }
}

pub fn create_protocol_manager_com() -> IWRdsProtocolManager {
    install_default_virtual_channel_bridge_handler_from_env();
    ComProtocolManager::new().into()
}

fn install_default_virtual_channel_bridge_handler_from_env() {
    if get_virtual_channel_bridge_handler().is_some() {
        return;
    }

    let Some(pipe_prefix) = std::env::var(VIRTUAL_CHANNEL_PIPE_BRIDGE_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return;
    };

    info!(
        pipe_prefix = %pipe_prefix,
        "Installing default virtual channel named-pipe bridge handler"
    );
    set_virtual_channel_bridge_handler(Some(Arc::new(NamedPipeBridgeHandler::new(pipe_prefix))));
}

struct NamedPipeBridgeHandler {
    pipe_prefix: String,
    workers: Mutex<HashMap<String, NamedPipeBridgeWorker>>,
    bridge_txs: Mutex<HashMap<String, VirtualChannelBridgeTx>>,
}

impl NamedPipeBridgeHandler {
    fn new(pipe_prefix: String) -> Self {
        Self {
            pipe_prefix,
            workers: Mutex::new(HashMap::new()),
            bridge_txs: Mutex::new(HashMap::new()),
        }
    }

    fn restart_worker(
        &self,
        endpoint: &VirtualChannelBridgeEndpoint,
        tx: VirtualChannelBridgeTx,
    ) -> mpsc::SyncSender<Vec<u8>> {
        let endpoint_key = bridge_endpoint_key(endpoint);
        let pipe_path = bridge_pipe_path(&self.pipe_prefix, endpoint);
        let endpoint_for_worker = endpoint.clone();

        let (to_pipe_tx, to_pipe_rx) = mpsc::sync_channel(VIRTUAL_CHANNEL_PIPE_BRIDGE_QUEUE_SIZE);
        let (stop_tx, stop_rx) = mpsc::channel();

        let join_handle = thread::spawn(move || {
            run_named_pipe_bridge_worker(endpoint_for_worker, pipe_path, tx, to_pipe_rx, stop_rx)
        });

        let mut workers = self.workers.lock();
        if let Some(previous) = workers.insert(
            endpoint_key,
            NamedPipeBridgeWorker {
                stop_tx,
                to_pipe_tx: to_pipe_tx.clone(),
                join_handle,
            },
        ) {
            previous.stop_and_join();
        }

        to_pipe_tx
    }

    fn stop_worker(&self, endpoint: &VirtualChannelBridgeEndpoint) {
        let endpoint_key = bridge_endpoint_key(endpoint);
        if let Some(worker) = self.workers.lock().remove(&endpoint_key) {
            worker.stop_and_join();
        }
    }

    fn get_bridge_tx(&self, endpoint: &VirtualChannelBridgeEndpoint) -> Option<VirtualChannelBridgeTx> {
        self.bridge_txs.lock().get(&bridge_endpoint_key(endpoint)).cloned()
    }
}

impl VirtualChannelBridgeHandler for NamedPipeBridgeHandler {
    fn on_channel_opened(&self, endpoint: &VirtualChannelBridgeEndpoint, tx: VirtualChannelBridgeTx) {
        self.bridge_txs.lock().insert(bridge_endpoint_key(endpoint), tx.clone());
        let _ = self.restart_worker(endpoint, tx);
    }

    fn on_channel_data(&self, endpoint: &VirtualChannelBridgeEndpoint, data: &[u8]) {
        let endpoint_key = bridge_endpoint_key(endpoint);
        let worker_tx = {
            let workers = self.workers.lock();
            workers.get(&endpoint_key).map(|worker| worker.to_pipe_tx.clone())
        };

        let tx = if let Some(tx) = worker_tx {
            tx
        } else {
            let Some(bridge_tx) = self.get_bridge_tx(endpoint) else {
                warn!(
                    endpoint = %endpoint.endpoint_name,
                    "Named-pipe bridge worker unavailable and bridge tx is not registered"
                );
                return;
            };

            self.restart_worker(endpoint, bridge_tx)
        };

        let result = tx.send(data.to_vec());

        if result.is_err() {
            warn!(
                endpoint = %endpoint.endpoint_name,
                "Failed to queue payload into named-pipe bridge worker"
            );
        }
    }

    fn on_channel_closed(&self, endpoint: &VirtualChannelBridgeEndpoint) {
        self.bridge_txs.lock().remove(&bridge_endpoint_key(endpoint));
        self.stop_worker(endpoint);
    }
}

struct NamedPipeBridgeWorker {
    stop_tx: mpsc::Sender<()>,
    to_pipe_tx: mpsc::SyncSender<Vec<u8>>,
    join_handle: thread::JoinHandle<()>,
}

impl NamedPipeBridgeWorker {
    fn stop_and_join(self) {
        if let Err(error) = self.stop_tx.send(()) {
            warn!(%error, "Failed to stop named-pipe bridge worker");
        }

        if let Err(error) = self.join_handle.join() {
            warn!(?error, "Named-pipe bridge worker thread panicked");
        }
    }
}

fn run_named_pipe_bridge_worker(
    endpoint: VirtualChannelBridgeEndpoint,
    pipe_path: String,
    to_channel_tx: VirtualChannelBridgeTx,
    to_pipe_rx: mpsc::Receiver<Vec<u8>>,
    stop_rx: mpsc::Receiver<()>,
) {
    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        let open_result = OpenOptions::new().read(true).write(true).open(&pipe_path);
        let mut pipe = match open_result {
            Ok(pipe) => {
                info!(endpoint = %endpoint.endpoint_name, pipe = %pipe_path, "Connected named-pipe bridge worker");
                pipe
            }
            Err(error) => {
                debug!(
                    endpoint = %endpoint.endpoint_name,
                    pipe = %pipe_path,
                    %error,
                    "Named-pipe bridge worker waiting for server"
                );
                thread::sleep(VIRTUAL_CHANNEL_PIPE_BRIDGE_RECONNECT_DELAY);
                continue;
            }
        };

        let mut from_pipe_buffer = Vec::with_capacity(4096);

        loop {
            if stop_rx.try_recv().is_ok() {
                return;
            }

            let mut write_failed = false;

            match to_pipe_rx.recv_timeout(VIRTUAL_CHANNEL_PIPE_BRIDGE_SEND_TIMEOUT) {
                Ok(payload) => {
                    if let Err(error) = write_length_prefixed(&mut pipe, &payload) {
                        warn!(
                            endpoint = %endpoint.endpoint_name,
                            pipe = %pipe_path,
                            %error,
                            "Named-pipe bridge write failed; reconnecting"
                        );
                        write_failed = true;
                    }

                    if write_failed {
                        break;
                    }

                    while let Ok(queued_payload) = to_pipe_rx.try_recv() {
                        if let Err(error) = write_length_prefixed(&mut pipe, &queued_payload) {
                            warn!(
                                endpoint = %endpoint.endpoint_name,
                                pipe = %pipe_path,
                                %error,
                                "Named-pipe bridge write failed while draining queue; reconnecting"
                            );
                            write_failed = true;
                            break;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }

            if write_failed {
                break;
            }

            if let Err(error) =
                pump_named_pipe_inbound_frames(&mut pipe, &endpoint, &to_channel_tx, &mut from_pipe_buffer)
            {
                warn!(
                    endpoint = %endpoint.endpoint_name,
                    pipe = %pipe_path,
                    %error,
                    "Named-pipe bridge read failed; reconnecting"
                );
                break;
            }
        }
    }
}

fn pump_named_pipe_inbound_frames(
    pipe: &mut File,
    endpoint: &VirtualChannelBridgeEndpoint,
    to_channel_tx: &VirtualChannelBridgeTx,
    read_buffer: &mut Vec<u8>,
) -> std::io::Result<()> {
    let mut chunk = [0u8; 8192];

    loop {
        let available = named_pipe_available_bytes(pipe)?;
        if available == 0 {
            break;
        }

        let read_len = usize::try_from(available).unwrap_or(usize::MAX).min(chunk.len());

        let read_count = match pipe.read(&mut chunk[..read_len]) {
            Ok(0) => {
                return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "named pipe closed"));
            }
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };

        read_buffer.extend_from_slice(&chunk[..read_count]);
        drain_length_prefixed_pipe_frames(endpoint, to_channel_tx, read_buffer)?;
    }

    Ok(())
}

fn named_pipe_available_bytes(pipe: &File) -> std::io::Result<u32> {
    let mut total_bytes_available = 0u32;

    // SAFETY: `pipe.as_raw_handle()` returns a live OS handle for this file. We only ask
    // for the available byte count and provide a valid out-pointer.
    unsafe {
        PeekNamedPipe(
            HANDLE(pipe.as_raw_handle()),
            None,
            0,
            None,
            Some(&mut total_bytes_available),
            None,
        )
    }
    .map_err(|error| {
        let kind = if error.code() == HRESULT::from_win32(ERROR_BROKEN_PIPE.0)
            || error.code() == HRESULT::from_win32(ERROR_NO_DATA.0)
        {
            std::io::ErrorKind::BrokenPipe
        } else {
            std::io::ErrorKind::Other
        };

        std::io::Error::new(kind, format!("failed to peek named pipe: {error}"))
    })?;

    Ok(total_bytes_available)
}

fn drain_length_prefixed_pipe_frames(
    endpoint: &VirtualChannelBridgeEndpoint,
    to_channel_tx: &VirtualChannelBridgeTx,
    read_buffer: &mut Vec<u8>,
) -> std::io::Result<()> {
    let mut frame_offset = 0usize;

    while read_buffer.len().saturating_sub(frame_offset) >= 4 {
        let frame_len_u32 = u32::from_le_bytes([
            read_buffer[frame_offset],
            read_buffer[frame_offset + 1],
            read_buffer[frame_offset + 2],
            read_buffer[frame_offset + 3],
        ]);
        let frame_len = usize::try_from(frame_len_u32).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "named-pipe bridge frame length does not fit in usize",
            )
        })?;

        if frame_len > VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "named-pipe bridge frame length exceeds limit (len={frame_len}, max={VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE})"
                ),
            ));
        }

        let frame_total_len = 4usize + frame_len;
        if read_buffer.len().saturating_sub(frame_offset) < frame_total_len {
            break;
        }

        let payload_start = frame_offset + 4;
        let payload_end = payload_start + frame_len;
        let payload = read_buffer[payload_start..payload_end].to_vec();

        match to_channel_tx.outbound_tx.try_send(payload) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                warn!(
                    endpoint = %endpoint.endpoint_name,
                    "Dropped named-pipe inbound frame because virtual channel outbound queue is full"
                );
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "virtual channel bridge sender is closed",
                ));
            }
        }

        frame_offset = payload_end;
    }

    if frame_offset > 0 {
        read_buffer.drain(..frame_offset);
    }

    Ok(())
}

fn write_length_prefixed(mut writer: impl Write, payload: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(payload.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "payload too large"))?;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(payload)
}

fn bridge_endpoint_key(endpoint: &VirtualChannelBridgeEndpoint) -> String {
    let kind = if endpoint.static_channel { "svc" } else { "dvc" };
    format!("{kind}:{}", endpoint.endpoint_name.to_ascii_lowercase())
}

fn bridge_pipe_path(pipe_prefix: &str, endpoint: &VirtualChannelBridgeEndpoint) -> String {
    let normalized_prefix = if pipe_prefix.starts_with(r"\\.\pipe\") {
        pipe_prefix.to_owned()
    } else {
        format!(r"\\.\pipe\{pipe_prefix}")
    };

    let kind = if endpoint.static_channel { "svc" } else { "dvc" };
    let channel = sanitize_pipe_segment(&endpoint.endpoint_name);

    format!("{normalized_prefix}.{kind}.{channel}")
}

fn sanitize_pipe_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }

    if out.is_empty() { "channel".to_owned() } else { out }
}

#[implement(IClassFactory)]
struct ProtocolManagerClassFactory;

impl IClassFactory_Impl for ProtocolManagerClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: windows_core::Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
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

        // SAFETY: `ppvobject` is non-null (checked above) and COM expects us to
        // initialize out-pointers on all paths.
        unsafe { *ppvobject = core::ptr::null_mut() };

        // SAFETY: `riid` is non-null (checked above) and points to a valid GUID per COM contract.
        let requested_iid = unsafe { *riid };

        if requested_iid == IWRdsProtocolManager::IID {
            let manager = create_protocol_manager_com();
            // SAFETY: `ppvobject` is non-null and this branch returns a valid COM interface pointer.
            unsafe { *ppvobject = manager.into_raw() };

            return Ok(());
        }

        if requested_iid == IUnknown::IID {
            let manager = create_protocol_manager_com();
            let unknown: IUnknown = manager.cast()?;
            // SAFETY: `ppvobject` is non-null and this branch returns a valid COM interface pointer.
            unsafe { *ppvobject = unknown.into_raw() };

            return Ok(());
        }

        Err(windows_core::Error::new(
            E_NOINTERFACE,
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

#[expect(unreachable_pub)]
#[unsafe(no_mangle)]
pub extern "system" fn DllGetClassObject(rclsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    let result = dll_get_class_object_impl(rclsid, riid, ppv);

    match result {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

#[expect(unreachable_pub)]
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
    ppv: *mut *mut c_void,
) -> windows_core::Result<()> {
    if ppv.is_null() || riid.is_null() || rclsid.is_null() {
        return Err(windows_core::Error::new(E_POINTER, "null class object pointer"));
    }

    // SAFETY: `ppv` is non-null (checked above) and COM expects out-pointers to be initialized.
    unsafe { *ppv = core::ptr::null_mut() };

    // SAFETY: `rclsid` is non-null (checked above) and points to a valid GUID per COM contract.
    let requested_clsid = unsafe { *rclsid };
    // SAFETY: `riid` is non-null (checked above) and points to a valid GUID per COM contract.
    let requested_iid = unsafe { *riid };
    debug_log_line(&format!(
        "DllGetClassObject requested_clsid={requested_clsid:?} requested_iid={requested_iid:?}",
    ));

    if requested_clsid != IRONRDP_PROTOCOL_MANAGER_CLSID {
        return Err(windows_core::Error::new(
            CLASS_E_CLASSNOTAVAILABLE,
            "unknown protocol manager CLSID",
        ));
    }

    let factory: IClassFactory = ProtocolManagerClassFactory.into();
    if requested_iid == IClassFactory::IID {
        // SAFETY: `ppv` is non-null and this branch returns a valid COM interface pointer.
        unsafe { *ppv = factory.into_raw() };

        return Ok(());
    }

    if requested_iid == IUnknown::IID {
        let unknown: IUnknown = factory.cast()?;
        // SAFETY: `ppv` is non-null and this branch returns a valid COM interface pointer.
        unsafe { *ppv = unknown.into_raw() };

        return Ok(());
    }

    Err(windows_core::Error::new(
        E_NOINTERFACE,
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
        debug_log_line("IWRdsProtocolManager::Initialize");
        info!("Initialized protocol manager");
        Ok(())
    }

    fn CreateListener(&self, wszlistenername: &PCWSTR) -> windows_core::Result<IWRdsProtocolListener> {
        let listener_name = if wszlistenername.is_null() {
            "IRDP-Tcp".to_owned()
        } else {
            // SAFETY: listener name is provided by termservice and expected to be a valid
            // NUL-terminated wide string.
            unsafe { wszlistenername.to_string() }.map_err(|error| {
                windows_core::Error::new(E_UNEXPECTED, format!("failed to decode listener name: {error}"))
            })?
        };

        debug_log_line(&format!(
            "IWRdsProtocolManager::CreateListener listener_name={listener_name}"
        ));
        info!(listener_name = %listener_name, "Created protocol listener");
        Ok(ComProtocolListener::new(self.inner.create_listener(), listener_name).into())
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
    listener_name: String,
    control_bridge: ProviderControlBridge,
    callback: Mutex<Option<IWRdsProtocolListenerCallback>>,
    worker: Mutex<Option<ListenerWorker>>,
}

impl ComProtocolListener {
    fn new(inner: ProtocolListener, listener_name: String) -> Self {
        Self {
            inner: Arc::new(inner),
            listener_name,
            control_bridge: ProviderControlBridge::from_env(),
            callback: Mutex::new(None),
            worker: Mutex::new(None),
        }
    }
}

fn retain_connection_callback(
    connection: &ProtocolConnection,
    callback_slot: &Mutex<Option<AgileReference<IWRdsProtocolConnectionCallback>>>,
    callback: &IWRdsProtocolConnectionCallback,
) -> windows_core::Result<bool> {
    let callback = AgileReference::new(callback)?;
    let previous = {
        let mut slot = callback_slot.lock();
        // UNTESTED: Close must mark the connection closed before clearing this slot to prevent concurrent registration
        // from restoring its callback; check both lock orders when changing registration or Close.
        if connection.state() == ConnectionLifecycleState::Closed {
            debug!(
                connection_id = connection.connection_id(),
                "Discarding callback for a closed connection"
            );
            return Ok(false);
        }
        slot.replace(callback)
    };
    drop(previous);
    Ok(true)
}

impl IWRdsProtocolListener_Impl for ComProtocolListener_Impl {
    fn GetSettings(
        &self,
        wrdslistenersettinglevel: WRDS_LISTENER_SETTING_LEVEL,
    ) -> windows_core::Result<WRDS_LISTENER_SETTINGS> {
        let setting_level_1 = WRDS_LISTENER_SETTING_LEVEL(1);

        if wrdslistenersettinglevel != setting_level_1 {
            debug_log_line(&format!(
                "IWRdsProtocolListener::GetSettings listener_name={} level={} -> E_NOTIMPL (unsupported level)",
                self.listener_name, wrdslistenersettinglevel.0
            ));

            return Err(windows_core::Error::new(
                E_NOTIMPL,
                "unsupported listener setting level",
            ));
        }

        let mut settings = WRDS_LISTENER_SETTINGS {
            WRdsListenerSettingLevel: setting_level_1,
            ..Default::default()
        };

        settings.WRdsListenerSetting.WRdsListenerSettings1 = WRDS_LISTENER_SETTINGS_1 {
            MaxProtocolListenerConnectionCount: UNLIMITED_LISTENER_CONNECTIONS,
            SecurityDescriptorSize: 0,
            pSecurityDescriptor: core::ptr::null_mut(),
        };

        debug_log_line(&format!(
            "IWRdsProtocolListener::GetSettings listener_name={} level={} -> ok (max_conn=unlimited sd_size=0 sd_ptr=0x0)",
            self.listener_name, wrdslistenersettinglevel.0
        ));

        Ok(settings)
    }

    fn StartListen(&self, pcallback: windows_core::Ref<'_, IWRdsProtocolListenerCallback>) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolListener::StartListen listener_name={}",
            self.listener_name
        ));

        let _com_init_guard = ComInitGuard::new();

        let callback = pcallback
            .ok()
            .map_err(|_| windows_core::Error::new(E_POINTER, "null listener callback"))?
            .clone();

        if self.worker.lock().is_some() {
            debug_log_line(&format!(
                "IWRdsProtocolListener::StartListen already_started listener_name={}",
                self.listener_name
            ));
            info!("Protocol listener already started");
            return Ok(());
        }

        debug_log_line(&format!(
            "ProviderControlBridge::start_listen begin listener_name={}",
            self.listener_name
        ));
        let control_bridge_enabled = self.control_bridge.start_listen(&self.listener_name)?;
        debug_log_line(&format!(
            "ProviderControlBridge::start_listen ok listener_name={} enabled={control_bridge_enabled}",
            self.listener_name
        ));

        let (stop_tx, stop_rx) = mpsc::channel();

        let callback_agile = callback.cast::<IAgileObject>().is_ok();
        debug_log_line(&format!(
            "listener callback agile={callback_agile} listener_name={}",
            self.listener_name
        ));

        // TermService's IWRdsProtocolListenerCallback does not have a registered proxy/stub IID on
        // Windows Server 2025 (REGDB_E_IIDNOTREG), so CoMarshalInterThreadInterfaceInStream fails.
        // In practice, this callback is in-proc and appears safe to invoke from a worker thread.
        // We keep ownership correct by AddRef-ing via clone + into_raw and reconstructing on the worker.
        #[expect(
            clippy::as_conversions,
            reason = "store a raw COM interface pointer as usize so it can cross a thread boundary"
        )]
        let callback_token = callback.clone().into_raw() as usize;
        let listener = Arc::clone(&self.inner);
        let control_bridge = self.control_bridge.clone();
        let listener_name = self.listener_name.clone();

        let join_handle = thread::spawn(move || {
            // SAFETY: each worker thread initializes and uninitializes COM exactly once.
            let co_initialize = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if let Err(error) = co_initialize.ok() {
                warn!(%error, "Failed to initialize COM on listener worker thread");
                return;
            }

            #[expect(
                clippy::as_conversions,
                reason = "reconstruct raw COM interface pointer from the usize token"
            )]
            let callback_for_worker = {
                // SAFETY: token was produced by `IWRdsProtocolListenerCallback::into_raw` in this process.
                unsafe { IWRdsProtocolListenerCallback::from_raw(callback_token as *mut c_void) }
            };

            let mut connection_callbacks: HashMap<u32, (Arc<ProtocolConnection>, IWRdsProtocolConnectionCallback)> =
                HashMap::new();

            if control_bridge_enabled {
                loop {
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }

                    // Server-initiated Close need not produce a companion ConnectionBroken event.
                    connection_callbacks
                        .retain(|_, (connection, _)| connection.state() != ConnectionLifecycleState::Closed);

                    let event = match control_bridge.wait_for_incoming(&listener_name, 250) {
                        Ok(event) => event,
                        Err(error) => {
                            warn!(%error, listener_name = %listener_name, "Failed to poll incoming connection from companion service");
                            thread::sleep(Duration::from_millis(200));
                            continue;
                        }
                    };

                    let Some(event) = event else {
                        // wait_for_incoming returned Ok(None): no new connection arrived in the
                        // polling window.  Sleep briefly before re-polling so that orphaned worker
                        // threads (left running in svchost after TermService stops) do not
                        // monopolise the companion's control pipe and starve the StartListen IPC
                        // call that a freshly loaded DLL instance must send.
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    };

                    let incoming = match event {
                        WaitForIncomingEvent::Incoming(incoming) => Some(incoming),
                        WaitForIncomingEvent::ConnectionBroken { connection_id, reason } => {
                            debug_log_line(&format!(
                                "Received connection_broken from companion connection_id={connection_id} reason={reason}",
                            ));

                            if let Some((_, callback)) = connection_callbacks
                                .remove(&connection_id)
                                .filter(|(connection, _)| connection.state() != ConnectionLifecycleState::Closed)
                            {
                                // SAFETY: callback is a valid COM interface returned by TermService.
                                let broken_result = unsafe { callback.BrokenConnection(0, 0) };
                                debug_log_line(&format!(
                                    "IWRdsProtocolConnectionCallback::BrokenConnection result={broken_result:?} connection_id={connection_id}",
                                ));
                            } else {
                                debug_log_line(&format!(
                                    "No TermService callback registered for broken connection_id={connection_id}",
                                ));
                            }

                            None
                        }
                    };

                    let Some(incoming) = incoming else {
                        continue;
                    };

                    let connection_entry = listener.create_connection_with_id(incoming.connection_id);
                    let connection_callback_slot = Arc::new(Mutex::new(None));
                    let connection: IWRdsProtocolConnection = ComProtocolConnection::new(
                        Arc::clone(&connection_entry),
                        Arc::clone(&connection_callback_slot),
                        control_bridge.clone(),
                    )
                    .into();

                    // Start the RDP server immediately so the client's X224 handshake is processed.
                    // RDS may call AcceptConnection later; the companion returns ConnectionReady
                    // if the session is already running.
                    if let Err(error) = control_bridge.accept_connection(incoming.connection_id) {
                        debug_log_line(&format!(
                            "Early accept_connection failed connection_id={} error={}; continuing",
                            incoming.connection_id, error
                        ));
                        warn!(
                            %error,
                            connection_id = incoming.connection_id,
                            "Early accept_connection failed; RDS may call AcceptConnection later"
                        );
                    }

                    match control_bridge.get_connection_credentials(incoming.connection_id) {
                        Ok(Some(credentials)) => {
                            debug_log_line(&format!(
                                "OnConnected credential gate satisfied connection_id={} user={} domain={}",
                                incoming.connection_id, credentials.0, credentials.1,
                            ));
                        }
                        Ok(None) => {
                            debug_log_line(&format!(
                                "OnConnected credential gate timed out connection_id={}",
                                incoming.connection_id
                            ));
                            let _ = control_bridge.close_connection(incoming.connection_id);
                            continue;
                        }
                        Err(error) => {
                            debug_log_line(&format!(
                                "OnConnected credential gate failed connection_id={} error={error}",
                                incoming.connection_id,
                            ));
                            let _ = control_bridge.close_connection(incoming.connection_id);
                            continue;
                        }
                    }

                    let mut settings = default_connection_settings(&listener_name);
                    enable_connection_autologon(&mut settings);

                    let sd_probe: Option<&'static mut [u8]> =
                        if std::env::var("IRONRDP_WTS_SD_PROBE").as_deref() == Ok("1") {
                            // DIAGNOSTIC: TermService may treat WRDS_LISTENER_SETTINGS_1.pSecurityDescriptor as an
                            // output buffer. Provide a large, process-lifetime buffer and detect whether it is written.
                            let probe: &'static mut [u8] = Box::leak(vec![0xCCu8; 4096].into_boxed_slice());
                            let probe_len = u32::try_from(probe.len()).unwrap_or(0);
                            // SAFETY: WRDS_CONNECTION_SETTINGS contains a union; `default_connection_settings` sets level=1.
                            unsafe {
                                let ls1 = &mut settings
                                    .WRdsConnectionSetting
                                    .WRdsConnectionSettings1
                                    .WRdsListenerSettings
                                    .WRdsListenerSetting
                                    .WRdsListenerSettings1;
                                ls1.SecurityDescriptorSize = probe_len;
                                ls1.pSecurityDescriptor = probe.as_mut_ptr();
                            }
                            Some(probe)
                        } else {
                            None
                        };

                    let (listener_level, protocol_type) = {
                        // SAFETY: union view is valid because `default_connection_settings` sets setting_level=1.
                        unsafe {
                            let s1 = &settings.WRdsConnectionSetting.WRdsConnectionSettings1;
                            (s1.WRdsListenerSettings.WRdsListenerSettingLevel.0, s1.ProtocolType)
                        }
                    };

                    debug_log_line(&format!(
                        "OnConnected calling listener_name={} connection_id={} conn_level={} listener_level={} protocol_type={}",
                        listener_name,
                        incoming.connection_id,
                        settings.WRdsConnectionSettingLevel.0,
                        listener_level,
                        protocol_type,
                    ));

                    // SAFETY: WRDS_CONNECTION_SETTINGS contains a union; `default_connection_settings` sets level=1.
                    unsafe {
                        let ls1 = &settings
                            .WRdsConnectionSetting
                            .WRdsConnectionSettings1
                            .WRdsListenerSettings
                            .WRdsListenerSetting
                            .WRdsListenerSettings1;
                        let sd_ptr = ls1.pSecurityDescriptor;
                        let sd_size = ls1.SecurityDescriptorSize;
                        let max_conn = ls1.MaxProtocolListenerConnectionCount;
                        if !sd_ptr.is_null() && sd_size > 0 {
                            let sd_size_usize = usize::try_from(sd_size).unwrap_or(0);
                            let sd_bytes = core::slice::from_raw_parts(sd_ptr, core::cmp::min(sd_size_usize, 40));
                            let hex: String = sd_bytes
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<Vec<_>>()
                                .join(" ");
                            debug_log_line(&format!(
                                "OnConnected SD dump: max_conn={max_conn} sd_size={sd_size} sd_ptr={sd_ptr:?} first_bytes=[{hex}]"
                            ));
                            let sd_psec = PSECURITY_DESCRIPTOR(sd_ptr.cast());
                            let valid = IsValidSecurityDescriptor(sd_psec);
                            debug_log_line(&format!("OnConnected SD valid={} right_before_call", valid.as_bool()));
                        } else {
                            debug_log_line(&format!(
                                "OnConnected SD: NULL or empty max_conn={max_conn} sd_size={sd_size} sd_ptr={sd_ptr:?}"
                            ));
                        }
                    }

                    // SAFETY: COM callback and connection object are valid for call duration.
                    let result = unsafe { callback_for_worker.OnConnected(&connection, &settings) };
                    if let Some(sd_probe) = sd_probe.as_deref() {
                        let sd_written = sd_probe.iter().any(|byte| *byte != 0xCC);
                        let b0 = sd_probe.first().copied().unwrap_or(0);
                        let b1 = sd_probe.get(1).copied().unwrap_or(0);
                        let b2 = sd_probe.get(2).copied().unwrap_or(0);
                        let b3 = sd_probe.get(3).copied().unwrap_or(0);
                        debug_log_line(&format!(
                            "OnConnected sd_probe written={sd_written} first4={b0:02X}{b1:02X}{b2:02X}{b3:02X}",
                        ));
                    }

                    match result {
                        Ok(connection_callback) => {
                            debug_log_line(&format!(
                                "IWRdsProtocolListenerCallback::OnConnected ok listener_name={} connection_id={}",
                                listener_name, incoming.connection_id
                            ));

                            // SAFETY: connection_callback is a valid COM interface returned by TermService.
                            let conn_id_result = unsafe { connection_callback.GetConnectionId() };
                            debug_log_line(&format!(
                                "GetConnectionId result={:?} connection_id={}",
                                conn_id_result, incoming.connection_id
                            ));

                            match retain_connection_callback(
                                &connection_entry,
                                &connection_callback_slot,
                                &connection_callback,
                            ) {
                                Ok(true) => {}
                                Ok(false) => continue,
                                Err(error) => {
                                    debug_log_line(&format!(
                                        "Failed to create AgileReference for connection callback connection_id={} error={error}",
                                        incoming.connection_id,
                                    ));
                                    warn!(
                                        %error,
                                        connection_id = incoming.connection_id,
                                        "Failed to create AgileReference for TermService connection callback"
                                    );
                                }
                            }

                            connection_callbacks
                                .insert(incoming.connection_id, (connection_entry, connection_callback.clone()));

                            debug_log_line(&format!(
                                "OnReady credential gate already satisfied connection_id={}",
                                incoming.connection_id,
                            ));

                            // SAFETY: connection_callback is a valid COM interface returned by TermService.
                            let ready_result = unsafe { connection_callback.OnReady() };
                            debug_log_line(&format!(
                                "OnReady result={:?} connection_id={}",
                                ready_result, incoming.connection_id
                            ));

                            debug!(
                                connection_id = incoming.connection_id,
                                peer_addr = ?incoming.peer_addr,
                                "Dispatched incoming connection from companion service"
                            );
                        }
                        Err(error) => {
                            debug_log_line(&format!(
                                "IWRdsProtocolListenerCallback::OnConnected failed listener_name={} connection_id={} error={error}",
                                listener_name, incoming.connection_id
                            ));
                            warn!(
                                %error,
                                connection_id = incoming.connection_id,
                                "Failed to dispatch OnConnected callback for incoming connection"
                            );
                        }
                    }
                }
            } else {
                let bootstrap_connection = listener.create_connection();
                let connection_callback_slot = Arc::new(Mutex::new(None));
                let connection: IWRdsProtocolConnection = ComProtocolConnection::new(
                    Arc::clone(&bootstrap_connection),
                    Arc::clone(&connection_callback_slot),
                    control_bridge,
                )
                .into();

                let mut settings = default_connection_settings(&listener_name);

                let sd_probe: Option<&'static mut [u8]> = if std::env::var("IRONRDP_WTS_SD_PROBE").as_deref() == Ok("1")
                {
                    let probe: &'static mut [u8] = Box::leak(vec![0xCCu8; 4096].into_boxed_slice());
                    let probe_len = u32::try_from(probe.len()).unwrap_or(0);
                    // SAFETY: WRDS_CONNECTION_SETTINGS contains a union; `default_connection_settings` sets level=1.
                    unsafe {
                        let ls1 = &mut settings
                            .WRdsConnectionSetting
                            .WRdsConnectionSettings1
                            .WRdsListenerSettings
                            .WRdsListenerSetting
                            .WRdsListenerSettings1;
                        ls1.SecurityDescriptorSize = probe_len;
                        ls1.pSecurityDescriptor = probe.as_mut_ptr();
                    }
                    Some(probe)
                } else {
                    None
                };

                let (listener_level, protocol_type) = {
                    // SAFETY: union view is valid because `default_connection_settings` sets setting_level=1.
                    unsafe {
                        let s1 = &settings.WRdsConnectionSetting.WRdsConnectionSettings1;
                        (s1.WRdsListenerSettings.WRdsListenerSettingLevel.0, s1.ProtocolType)
                    }
                };

                debug_log_line(&format!(
                    "OnConnected calling listener_name={} connection_id={} conn_level={} listener_level={} protocol_type={}",
                    listener_name, 0, settings.WRdsConnectionSettingLevel.0, listener_level, protocol_type,
                ));

                // SAFETY: COM callback and connection object are valid for call duration.
                let result = unsafe { callback_for_worker.OnConnected(&connection, &settings) };
                if let Some(sd_probe) = sd_probe.as_deref() {
                    let sd_written = sd_probe.iter().any(|byte| *byte != 0xCC);
                    let b0 = sd_probe.first().copied().unwrap_or(0);
                    let b1 = sd_probe.get(1).copied().unwrap_or(0);
                    let b2 = sd_probe.get(2).copied().unwrap_or(0);
                    let b3 = sd_probe.get(3).copied().unwrap_or(0);
                    debug_log_line(&format!(
                        "OnConnected sd_probe written={sd_written} first4={b0:02X}{b1:02X}{b2:02X}{b3:02X}",
                    ));
                }

                match result {
                    Ok(connection_callback) => match retain_connection_callback(
                        &bootstrap_connection,
                        &connection_callback_slot,
                        &connection_callback,
                    ) {
                        Ok(_) => {}
                        Err(error) => {
                            debug_log_line(&format!(
                                "Failed to create AgileReference for connection callback (standalone) error={error}",
                            ));
                            warn!(
                                %error,
                                "Failed to create AgileReference for TermService connection callback (standalone)"
                            );
                        }
                    },
                    Err(error) => {
                        warn!(%error, "Failed to dispatch OnConnected callback");
                    }
                }

                let _ = stop_rx.recv();
            }

            // SAFETY: paired with successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
        });

        *self.worker.lock() = Some(ListenerWorker { stop_tx, join_handle });
        *self.callback.lock() = Some(callback);
        debug_log_line(&format!(
            "IWRdsProtocolListener::StartListen worker_started listener_name={}",
            self.listener_name
        ));
        info!("Started protocol listener worker");

        Ok(())
    }

    fn StopListen(&self) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolListener::StopListen listener_name={}",
            self.listener_name
        ));
        if let Some(worker) = self.worker.lock().take() {
            if let Err(error) = worker.stop_tx.send(()) {
                warn!(%error, "Failed to signal listener worker stop");
            }

            if let Err(error) = worker.join_handle.join() {
                warn!(?error, "Listener worker thread panicked");
            }
        }

        *self.callback.lock() = None;

        if let Err(error) = self.control_bridge.stop_listen(&self.listener_name) {
            warn!(%error, listener_name = %self.listener_name, "Failed to stop companion service listener");
        }

        info!("Stopped protocol listener worker");
        Ok(())
    }
}

// Minimal IWRdsProtocolLicenseConnection implementation that allows TermService to proceed past
// licensing.  Returning E_NOTIMPL from GetLicenseConnection causes TermService to close the
// connection immediately; providing this stub object allows the connection to advance to
// IsUserAllowedToLogon so that the user token can be passed back and auto-logon can occur.
#[implement(IWRdsProtocolLicenseConnection)]
struct WrdsLicenseConnection;

fn fill_license_capabilities(
    license_capabilities: *mut WTS_LICENSE_CAPABILITIES,
    license_capabilities_size: *mut u32,
) -> windows_core::Result<()> {
    if license_capabilities.is_null() || license_capabilities_size.is_null() {
        return Err(windows_core::Error::new(
            E_POINTER,
            "null licensing capabilities pointer",
        ));
    }

    let required_size =
        u32::try_from(size_of::<WTS_LICENSE_CAPABILITIES>()).expect("WTS_LICENSE_CAPABILITIES size fits in u32");
    // SAFETY: `license_capabilities_size` is non-null and points to the caller's in/out capacity.
    let capacity = unsafe { *license_capabilities_size };
    // SAFETY: the size pointer is writable and receives the required size on every outcome.
    unsafe { *license_capabilities_size = required_size };

    if capacity < required_size {
        return Err(windows_core::Error::new(
            HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0),
            "licensing capabilities buffer is too small",
        ));
    }

    let capabilities = WTS_LICENSE_CAPABILITIES {
        KeyExchangeAlg: WTS_KEY_EXCHANGE_ALG_RSA,
        ProtocolVer: WTS_LICENSE_PREAMBLE_VERSION,
        fAuthenticateServer: BOOL(0),
        CertType: WTS_CERT_TYPE_INVALID,
        cbClientName: 0,
        rgbClientName: [0; 42],
    };
    // SAFETY: the caller reported enough writable storage for the structure.
    unsafe { core::ptr::write(license_capabilities, capabilities) };

    Ok(())
}

impl IWRdsProtocolLicenseConnection_Impl for WrdsLicenseConnection_Impl {
    fn RequestLicensingCapabilities(
        &self,
        pplicensecapabilities: *mut WTS_LICENSE_CAPABILITIES,
        pcblicensecapabilities: *mut u32,
    ) -> windows_core::Result<()> {
        debug_log_line("WrdsLicenseConnection::RequestLicensingCapabilities");
        fill_license_capabilities(pplicensecapabilities, pcblicensecapabilities)
    }

    fn SendClientLicense(&self, _pclientlicense: *const u8, _cbclientlicense: u32) -> windows_core::Result<()> {
        debug_log_line("WrdsLicenseConnection::SendClientLicense");
        Ok(())
    }

    fn RequestClientLicense(
        &self,
        _reserve1: *const u8,
        _reserve2: u32,
        _ppclientlicense: *mut u8,
        pcbclientlicense: *mut u32,
    ) -> windows_core::Result<()> {
        debug_log_line("WrdsLicenseConnection::RequestClientLicense");
        // Return an empty license (0 bytes) — sufficient for the RDS grace period and
        // "Remote Desktop for Administration" (2-admin-connection) mode on a non-CAL server.
        if !pcbclientlicense.is_null() {
            // SAFETY: pcbclientlicense is a valid out-pointer when non-null.
            unsafe { *pcbclientlicense = 0 };
        }
        Ok(())
    }

    fn ProtocolComplete(&self, _ulcomplete: u32) -> windows_core::Result<()> {
        debug_log_line("WrdsLicenseConnection::ProtocolComplete");
        Ok(())
    }
}

#[implement(IWRdsProtocolLogonErrorRedirector)]
struct WrdsLogonErrorRedirector;

impl IWRdsProtocolLogonErrorRedirector_Impl for WrdsLogonErrorRedirector_Impl {
    fn OnBeginPainting(&self) -> windows_core::Result<()> {
        debug_log_line("WrdsLogonErrorRedirector::OnBeginPainting");
        Ok(())
    }

    fn RedirectStatus(&self, pszmessage: &PCWSTR) -> windows_core::Result<WTS_LOGON_ERROR_REDIRECTOR_RESPONSE> {
        let message = if pszmessage.0.is_null() {
            String::new()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { pszmessage.to_string() }.unwrap_or_default()
        };

        debug_log_line(&format!("WrdsLogonErrorRedirector::RedirectStatus message={message}",));
        Ok(WTS_LOGON_ERR_NOT_HANDLED)
    }

    fn RedirectMessage(
        &self,
        pszcaption: &PCWSTR,
        pszmessage: &PCWSTR,
        utype: u32,
    ) -> windows_core::Result<WTS_LOGON_ERROR_REDIRECTOR_RESPONSE> {
        let caption = if pszcaption.0.is_null() {
            String::new()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { pszcaption.to_string() }.unwrap_or_default()
        };
        let message = if pszmessage.0.is_null() {
            String::new()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { pszmessage.to_string() }.unwrap_or_default()
        };

        debug_log_line(&format!(
            "WrdsLogonErrorRedirector::RedirectMessage utype={utype} caption={caption} message={message}",
        ));
        Ok(WTS_LOGON_ERR_NOT_HANDLED)
    }

    fn RedirectLogonError(
        &self,
        ntsstatus: i32,
        ntssubstatus: i32,
        pszcaption: &PCWSTR,
        pszmessage: &PCWSTR,
        utype: u32,
    ) -> windows_core::Result<WTS_LOGON_ERROR_REDIRECTOR_RESPONSE> {
        let caption = if pszcaption.0.is_null() {
            String::new()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { pszcaption.to_string() }.unwrap_or_default()
        };
        let message = if pszmessage.0.is_null() {
            String::new()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { pszmessage.to_string() }.unwrap_or_default()
        };

        debug_log_line(&format!(
            "WrdsLogonErrorRedirector::RedirectLogonError ntsstatus={ntsstatus} ntssubstatus={ntssubstatus} utype={utype} caption={caption} message={message}",
        ));
        Ok(WTS_LOGON_ERR_NOT_HANDLED)
    }
}

#[implement(
    IWRdsProtocolConnection,
    IWRdsProtocolConnection2,
    IWRdsEnhancedFastReconnectArbitrator,
    IWRdsWddmIddProps,
    IWRdsWddmIddProps1,
    IAgileObject
)]
struct ComProtocolConnection {
    inner: Arc<ProtocolConnection>,
    connection_callback: Arc<Mutex<Option<AgileReference<IWRdsProtocolConnectionCallback>>>>,
    control_bridge: ProviderControlBridge,
    published_applications: PublishedApplications,
    wddm_idd_enabled: AtomicBool,
    idd_last_driver_load_session_id: AtomicU32,
    idd_driver_load_notified: AtomicBool,
    driver_handle_raw: AtomicUsize,
    ready_notified: Mutex<bool>,
    cached_credentials: Mutex<Option<(String, String, String)>>,
    last_input_time: Mutex<u64>,
    video_handle: Mutex<Option<HANDLE>>,
    session_impersonation_token: Mutex<Option<(u32, HANDLE)>>,
    video_handle_source: Mutex<VideoHandleSource>,
    driver_device_instance: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VideoHandleSource {
    Unknown,
    DriverLoadCallback,
}

impl VideoHandleSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::DriverLoadCallback => "driver_load_callback",
        }
    }
}

fn update_idd_runtime_video_source(connection_id: u32, source: VideoHandleSource, path: Option<&str>) {
    if let Some(path) = path {
        update_idd_runtime_state(
            &[
                ("active_video_source", source.as_str().to_owned()),
                ("active_video_path", path.to_owned()),
                ("provider_connection_id", connection_id.to_string()),
            ],
            "video_handle_selected",
        );
    } else {
        update_idd_runtime_state(
            &[
                ("active_video_source", source.as_str().to_owned()),
                ("provider_connection_id", connection_id.to_string()),
            ],
            "video_handle_selected",
        );
    }
}

fn env_var_truthy(name: &str) -> bool {
    match provider_override_env_var(name) {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => false,
    }
}

fn provider_override_env_var(name: &str) -> Result<String, std::env::VarError> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        Err(std::env::VarError::NotPresent) => read_termservice_environment_var(name)
            .or_else(|| read_machine_environment_var(name))
            .ok_or(std::env::VarError::NotPresent),
        Ok(_) => read_termservice_environment_var(name)
            .or_else(|| read_machine_environment_var(name))
            .ok_or(std::env::VarError::NotPresent),
        Err(error) => Err(error),
    }
}

fn read_termservice_environment_var(name: &str) -> Option<String> {
    let mut bytes = 0u32;
    let flags = RRF_RT_REG_MULTI_SZ | RRF_ZEROONFAILURE;

    let query_status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            windows::core::w!("SYSTEM\\CurrentControlSet\\Services\\TermService"),
            windows::core::w!("Environment"),
            flags,
            None,
            None,
            Some(&mut bytes),
        )
    };

    if query_status != WIN32_ERROR(0) || bytes < 2 {
        return None;
    }

    let mut wide = vec![0u16; usize::try_from(bytes).ok()?.div_ceil(size_of::<u16>())];
    let read_status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            windows::core::w!("SYSTEM\\CurrentControlSet\\Services\\TermService"),
            windows::core::w!("Environment"),
            flags,
            None,
            Some(wide.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };

    if read_status != WIN32_ERROR(0) {
        debug_log_line(&format!(
            "read_termservice_environment_var name={name} RegGetValueW failed status={read_status:?}",
        ));
        return None;
    }

    let used_len = usize::try_from(bytes).ok()?.div_ceil(size_of::<u16>());
    for entry in wide[..used_len].split(|ch| *ch == 0) {
        if entry.is_empty() {
            continue;
        }

        let item = String::from_utf16_lossy(entry);
        let Some((entry_name, entry_value)) = item.split_once('=') else {
            continue;
        };

        if entry_name.eq_ignore_ascii_case(name) {
            return Some(entry_value.to_owned());
        }
    }

    None
}

fn read_machine_environment_var(name: &str) -> Option<String> {
    let value_name: Vec<u16> = name.encode_utf16().chain(core::iter::once(0)).collect();
    let mut bytes = 0u32;
    let flags = RRF_RT_REG_SZ | RRF_ZEROONFAILURE;

    let query_status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            windows::core::w!("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment"),
            PCWSTR(value_name.as_ptr()),
            flags,
            None,
            None,
            Some(&mut bytes),
        )
    };

    if query_status != WIN32_ERROR(0) || bytes < 2 {
        return None;
    }

    let mut wide = vec![0u16; usize::try_from(bytes).ok()?.div_ceil(size_of::<u16>())];
    let read_status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            windows::core::w!("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment"),
            PCWSTR(value_name.as_ptr()),
            flags,
            None,
            Some(wide.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };

    if read_status != WIN32_ERROR(0) {
        debug_log_line(&format!(
            "read_machine_environment_var name={name} RegGetValueW failed status={read_status:?}",
        ));
        return None;
    }

    let used_len = usize::try_from(bytes).ok()?.div_ceil(size_of::<u16>());
    let value = String::from_utf16_lossy(&wide[..used_len]);
    let trimmed = value.trim_end_matches('\0').trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn provider_fast_reconnect_mode() -> u32 {
    static MODE: OnceLock<u32> = OnceLock::new();

    *MODE.get_or_init(|| {
        // We do not implement session reattachment in AuthenticateClientToSession, so advertising
        // fast reconnect by default gives RCM misleading reconnect hints after CredSSP completes.
        // On current Server builds that tends to rebind the connection to the hidden pre-logon
        // temporary session instead of creating a fresh interactive session. Keep the override for
        // targeted experiments, but default to disabled until reconnect is implemented end-to-end.
        let configured_raw = provider_override_env_var("IRONRDP_WTS_PROVIDER_FAST_RECONNECT_MODE").ok();
        let configured = configured_raw
            .as_deref()
            .and_then(|value| value.trim().parse::<u32>().ok())
            .unwrap_or(FAST_RECONNECT_DISABLED);

        debug_log_line(&format!(
            "provider_fast_reconnect_mode resolved raw={:?} configured={configured}",
            configured_raw,
        ));

        match configured {
            FAST_RECONNECT_DISABLED | FAST_RECONNECT_BASIC | FAST_RECONNECT_ENHANCED => configured,
            _ => FAST_RECONNECT_DISABLED,
        }
    })
}

fn provider_video_handle_probe_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();

    *ENABLED.get_or_init(|| env_var_truthy("IRONRDP_WTS_PROVIDER_ENABLE_VIDEO_HANDLE_PROBES"))
}

fn provider_props1_driver_probe_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();

    *ENABLED.get_or_init(|| env_var_truthy("IRONRDP_WTS_PROVIDER_ENABLE_PROPS1_DRIVER_PROBE"))
}

impl IAgileObject_Impl for ComProtocolConnection_Impl {}

impl ComProtocolConnection {
    fn new(
        inner: Arc<ProtocolConnection>,
        connection_callback: Arc<Mutex<Option<AgileReference<IWRdsProtocolConnectionCallback>>>>,
        control_bridge: ProviderControlBridge,
    ) -> Self {
        Self {
            inner,
            connection_callback,
            control_bridge,
            published_applications: PublishedApplications::default(),
            wddm_idd_enabled: AtomicBool::new(false),
            idd_last_driver_load_session_id: AtomicU32::new(0),
            idd_driver_load_notified: AtomicBool::new(false),
            driver_handle_raw: AtomicUsize::new(0),
            ready_notified: Mutex::new(false),
            cached_credentials: Mutex::new(None),
            last_input_time: Mutex::new(0),
            video_handle: Mutex::new(None),
            session_impersonation_token: Mutex::new(None),
            video_handle_source: Mutex::new(VideoHandleSource::Unknown),
            driver_device_instance: Mutex::new(None),
        }
    }

    fn open_native_channel(&self, endpoint_name: &str, is_static: bool) -> windows_core::Result<usize> {
        let session_id = self
            .inner
            .session_id()
            .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "native channel session is not assigned"))?;
        let connection_id = self.inner.connection_id();
        let bridge = &self.control_bridge;
        with_provider_identity(|| {
            let event = bridge.send_command(&ProviderCommand::OpenVirtualChannel {
                connection_id,
                session_id,
                endpoint_name: endpoint_name.to_owned(),
                static_channel: is_static,
            })?;
            let pipe_name = match event {
                Some(ServiceEvent::VirtualChannelReady { pipe_name }) => pipe_name,
                Some(ServiceEvent::Error { message }) => return Err(windows_core::Error::new(E_UNEXPECTED, message)),
                _ => {
                    return Err(windows_core::Error::new(
                        E_UNEXPECTED,
                        "native channel transport did not become ready",
                    ));
                }
            };
            if !pipe_name.starts_with(r"\\.\pipe\IronRdpNativeDvc-") {
                return Err(windows_core::Error::new(
                    E_UNEXPECTED,
                    "invalid native channel pipe name",
                ));
            }
            let pipe = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(FILE_FLAG_OVERLAPPED.0 | SECURITY_SQOS_PRESENT.0 | SECURITY_IDENTIFICATION.0)
                .open(&pipe_name)
                .map_err(|error| io_error_to_windows_error(error, "open native channel pipe"))?;
            // SAFETY: The file owns a connected message-pipe handle for the duration of this call.
            unsafe { SetNamedPipeHandleState(HANDLE(pipe.as_raw_handle()), Some(&PIPE_READMODE_MESSAGE), None, None) }?;
            debug_log_line(&format!(
                "IWRdsProtocolConnection::CreateVirtualChannel ready connection_id={connection_id} session_id={session_id} endpoint={endpoint_name} static={is_static}",
            ));
            // TermService duplicates this handle into the authorized WTS caller and closes the original.
            Ok(pipe.into_raw_handle() as usize)
        })
    }

    fn close_device_handles(&self) {
        // Keyboard and mouse handles are not tracked here: GetInputHandles transfers them to TermService, which closes them.
        self.close_cached_video_handle();

        if let Some((session_id, token)) = self.session_impersonation_token.lock().take() {
            debug_log_line(&format!("release_session_impersonation_token session_id={session_id}"));
            unsafe {
                let _ = CloseHandle(token);
            }
        }

        *self.video_handle_source.lock() = VideoHandleSource::Unknown;
        *self.driver_device_instance.lock() = None;
        self.driver_handle_raw.store(0, Ordering::SeqCst);
        self.idd_last_driver_load_session_id.store(0, Ordering::SeqCst);
        self.idd_driver_load_notified.store(false, Ordering::SeqCst);
    }

    fn fill_idd_hardware_id(pdisplaydriverhardwareid: &PCWSTR, count: u32) -> windows_core::Result<()> {
        ensure_wudfrd_running();

        let hardware_id = IRONRDP_IDD_HARDWARE_ID;
        update_idd_runtime_state(&[("hardware_id", hardware_id.to_owned())], "get_hardware_id");

        let wide: Vec<u16> = hardware_id.encode_utf16().chain(Some(0)).collect();

        let required = wide.len();
        let capacity = usize::try_from(count).unwrap_or(0);

        if pdisplaydriverhardwareid.0.is_null() {
            debug_log_line("IWRdsWddmIddProps::GetHardwareId null out-buffer");
            return Err(windows_core::Error::new(E_POINTER, "hardware id buffer is null"));
        }

        if capacity < required {
            debug_log_line(&format!(
                "IWRdsWddmIddProps::GetHardwareId insufficient buffer capacity={capacity} required={required}",
            ));
            return Err(windows_core::Error::new(
                HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0),
                "hardware id buffer is too small",
            ));
        }

        unsafe {
            core::ptr::copy_nonoverlapping(wide.as_ptr(), pdisplaydriverhardwareid.0.cast_mut(), required);
        }

        debug_log_line(&format!(
            "IWRdsWddmIddProps::GetHardwareId wrote '{hardware_id}' chars={}",
            required.saturating_sub(1)
        ));

        Ok(())
    }

    fn parse_driver_callback_session_id(device_instance: &str) -> Option<u32> {
        let normalized = device_instance.to_ascii_lowercase();
        let marker = "sessionid_";
        let index = normalized.find(marker)?;
        let suffix = normalized.get(index + marker.len()..)?;
        let digits = suffix.chars().take_while(|ch| ch.is_ascii_digit()).collect::<String>();
        if digits.is_empty() {
            return None;
        }

        digits.parse::<u32>().ok()
    }

    fn open_ironrdp_idd_from_driver_callback_device_instance(
        &self,
        session_id: u32,
        device_instance: &str,
    ) -> windows_core::Result<(HANDLE, String)> {
        let callback_session_id = Self::parse_driver_callback_session_id(device_instance);
        let session_filter = callback_session_id.or(Some(session_id));
        let callback_prefix = format!("callback_device_instance:{device_instance}");

        self.open_ironrdp_idd_from_setupapi_interface(session_filter)
            .map(|(handle, path)| (handle, format!("{callback_prefix}|{path}")))
            .or_else(|_| {
                self.open_ironrdp_idd_from_setupapi_pdo(session_filter)
                    .map(|(handle, path)| (handle, format!("{callback_prefix}|{path}")))
            })
    }

    fn close_cached_video_handle(&self) {
        if let Some(handle) = self.video_handle.lock().take() {
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
    }

    fn pack_cached_user_credentials(
        &self,
        connection_id: u32,
        source: &'static str,
    ) -> windows_core::Result<(*mut WRDS_SERIALIZED_USER_CREDENTIAL, u32)> {
        let credentials = self.connection_credentials(connection_id, source)?;

        let Some((winlogon_username, winlogon_domain, password)) = credentials else {
            debug_log_line(&format!(
                "GetSerializedUserCredential no_credentials connection_id={connection_id} source={source}",
            ));
            return Err(windows_core::Error::new(
                E_NOTIMPL,
                "no CredSSP credentials available yet",
            ));
        };

        let packed_username = if winlogon_domain.is_empty() || winlogon_username.contains('@') {
            winlogon_username
        } else {
            format!(r"{winlogon_domain}\{winlogon_username}")
        };

        let mut username_wide: Vec<u16> = packed_username.encode_utf16().chain(Some(0)).collect();
        let mut password_wide: Vec<u16> = password.encode_utf16().chain(Some(0)).collect();
        let mut packed_size = 0u32;

        // SAFETY: probing required size with null output buffer follows `CredPackAuthenticationBufferW` contract.
        let probe = unsafe {
            CredPackAuthenticationBufferW(
                CRED_PACK_UNPROTECTED,
                PWSTR(username_wide.as_mut_ptr()),
                PWSTR(password_wide.as_mut_ptr()),
                core::ptr::null_mut(),
                &mut packed_size,
            )
        };

        if probe.as_bool() {
            debug_log_line(&format!(
                "GetSerializedUserCredential unexpected zero-length probe success connection_id={connection_id}",
            ));
        }

        if packed_size == 0 {
            let error = unsafe { GetLastError() };
            debug_log_line(&format!(
                "GetSerializedUserCredential probe failed connection_id={connection_id} user={packed_username} last_error={}",
                error.0
            ));
            return Err(windows_core::Error::from_hresult(HRESULT::from_win32(error.0)));
        }

        let packed_len = usize::try_from(packed_size).unwrap_or(0);
        if packed_len == 0 {
            return Err(windows_core::Error::new(
                E_OUTOFMEMORY,
                "invalid packed credential size",
            ));
        }

        // SAFETY: COM allocates the returned blob so TermService can free it with COM task allocator rules.
        let packed_ptr = unsafe { CoTaskMemAlloc(packed_len) }.cast::<u8>();
        if packed_ptr.is_null() {
            return Err(windows_core::Error::new(
                E_OUTOFMEMORY_WIN32,
                "CoTaskMemAlloc failed for packed credentials",
            ));
        }

        // SAFETY: `packed_ptr` references `packed_len` writable bytes from `CoTaskMemAlloc`.
        let packed_ok = unsafe {
            CredPackAuthenticationBufferW(
                CRED_PACK_UNPROTECTED,
                PWSTR(username_wide.as_mut_ptr()),
                PWSTR(password_wide.as_mut_ptr()),
                packed_ptr,
                &mut packed_size,
            )
        };

        if !packed_ok.as_bool() {
            let error = unsafe { GetLastError() };
            debug_log_line(&format!(
                "GetSerializedUserCredential pack failed connection_id={connection_id} user={packed_username} last_error={}",
                error.0
            ));
            // SAFETY: `packed_ptr` came from `CoTaskMemAlloc`.
            unsafe {
                CoTaskMemFree(Some(packed_ptr.cast()));
            }
            return Err(windows_core::Error::from_hresult(HRESULT::from_win32(error.0)));
        }

        // SAFETY: COM allocates the returned struct so TermService can free it with COM task allocator rules.
        let wrapper_ptr = unsafe { CoTaskMemAlloc(size_of::<WRDS_SERIALIZED_USER_CREDENTIAL>()) }
            .cast::<WRDS_SERIALIZED_USER_CREDENTIAL>();
        if wrapper_ptr.is_null() {
            unsafe {
                CoTaskMemFree(Some(packed_ptr.cast()));
            }
            return Err(windows_core::Error::new(
                E_OUTOFMEMORY_WIN32,
                "CoTaskMemAlloc failed for serialized credential wrapper",
            ));
        }

        // SAFETY: `wrapper_ptr` references writable storage allocated above.
        unsafe {
            wrapper_ptr.write(WRDS_SERIALIZED_USER_CREDENTIAL {
                serialization_length: packed_size,
                serialization: packed_ptr,
            });
        }

        debug_log_line(&format!(
            "GetSerializedUserCredential packed connection_id={connection_id} user={packed_username} packed_bytes={packed_size}",
        ));

        Ok((wrapper_ptr, packed_size))
    }

    fn commit_driver_load_state(
        &self,
        sessionid: u32,
        driver_handle_raw: usize,
        connection_id: u32,
        callback_name: &str,
        device_instance: Option<&str>,
        opened_path: Option<&str>,
    ) {
        self.driver_handle_raw.store(driver_handle_raw, Ordering::SeqCst);
        *self.video_handle_source.lock() = if driver_handle_raw != 0 {
            VideoHandleSource::DriverLoadCallback
        } else {
            VideoHandleSource::Unknown
        };
        *self.driver_device_instance.lock() = device_instance.map(ToOwned::to_owned);
        self.idd_last_driver_load_session_id.store(sessionid, Ordering::SeqCst);
        self.idd_driver_load_notified.store(false, Ordering::SeqCst);

        let mut runtime_updates = vec![
            ("driver_loaded", (driver_handle_raw != 0).to_string()),
            ("session_id", sessionid.to_string()),
            ("driver_handle_nonzero", (driver_handle_raw != 0).to_string()),
            (
                "active_video_source",
                if driver_handle_raw != 0 {
                    VideoHandleSource::DriverLoadCallback.as_str().to_owned()
                } else {
                    VideoHandleSource::Unknown.as_str().to_owned()
                },
            ),
            ("provider_connection_id", connection_id.to_string()),
            ("driver_callback", callback_name.to_owned()),
        ];

        if let Some(device_instance) = device_instance {
            runtime_updates.push(("driver_device_instance", device_instance.to_owned()));
        }

        if let Some(opened_path) = opened_path {
            runtime_updates.push(("active_video_path", opened_path.to_owned()));
        }

        update_idd_runtime_state(&runtime_updates, "on_driver_load");
    }

    fn maybe_notify_driver_load(&self, sessionid: u32, connection_id: u32) {
        if self.driver_handle_raw.load(Ordering::SeqCst) == 0 {
            debug_log_line(&format!(
                "SESSION_PROOF_PROVIDER_IDD_NOTIFY_DRIVER_LOAD_DEFERRED source=on_driver_load connection_id={connection_id} session_id={sessionid} reason=no_driver_handle",
            ));
            return;
        }

        if let Err(error) = self.control_bridge.notify_idd_driver_loaded(sessionid) {
            debug_log_line(&format!(
                "IWRdsWddmIddProps::OnDriverLoad notify_idd_driver_loaded failed: {error}"
            ));
            debug_log_line(&format!(
                "SESSION_PROOF_PROVIDER_IDD_NOTIFY_DRIVER_LOAD_ERROR connection_id={connection_id} session_id={sessionid} error={error}",
            ));
        } else {
            self.idd_driver_load_notified.store(true, Ordering::SeqCst);
            debug_log_line(&format!(
                "SESSION_PROOF_PROVIDER_IDD_NOTIFY_DRIVER_LOAD_ACK connection_id={connection_id} session_id={sessionid}",
            ));
        }
    }

    fn clear_driver_load_state(&self, connection_id: u32, _sessionid: u32, callback_name: &str) {
        self.close_cached_video_handle();
        self.driver_handle_raw.store(0, Ordering::SeqCst);
        *self.video_handle_source.lock() = VideoHandleSource::Unknown;
        *self.driver_device_instance.lock() = None;
        self.idd_last_driver_load_session_id.store(0, Ordering::SeqCst);
        self.idd_driver_load_notified.store(false, Ordering::SeqCst);
        update_idd_runtime_state(
            &[
                ("driver_loaded", "0".to_owned()),
                ("session_id", "0".to_owned()),
                ("driver_handle_nonzero", "false".to_owned()),
                ("active_video_source", VideoHandleSource::Unknown.as_str().to_owned()),
                ("provider_connection_id", connection_id.to_string()),
                ("driver_callback", callback_name.to_owned()),
                ("driver_device_instance", String::new()),
                ("active_video_path", String::new()),
            ],
            "on_driver_unload",
        );
    }

    fn duplicate_impersonation_token(
        token: HANDLE,
        desired_access: windows::Win32::Security::TOKEN_ACCESS_MASK,
    ) -> windows_core::Result<HANDLE> {
        let mut impersonation_token = HANDLE::default();

        unsafe {
            DuplicateTokenEx(
                token,
                desired_access,
                None,
                SecurityImpersonation,
                TokenImpersonation,
                &mut impersonation_token,
            )?;
        }

        Ok(impersonation_token)
    }

    fn cache_session_impersonation_token(&self, session_id: u32, usertoken: HANDLE_PTR) {
        if usertoken.0 == 0 {
            debug_log_line(&format!(
                "cache_session_impersonation_token skipped session_id={session_id} reason=null_token",
            ));
            return;
        }

        let token = HANDLE(usertoken.0 as *mut c_void);
        match Self::duplicate_impersonation_token(token, TOKEN_DUPLICATE | TOKEN_IMPERSONATE | TOKEN_QUERY) {
            Ok(impersonation_token) => {
                let mut slot = self.session_impersonation_token.lock();
                if let Some((previous_session_id, previous_token)) = slot.replace((session_id, impersonation_token)) {
                    debug_log_line(&format!(
                        "cache_session_impersonation_token replaced previous_session_id={previous_session_id} new_session_id={session_id}",
                    ));
                    unsafe {
                        let _ = CloseHandle(previous_token);
                    }
                } else {
                    debug_log_line(&format!(
                        "cache_session_impersonation_token stored session_id={session_id}"
                    ));
                }
            }
            Err(error) => {
                debug_log_line(&format!(
                    "cache_session_impersonation_token failed session_id={session_id} error={error}"
                ));
            }
        }
    }

    fn duplicate_cached_session_impersonation_token(&self, session_id: u32) -> windows_core::Result<HANDLE> {
        let token = {
            let slot = self.session_impersonation_token.lock();
            match *slot {
                Some((cached_session_id, token)) if cached_session_id == session_id => token,
                Some((cached_session_id, _)) => {
                    return Err(windows_core::Error::new(
                        HRESULT::from_win32(ERROR_NO_TOKEN.0),
                        format!(
                            "cached session impersonation token targets session {cached_session_id}, not {session_id}"
                        ),
                    ));
                }
                None => {
                    return Err(windows_core::Error::new(
                        HRESULT::from_win32(ERROR_NO_TOKEN.0),
                        "no cached session impersonation token available",
                    ));
                }
            }
        };

        Self::duplicate_impersonation_token(token, TOKEN_IMPERSONATE | TOKEN_QUERY)
    }

    fn idd_readiness_snapshot(&self) -> (bool, bool, bool, Option<u32>) {
        let wddm_enabled = self.wddm_idd_enabled.load(Ordering::SeqCst);
        let driver_handle_seen = self.driver_handle_raw.load(Ordering::SeqCst) != 0;
        let driver_load_notified = self.idd_driver_load_notified.load(Ordering::SeqCst);
        let driver_load_session_id = match self.idd_last_driver_load_session_id.load(Ordering::SeqCst) {
            0 => None,
            value => Some(value),
        };

        (
            wddm_enabled,
            driver_handle_seen,
            driver_load_notified,
            driver_load_session_id,
        )
    }

    fn probe_idd_driver_load(
        &self,
        session_id: u32,
        source: &'static str,
        attempts: u32,
        retry_delay: Duration,
    ) -> bool {
        if !self.wddm_idd_enabled.load(Ordering::SeqCst) {
            return false;
        }

        if !provider_video_handle_probe_enabled() {
            let connection_id = self.inner.connection_id();
            debug_log_line(&format!(
                "SESSION_PROOF_PROVIDER_IDD_DIAGNOSTIC_PROBE_SKIPPED source={source} connection_id={connection_id} session_id={session_id} reason=disabled",
            ));
            return false;
        }

        let connection_id = self.inner.connection_id();
        let probe_attempts = attempts.max(1);

        for attempt in 1..=probe_attempts {
            if self.driver_handle_raw.load(Ordering::SeqCst) != 0 {
                debug_log_line(&format!(
                    "SESSION_PROOF_PROVIDER_IDD_DIAGNOSTIC_PROBE_SKIPPED source={source} connection_id={connection_id} session_id={session_id} reason=driver_handle_already_from_on_driver_load",
                ));
                return true;
            }

            let open_result = self
                .open_ironrdp_idd_from_query_dos_device(Some(session_id))
                .or_else(|_| self.open_ironrdp_idd_from_setupapi_interface(Some(session_id)))
                .or_else(|_| self.open_ironrdp_idd_from_setupapi_pdo(Some(session_id)));

            match open_result {
                Ok((handle, path)) => {
                    unsafe {
                        let _ = CloseHandle(handle);
                    }

                    debug_log_line(&format!(
                        "SESSION_PROOF_PROVIDER_IDD_DIAGNOSTIC_PROBE source={source} connection_id={connection_id} session_id={session_id} attempt={attempt} result=open_ok handle_nonzero=true path={path}",
                    ));
                    return true;
                }
                Err(error) => {
                    debug_log_line(&format!(
                        "SESSION_PROOF_PROVIDER_IDD_DIAGNOSTIC_PROBE_RETRY source={source} connection_id={connection_id} session_id={session_id} attempt={attempt} error={error}",
                    ));

                    if attempt == probe_attempts || !ComProtocolConnection::is_transient_idd_open_error(&error) {
                        break;
                    }

                    thread::sleep(retry_delay);
                }
            }
        }

        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_IDD_DIAGNOSTIC_PROBE_SKIPPED source={source} connection_id={connection_id} session_id={session_id} reason=no_driver_handle_yet_after_probe",
        ));

        false
    }

    fn open_device_handle_raw(path: &str, desired_access: u32) -> windows_core::Result<HANDLE> {
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();

        unsafe {
            // SAFETY: `wide` is NUL-terminated and lives for the duration of the call.
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                desired_access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }
    }

    fn open_device_handle_with_access(path: &str, desired_access: u32) -> windows_core::Result<HANDLE> {
        Self::with_process_thread_identity("open_device_handle_with_access", || {
            Self::open_device_handle_raw(path, desired_access)
        })
    }

    fn with_process_thread_identity<T, F>(operation_name: &str, operation: F) -> windows_core::Result<T>
    where
        F: FnOnce() -> windows_core::Result<T>,
    {
        let mut thread_token = HANDLE::default();
        match unsafe {
            OpenThreadToken(
                GetCurrentThread(),
                TOKEN_QUERY | TOKEN_DUPLICATE,
                true,
                &mut thread_token,
            )
        } {
            Ok(()) => {
                let mut restore_token = HANDLE::default();
                let duplicate_result = unsafe {
                    DuplicateTokenEx(
                        thread_token,
                        TOKEN_IMPERSONATE | TOKEN_QUERY,
                        None,
                        SecurityImpersonation,
                        TokenImpersonation,
                        &mut restore_token,
                    )
                };

                match duplicate_result {
                    Ok(()) => {
                        debug_log_line(&format!(
                            "{operation_name}: reverting impersonated thread token before device open"
                        ));
                        unsafe { RevertToSelf()? };

                        let result = operation();

                        if let Err(error) = unsafe { SetThreadToken(None, Some(restore_token)) } {
                            debug_log_line(&format!(
                                "{operation_name}: failed to restore impersonated thread token: {error}"
                            ));
                        } else {
                            debug_log_line(&format!(
                                "{operation_name}: restored impersonated thread token after device open"
                            ));
                        }

                        unsafe {
                            let _ = CloseHandle(restore_token);
                            let _ = CloseHandle(thread_token);
                        }

                        return result;
                    }
                    Err(error) => {
                        debug_log_line(&format!(
                            "{operation_name}: failed to duplicate impersonated thread token for restore: {error}"
                        ));
                    }
                }

                unsafe {
                    let _ = CloseHandle(thread_token);
                }
            }
            Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_TOKEN.0) => {}
            Err(error) => {
                debug_log_line(&format!(
                    "{operation_name}: OpenThreadToken did not yield a restorable impersonation token: {error}"
                ));
            }
        }

        operation()
    }

    fn with_cached_session_thread_identity<T, F>(
        &self,
        operation_name: &str,
        session_id: u32,
        operation: F,
    ) -> windows_core::Result<T>
    where
        F: FnOnce() -> windows_core::Result<T>,
    {
        let cached_token = match self.duplicate_cached_session_impersonation_token(session_id) {
            Ok(token) => token,
            Err(error) => {
                debug_log_line(&format!(
                    "{operation_name}: no cached session impersonation token for session_id={session_id}: {error}",
                ));
                return Self::with_process_thread_identity(operation_name, operation);
            }
        };

        let mut thread_token = HANDLE::default();
        let existing_thread_token = match unsafe {
            OpenThreadToken(
                GetCurrentThread(),
                TOKEN_QUERY | TOKEN_DUPLICATE,
                true,
                &mut thread_token,
            )
        } {
            Ok(()) => Some(thread_token),
            Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_TOKEN.0) => None,
            Err(error) => {
                debug_log_line(&format!(
                    "{operation_name}: OpenThreadToken did not yield a restorable token before session impersonation: {error}",
                ));
                None
            }
        };

        let restore_token = match existing_thread_token {
            Some(thread_token) => {
                match Self::duplicate_impersonation_token(thread_token, TOKEN_IMPERSONATE | TOKEN_QUERY) {
                    Ok(token) => Some(token),
                    Err(error) => {
                        debug_log_line(&format!(
                            "{operation_name}: failed to duplicate existing thread token before session impersonation: {error}",
                        ));
                        None
                    }
                }
            }
            None => None,
        };

        debug_log_line(&format!(
            "{operation_name}: impersonating cached session token for session_id={session_id} before device open",
        ));

        unsafe { SetThreadToken(None, Some(cached_token))? };
        let result = operation();

        match restore_token {
            Some(token) => {
                if let Err(error) = unsafe { SetThreadToken(None, Some(token)) } {
                    debug_log_line(&format!(
                        "{operation_name}: failed to restore prior thread token after session impersonation: {error}",
                    ));
                } else {
                    debug_log_line(&format!(
                        "{operation_name}: restored prior thread token after session impersonation"
                    ));
                }

                unsafe {
                    let _ = CloseHandle(token);
                }
            }
            None => {
                if let Err(error) = unsafe { RevertToSelf() } {
                    debug_log_line(&format!(
                        "{operation_name}: failed to revert thread token after session impersonation: {error}",
                    ));
                } else {
                    debug_log_line(&format!(
                        "{operation_name}: reverted thread token after session impersonation"
                    ));
                }
            }
        }

        unsafe {
            let _ = CloseHandle(cached_token);
            if let Some(thread_token) = existing_thread_token {
                let _ = CloseHandle(thread_token);
            }
        }

        result
    }

    fn open_device_handle_with_session_access(
        &self,
        path: &str,
        desired_access: u32,
        session_id: u32,
    ) -> windows_core::Result<HANDLE> {
        self.with_cached_session_thread_identity("open_device_handle_with_session_access", session_id, || {
            Self::open_device_handle_raw(path, desired_access)
        })
    }

    fn open_first_device_handle(candidates: &[(&'static str, u32)]) -> windows_core::Result<HANDLE> {
        Self::open_first_device_handle_with_path(candidates).map(|(handle, _)| handle)
    }

    fn open_first_device_handle_with_path(
        candidates: &[(&'static str, u32)],
    ) -> windows_core::Result<(HANDLE, &'static str)> {
        let mut last_error: Option<windows_core::Error> = None;
        for (path, access) in candidates {
            match Self::open_device_handle_with_access(path, *access) {
                Ok(handle) => return Ok((handle, path)),
                Err(error) => last_error = Some(error),
            }
        }

        Err(last_error.unwrap_or_else(|| windows_core::Error::new(E_UNEXPECTED, "no device handle candidates")))
    }

    fn parse_multi_sz(buffer: &[u16]) -> Vec<String> {
        let mut values = Vec::new();
        let mut start = 0usize;

        for (index, ch) in buffer.iter().enumerate() {
            if *ch != 0 {
                continue;
            }

            if index == start {
                break;
            }

            values.push(String::from_utf16_lossy(&buffer[start..index]));
            start = index + 1;
        }

        values
    }

    fn query_dos_device_targets(device_name: &str) -> windows_core::Result<Vec<String>> {
        let mut buffer_len: u32 = 1024;
        let device_name_utf16: Vec<u16> = device_name.encode_utf16().chain(Some(0)).collect();

        loop {
            let mut buffer = vec![0u16; usize::try_from(buffer_len).unwrap_or(1024)];

            // SAFETY: `device_name_utf16` is NUL-terminated and `buffer` is writable for the call duration.
            let written = unsafe { QueryDosDeviceW(PCWSTR(device_name_utf16.as_ptr()), Some(&mut buffer)) };

            if written != 0 {
                return Ok(Self::parse_multi_sz(&buffer));
            }

            let error = windows_core::Error::from_thread();
            if error.code() == HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0) && buffer_len < 65_536 {
                buffer_len = buffer_len.saturating_mul(2);
                continue;
            }

            return Err(error);
        }
    }

    fn open_ironrdp_idd_from_query_dos_device(
        &self,
        session_id: Option<u32>,
    ) -> windows_core::Result<(HANDLE, String)> {
        let mut last_error: Option<windows_core::Error> = None;
        let rw = (windows::Win32::Storage::FileSystem::FILE_GENERIC_READ
            | windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE)
            .0;
        let access_modes = [rw, windows::Win32::Storage::FileSystem::FILE_GENERIC_READ.0, 0];

        for dos_name in ["IronRdpIddVideo", r"Global\IronRdpIddVideo"] {
            match Self::query_dos_device_targets(dos_name) {
                Ok(targets) => {
                    if targets.is_empty() {
                        debug_log_line(&format!("GetVideoHandle: QueryDosDevice no targets for {dos_name}"));
                        continue;
                    }

                    debug_log_line(&format!(
                        "GetVideoHandle: QueryDosDevice targets for {dos_name}: {}",
                        targets.join(" | "),
                    ));

                    for target in targets {
                        let normalized_target = target.trim_matches('\0').trim();
                        if normalized_target.is_empty() {
                            continue;
                        }

                        for prefix in [r"\\?\GLOBALROOT", r"\\.\GLOBALROOT"] {
                            let open_path = format!("{prefix}{normalized_target}");
                            for access in access_modes {
                                let open_result = session_id.map_or_else(
                                    || Self::open_device_handle_with_access(&open_path, access),
                                    |session_id| {
                                        self.open_device_handle_with_session_access(&open_path, access, session_id)
                                    },
                                );
                                match open_result {
                                    Ok(handle) => {
                                        return Ok((
                                            handle,
                                            format!("querydosdevice:{dos_name}->{open_path}:access=0x{access:08X}"),
                                        ));
                                    }
                                    Err(error) => {
                                        last_error = Some(error);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    debug_log_line(&format!(
                        "GetVideoHandle: QueryDosDevice failed for {dos_name}: {error}"
                    ));
                    last_error = Some(error);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            windows_core::Error::new(E_UNEXPECTED, "no QueryDosDevice mapping available for IronRdpIddVideo")
        }))
    }

    fn setupdi_get_registry_property_strings(
        device_info_set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
        device_info_data: &SP_DEVINFO_DATA,
        property: SETUP_DI_REGISTRY_PROPERTY,
    ) -> windows_core::Result<Vec<String>> {
        let mut required_size = 0u32;
        // SAFETY: probing required size with null output buffers follows SetupAPI contract.
        let first_try = unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                device_info_set,
                device_info_data,
                property,
                None,
                None,
                Some(&mut required_size),
            )
        };

        match first_try {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0) => {}
            Err(error) => return Err(error),
        }

        if required_size == 0 {
            return Ok(Vec::new());
        }

        let mut buffer = vec![0u8; usize::try_from(required_size).unwrap_or(0)];
        // SAFETY: `buffer` is writable and sized from SetupAPI-provided required size.
        unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                device_info_set,
                device_info_data,
                property,
                None,
                Some(&mut buffer),
                Some(&mut required_size),
            )?;
        }

        if buffer.len() < 2 {
            return Ok(Vec::new());
        }

        let wide: Vec<u16> = buffer
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        Ok(Self::parse_multi_sz(&wide))
    }

    fn setupdi_get_device_instance_id(
        device_info_set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
        device_info_data: &SP_DEVINFO_DATA,
    ) -> windows_core::Result<String> {
        let mut required_size = 0u32;
        let required_size_ptr = &mut required_size as *mut u32;
        let first_try =
            unsafe { SetupDiGetDeviceInstanceIdW(device_info_set, device_info_data, None, Some(required_size_ptr)) };

        match first_try {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0) => {}
            Err(error) => return Err(error),
        }

        if required_size == 0 {
            return Err(windows_core::Error::new(
                E_UNEXPECTED,
                "SetupAPI returned empty size for device instance id",
            ));
        }

        let mut buffer = vec![0u16; usize::try_from(required_size).unwrap_or(0)];
        let required_size_ptr = &mut required_size as *mut u32;
        unsafe {
            SetupDiGetDeviceInstanceIdW(
                device_info_set,
                device_info_data,
                Some(buffer.as_mut_slice()),
                Some(required_size_ptr),
            )?;
        }

        let end = buffer.iter().position(|ch| *ch == 0).unwrap_or(buffer.len());
        Ok(String::from_utf16_lossy(&buffer[..end]))
    }

    fn setupapi_instance_matches(instance_id: &str, session_id: Option<u32>) -> bool {
        let instance_id_lower = instance_id.to_ascii_lowercase();
        let matches_ironrdp_idd = IRONRDP_IDD_DEVICE_MARKERS
            .iter()
            .any(|marker| instance_id_lower.contains(&marker.to_ascii_lowercase()));
        if !matches_ironrdp_idd {
            return false;
        }

        let session_marker_padded = session_id.map(|value| format!("sessionid_{value:04}"));
        let session_marker_plain = session_id.map(|value| format!("sessionid_{value}"));

        match (session_marker_padded, session_marker_plain) {
            (Some(padded), Some(plain)) => instance_id_lower.contains(&padded) || instance_id_lower.contains(&plain),
            _ => true,
        }
    }

    fn open_ironrdp_idd_from_setupapi_interface(
        &self,
        session_id: Option<u32>,
    ) -> windows_core::Result<(HANDLE, String)> {
        let rw = (windows::Win32::Storage::FileSystem::FILE_GENERIC_READ
            | windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE)
            .0;
        let access_modes = [rw, windows::Win32::Storage::FileSystem::FILE_GENERIC_READ.0, 0];
        let mut last_error: Option<windows_core::Error> = None;

        for flags in [DIGCF_PRESENT | DIGCF_DEVICEINTERFACE, DIGCF_DEVICEINTERFACE] {
            let flags_raw = flags.0;
            let device_info_set = unsafe {
                SetupDiGetClassDevsW(Some(&GUID_DEVINTERFACE_IRONRDP_IDD_VIDEO), PCWSTR::null(), None, flags)?
            };

            let result = (|| {
                let mut interface_index = 0u32;
                let mut scoped_last_error: Option<windows_core::Error> = None;

                loop {
                    let mut interface_data = SP_DEVICE_INTERFACE_DATA {
                        cbSize: u32::try_from(size_of::<SP_DEVICE_INTERFACE_DATA>()).unwrap_or(0),
                        ..Default::default()
                    };
                    let interface_result = unsafe {
                        SetupDiEnumDeviceInterfaces(
                            device_info_set,
                            None,
                            &GUID_DEVINTERFACE_IRONRDP_IDD_VIDEO,
                            interface_index,
                            &mut interface_data,
                        )
                    };

                    match interface_result {
                        Ok(()) => {}
                        Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_MORE_ITEMS.0) => break,
                        Err(error) => {
                            scoped_last_error = Some(error);
                            break;
                        }
                    }

                    interface_index = interface_index.saturating_add(1);

                    let mut required_size = 0u32;
                    let mut device_info_data = SP_DEVINFO_DATA {
                        cbSize: u32::try_from(size_of::<SP_DEVINFO_DATA>()).unwrap_or(0),
                        ..Default::default()
                    };
                    let first_try = unsafe {
                        SetupDiGetDeviceInterfaceDetailW(
                            device_info_set,
                            &interface_data,
                            None,
                            0,
                            Some(&mut required_size as *mut u32),
                            Some(&mut device_info_data as *mut SP_DEVINFO_DATA),
                        )
                    };

                    match first_try {
                        Ok(()) => {}
                        Err(error) if error.code() == HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0) => {}
                        Err(error) => {
                            scoped_last_error = Some(error);
                            continue;
                        }
                    }

                    if required_size == 0 {
                        continue;
                    }

                    let instance_id = match Self::setupdi_get_device_instance_id(device_info_set, &device_info_data) {
                        Ok(value) => value,
                        Err(error) => {
                            scoped_last_error = Some(error);
                            continue;
                        }
                    };

                    if !Self::setupapi_instance_matches(&instance_id, session_id) {
                        continue;
                    }

                    let detail_word_count = usize::try_from(required_size).unwrap_or(0).div_ceil(size_of::<u32>());
                    let mut detail_buffer = vec![0u32; detail_word_count];
                    let detail_ptr = detail_buffer.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();

                    // SAFETY: detail_ptr points to `detail_buffer`, which is writable for `required_size` bytes.
                    unsafe {
                        (*detail_ptr).cbSize =
                            u32::try_from(size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>()).unwrap_or(0);
                    }

                    let detail_result = unsafe {
                        SetupDiGetDeviceInterfaceDetailW(
                            device_info_set,
                            &interface_data,
                            Some(detail_ptr),
                            required_size,
                            Some(&mut required_size as *mut u32),
                            Some(&mut device_info_data as *mut SP_DEVINFO_DATA),
                        )
                    };

                    if let Err(error) = detail_result {
                        scoped_last_error = Some(error);
                        continue;
                    }

                    let device_path = unsafe {
                        let ptr = (*detail_ptr).DevicePath.as_ptr();
                        let mut len = 0usize;
                        while *ptr.add(len) != 0 {
                            len = len.saturating_add(1);
                        }

                        String::from_utf16_lossy(core::slice::from_raw_parts(ptr, len))
                    };

                    if device_path.is_empty() {
                        continue;
                    }

                    debug_log_line(&format!(
                        "GetVideoHandle: SetupAPI interface matched instance_id={} device_path={} session_filter={:?} flags=0x{flags_raw:08X}",
                        instance_id, device_path, session_id,
                    ));

                    for access in access_modes {
                        let open_result = session_id.map_or_else(
                            || Self::open_device_handle_with_access(&device_path, access),
                            |session_id| self.open_device_handle_with_session_access(&device_path, access, session_id),
                        );
                        match open_result {
                            Ok(handle) => {
                                return Ok((
                                    handle,
                                    format!(
                                        "setupapi_interface:{device_path}:access=0x{access:08X}:flags=0x{flags_raw:08X}"
                                    ),
                                ));
                            }
                            Err(error) => {
                                scoped_last_error = Some(error);
                            }
                        }
                    }
                }

                Err(scoped_last_error.unwrap_or_else(|| {
                    windows_core::Error::new(
                        HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0),
                        "no SetupAPI device interface available for IronRdpIdd marker/session",
                    )
                }))
            })();

            // SAFETY: `device_info_set` was returned by SetupDiGetClassDevsW and must be destroyed exactly once.
            unsafe {
                let _ = SetupDiDestroyDeviceInfoList(device_info_set);
            }

            match result {
                Ok(value) => return Ok(value),
                Err(error) => {
                    debug_log_line(&format!(
                        "GetVideoHandle: SetupAPI interface enumeration failed flags=0x{flags_raw:08X} error={error}"
                    ));
                    last_error = Some(error);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            windows_core::Error::new(
                HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0),
                "no SetupAPI device interface available for IronRdpIdd marker/session",
            )
        }))
    }

    fn open_ironrdp_idd_from_setupapi_pdo(&self, session_id: Option<u32>) -> windows_core::Result<(HANDLE, String)> {
        let rw = (windows::Win32::Storage::FileSystem::FILE_GENERIC_READ
            | windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE)
            .0;
        let access_modes = [rw, windows::Win32::Storage::FileSystem::FILE_GENERIC_READ.0, 0];

        let device_info_set =
            unsafe { SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_ALLCLASSES | DIGCF_PRESENT)? };

        let result = (|| {
            let mut index = 0u32;
            let mut last_error: Option<windows_core::Error> = None;

            loop {
                let mut device_info_data = SP_DEVINFO_DATA {
                    cbSize: u32::try_from(size_of::<SP_DEVINFO_DATA>()).unwrap_or(0),
                    ..Default::default()
                };

                let enum_result = unsafe { SetupDiEnumDeviceInfo(device_info_set, index, &mut device_info_data) };

                match enum_result {
                    Ok(()) => {}
                    Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_MORE_ITEMS.0) => break,
                    Err(error) => {
                        last_error = Some(error);
                        break;
                    }
                }

                index = index.saturating_add(1);

                let instance_id = match Self::setupdi_get_device_instance_id(device_info_set, &device_info_data) {
                    Ok(value) => value,
                    Err(error) => {
                        last_error = Some(error);
                        continue;
                    }
                };

                if !Self::setupapi_instance_matches(&instance_id, session_id) {
                    continue;
                }

                debug_log_line(&format!(
                    "GetVideoHandle: SetupAPI matched device instance id {} (session_filter={:?})",
                    instance_id, session_id,
                ));

                let pdo_names = match Self::setupdi_get_registry_property_strings(
                    device_info_set,
                    &device_info_data,
                    SPDRP_PHYSICAL_DEVICE_OBJECT_NAME,
                ) {
                    Ok(names) => names,
                    Err(error) => {
                        last_error = Some(error);
                        continue;
                    }
                };

                for pdo_name in pdo_names {
                    let normalized_pdo_name = pdo_name.trim_matches('\0').trim();
                    if normalized_pdo_name.is_empty() {
                        continue;
                    }

                    for prefix in [r"\\?\GLOBALROOT", r"\\.\GLOBALROOT"] {
                        let open_path = format!("{prefix}{normalized_pdo_name}");
                        debug_log_line(&format!(
                            "GetVideoHandle: SetupAPI matched device_markers={:?} pdo_name={} open_path={}",
                            IRONRDP_IDD_DEVICE_MARKERS, normalized_pdo_name, open_path,
                        ));

                        for access in access_modes {
                            let open_result = session_id.map_or_else(
                                || Self::open_device_handle_with_access(&open_path, access),
                                |session_id| {
                                    self.open_device_handle_with_session_access(&open_path, access, session_id)
                                },
                            );
                            match open_result {
                                Ok(handle) => {
                                    return Ok((handle, format!("setupapi_pdo:{open_path}:access=0x{access:08X}")));
                                }
                                Err(error) => {
                                    last_error = Some(error);
                                }
                            }
                        }
                    }
                }
            }

            Err(last_error.unwrap_or_else(|| {
                windows_core::Error::new(
                    HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0),
                    "no SetupAPI PDO mapping available for IronRdpIdd device marker/session",
                )
            }))
        })();

        // SAFETY: `device_info_set` was returned by SetupDiGetClassDevsW and must be destroyed exactly once.
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(device_info_set);
        }

        result
    }

    fn is_transient_idd_open_error(error: &windows_core::Error) -> bool {
        let code = error.code();
        code == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0)
            || code == HRESULT::from_win32(ERROR_PATH_NOT_FOUND.0)
            || code == HRESULT::from_win32(0x1B1)
    }

    fn open_keyboard_device_handle() -> windows_core::Result<HANDLE> {
        // On some Windows Server builds, the Win32 \\.\KeyboardClassN links are missing,
        // but GLOBALROOT device paths are present and can be opened with 0 desired access.
        let rw = (windows::Win32::Storage::FileSystem::FILE_GENERIC_READ
            | windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE)
            .0;

        Self::open_first_device_handle(&[
            (r"\\.\KeyboardClass0", rw),
            (r"\\?\GLOBALROOT\Device\KeyboardClass0", 0),
            (r"\\?\GLOBALROOT\Device\KeyboardClass1", 0),
        ])
    }

    fn open_pointer_device_handle() -> windows_core::Result<HANDLE> {
        let rw = (windows::Win32::Storage::FileSystem::FILE_GENERIC_READ
            | windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE)
            .0;

        Self::open_first_device_handle(&[
            (r"\\.\PointerClass0", rw),
            (r"\\?\GLOBALROOT\Device\PointerClass0", 0),
            (r"\\?\GLOBALROOT\Device\PointerClass1", 0),
        ])
    }

    fn fetch_and_cache_connection_credentials(
        &self,
        connection_id: u32,
        source: &'static str,
    ) -> windows_core::Result<Option<(String, String, String)>> {
        let Some((username, domain, password)) = self.control_bridge.get_connection_credentials(connection_id)? else {
            debug_log_line(&format!(
                "fetch_and_cache_connection_credentials none connection_id={connection_id} source={source}",
            ));
            return Ok(None);
        };

        let (winlogon_username, winlogon_domain) = normalize_winlogon_credentials(&username, &domain);
        let credentials = (winlogon_username, winlogon_domain, password);
        *self.cached_credentials.lock() = Some(credentials.clone());

        debug_log_line(&format!(
            "fetch_and_cache_connection_credentials cached connection_id={connection_id} source={source} user={} domain={}",
            credentials.0, credentials.1,
        ));

        Ok(Some(credentials))
    }

    fn cached_connection_credentials(&self) -> Option<(String, String, String)> {
        self.cached_credentials.lock().clone()
    }

    fn connection_credentials(
        &self,
        connection_id: u32,
        source: &'static str,
    ) -> windows_core::Result<Option<(String, String, String)>> {
        match self.cached_connection_credentials() {
            Some(credentials) => Ok(Some(credentials)),
            None => self.fetch_and_cache_connection_credentials(connection_id, source),
        }
    }

    fn release_connection_callback(&self) {
        let callback = self.connection_callback.lock().take();
        drop(callback);
    }
}

impl IWRdsProtocolConnectionSettings_Impl for ComProtocolConnection_Impl {
    fn SetConnectionSetting(
        &self,
        _propertyid: &GUID,
        _ppropertyentriesin: *const WTS_PROPERTY_VALUE,
    ) -> windows_core::Result<()> {
        debug_log_line("IWRdsProtocolConnectionSettings::SetConnectionSetting called");
        Ok(())
    }

    fn GetConnectionSetting(
        &self,
        _propertyid: &GUID,
        _ppropertyentriesout: *mut WTS_PROPERTY_VALUE,
    ) -> windows_core::Result<()> {
        debug_log_line("IWRdsProtocolConnectionSettings::GetConnectionSetting called");
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "GetConnectionSetting not implemented",
        ))
    }
}

impl IWRdsWddmIddProps_Impl for ComProtocolConnection_Impl {
    fn GetHardwareId(&self, pdisplaydriverhardwareid: &PCWSTR, count: u32) -> windows_core::Result<()> {
        ComProtocolConnection::fill_idd_hardware_id(pdisplaydriverhardwareid, count)
    }

    fn OnDriverLoad(&self, sessionid: u32, driverhandle: HANDLE_PTR) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsWddmIddProps::OnDriverLoad session_id={sessionid} handle={driverhandle:?}",
        ));
        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_IDD_ON_DRIVER_LOAD connection_id={connection_id} session_id={sessionid} handle_nonzero={} callback=props",
            driverhandle.0 != 0,
        ));

        self.close_cached_video_handle();
        self.commit_driver_load_state(sessionid, driverhandle.0, connection_id, "props", None, None);
        self.maybe_notify_driver_load(sessionid, connection_id);
        Ok(())
    }

    fn OnDriverUnload(&self, sessionid: u32) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!("IWRdsWddmIddProps::OnDriverUnload session_id={sessionid}"));
        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_IDD_ON_DRIVER_UNLOAD connection_id={connection_id} session_id={sessionid} callback=props",
        ));
        self.clear_driver_load_state(connection_id, sessionid, "props");
        Ok(())
    }

    fn EnableWddmIdd(&self, enabled: BOOL) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        let enabled_bool = enabled.as_bool();
        debug_log_line(&format!("IWRdsWddmIddProps::EnableWddmIdd enabled={}", enabled_bool));
        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_IDD_WDDM_ENABLE connection_id={connection_id} enabled={enabled_bool}",
        ));

        self.wddm_idd_enabled.store(enabled_bool, Ordering::SeqCst);
        update_idd_runtime_state(
            &[
                ("wddm_idd_enabled", enabled_bool.to_string()),
                ("provider_connection_id", connection_id.to_string()),
            ],
            "enable_wddm_idd",
        );
        Ok(())
    }
}

impl IWRdsWddmIddProps1_Impl for ComProtocolConnection_Impl {
    fn GetHardwareId(&self, pdisplaydriverhardwareid: &PCWSTR, count: u32) -> windows_core::Result<()> {
        ComProtocolConnection::fill_idd_hardware_id(pdisplaydriverhardwareid, count)
    }

    fn OnDriverLoad(&self, sessionid: u32, deviceinstance: &PCWSTR) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        let reported_device_instance = if deviceinstance.is_null() {
            String::new()
        } else {
            unsafe { deviceinstance.to_string() }.unwrap_or_default()
        };

        debug_log_line(&format!(
            "IWRdsWddmIddProps1::OnDriverLoad session_id={sessionid} device_instance={reported_device_instance}",
        ));

        self.close_cached_video_handle();
        let diagnostic_probe_enabled = provider_props1_driver_probe_enabled();
        let mut diagnostic_probe_succeeded = false;
        let mut diagnostic_open_path = None;

        if reported_device_instance.is_empty() {
            debug_log_line(&format!(
                "SESSION_PROOF_PROVIDER_IDD_ON_DRIVER_LOAD_ERROR connection_id={connection_id} session_id={sessionid} callback=props1 reason=empty_device_instance",
            ));
        } else if diagnostic_probe_enabled {
            match self.open_ironrdp_idd_from_driver_callback_device_instance(sessionid, &reported_device_instance) {
                Ok((handle, path)) => {
                    diagnostic_probe_succeeded = true;
                    diagnostic_open_path = Some(path);
                    unsafe {
                        let _ = CloseHandle(handle);
                    }
                }
                Err(error) => {
                    debug_log_line(&format!(
                        "SESSION_PROOF_PROVIDER_IDD_ON_DRIVER_LOAD_DIAGNOSTIC_ERROR connection_id={connection_id} session_id={sessionid} callback=props1 device_instance={reported_device_instance} error={error}",
                    ));
                }
            }
        }

        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_IDD_ON_DRIVER_LOAD connection_id={connection_id} session_id={sessionid} handle_nonzero=false callback=props1 device_instance={reported_device_instance} diagnostic_probe_enabled={} diagnostic_probe_succeeded={} diagnostic_open_path={}",
            diagnostic_probe_enabled,
            diagnostic_probe_succeeded,
            diagnostic_open_path.as_deref().unwrap_or("none"),
        ));

        self.commit_driver_load_state(
            sessionid,
            0,
            connection_id,
            "props1",
            if reported_device_instance.is_empty() {
                None
            } else {
                Some(reported_device_instance.as_str())
            },
            None,
        );
        Ok(())
    }

    fn OnDriverUnload(&self, sessionid: u32) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!("IWRdsWddmIddProps1::OnDriverUnload session_id={sessionid}"));
        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_IDD_ON_DRIVER_UNLOAD connection_id={connection_id} session_id={sessionid} callback=props1",
        ));
        self.clear_driver_load_state(connection_id, sessionid, "props1");
        Ok(())
    }
}

impl IWRdsProtocolConnection2_Impl for ComProtocolConnection_Impl {
    fn GetSerializedUserCredential(
        &self,
        usercredential: *mut *mut WRDS_SERIALIZED_USER_CREDENTIAL,
    ) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsProtocolConnection2::GetSerializedUserCredential called connection_id={connection_id}",
        ));

        if usercredential.is_null() {
            return Err(windows_core::Error::new(
                E_POINTER,
                "null serialized credential pointer",
            ));
        }

        // SAFETY: caller provided a writable out-parameter buffer.
        unsafe {
            *usercredential = core::ptr::null_mut();
        }

        let (credential_ptr, packed_size) =
            self.pack_cached_user_credentials(connection_id, "get_serialized_user_credential")?;

        // SAFETY: caller provided a writable out-parameter buffer.
        unsafe {
            *usercredential = credential_ptr;
        }

        debug_log_line(&format!(
            "IWRdsProtocolConnection2::GetSerializedUserCredential ok connection_id={connection_id} packed_bytes={packed_size}",
        ));
        Ok(())
    }
}

impl IWRdsEnhancedFastReconnectArbitrator_Impl for ComProtocolConnection_Impl {
    fn GetSessionForEnhancedFastReconnect(
        &self,
        psessionidarray: *const i32,
        dwsessioncount: u32,
        presultsessionid: *mut i32,
    ) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsEnhancedFastReconnectArbitrator::GetSessionForEnhancedFastReconnect called connection_id={connection_id} count={dwsessioncount}",
        ));

        if presultsessionid.is_null() {
            return Err(windows_core::Error::new(
                E_POINTER,
                "null enhanced fast reconnect result pointer",
            ));
        }

        unsafe {
            *presultsessionid = -1;
        }

        if dwsessioncount == 0 {
            debug_log_line(&format!(
                "IWRdsEnhancedFastReconnectArbitrator::GetSessionForEnhancedFastReconnect no_candidates connection_id={connection_id}",
            ));
            return Err(windows_core::Error::new(
                E_NOTIMPL,
                "no enhanced fast reconnect candidates",
            ));
        }

        if psessionidarray.is_null() {
            return Err(windows_core::Error::new(
                E_POINTER,
                "null enhanced fast reconnect session array",
            ));
        }

        let count = usize::try_from(dwsessioncount).unwrap_or(0);
        let candidates = unsafe { core::slice::from_raw_parts(psessionidarray, count) };

        let mut selected: Option<(u32, &'static str)> = None;
        let mut summary = Vec::with_capacity(candidates.len());

        for candidate in candidates {
            let raw_session_id = u32::try_from(*candidate).unwrap_or(u32::MAX);
            let session_id = canonicalize_enumerated_session_id(raw_session_id);
            let has_token = session_id != u32::MAX && session_has_user_token(session_id);
            let has_explorer = session_id != u32::MAX && session_has_process(session_id, "explorer.exe");
            let has_winlogon = session_id != u32::MAX && session_has_process(session_id, "winlogon.exe");
            let has_logonui = session_id != u32::MAX && session_has_process(session_id, "LogonUI.exe");
            let interactive = session_id != u32::MAX && session_is_interactive(session_id);

            summary.push(format!(
                "{raw_session_id}->{session_id}:interactive={interactive},token={has_token},explorer={has_explorer},winlogon={has_winlogon},logonui={has_logonui}"
            ));

            if session_id == u32::MAX {
                continue;
            }

            if interactive {
                selected = Some((session_id, "interactive"));
                break;
            }

            if has_token && selected.is_none() {
                selected = Some((session_id, "token"));
            }
        }

        debug_log_line(&format!(
            "IWRdsEnhancedFastReconnectArbitrator::GetSessionForEnhancedFastReconnect candidates connection_id={connection_id} values=[{}]",
            summary.join("; "),
        ));

        let Some((session_id, selected_reason)) = selected else {
            debug_log_line(&format!(
                "IWRdsEnhancedFastReconnectArbitrator::GetSessionForEnhancedFastReconnect no_valid_candidate connection_id={connection_id}",
            ));
            return Err(windows_core::Error::new(
                E_NOTIMPL,
                "no valid enhanced fast reconnect candidate",
            ));
        };

        unsafe {
            *presultsessionid = i32::try_from(session_id).unwrap_or(-1);
        }

        debug_log_line(&format!(
            "IWRdsEnhancedFastReconnectArbitrator::GetSessionForEnhancedFastReconnect selected connection_id={connection_id} session_id={session_id} reason={selected_reason}",
        ));

        Ok(())
    }
}

impl IWRdsProtocolConnection_Impl for ComProtocolConnection_Impl {
    fn GetLogonErrorRedirector(&self) -> windows_core::Result<IWRdsProtocolLogonErrorRedirector> {
        Ok(WrdsLogonErrorRedirector.into())
    }

    fn AcceptConnection(&self) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsProtocolConnection::AcceptConnection called connection_id={}",
            connection_id
        ));

        self.inner.accept_connection().map_err(transition_error)?;

        *self.ready_notified.lock() = true;

        debug_log_line(&format!(
            "IWRdsProtocolConnection::AcceptConnection ok connection_id={connection_id}",
        ));
        Ok(())
    }

    fn GetClientData(&self, pclientdata: *mut WTS_CLIENT_DATA) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetClientData called connection_id={}",
            self.inner.connection_id()
        ));

        if pclientdata.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null client data pointer"));
        }

        // SAFETY: pclientdata is non-null and points to writable memory owned by TermService.
        let client_data = unsafe { &mut *pclientdata };
        *client_data = WTS_CLIENT_DATA::default();
        // Match typical RDP defaults so Winlogon doesn't block on secure-attention requirements.
        client_data.fDisableCtrlAltDel = true;
        client_data.fMouse = true;
        client_data.fMaximizeShell = true;
        client_data.fEnableWindowsKey = true;
        // Only advertise auto-logon once we actually have CredSSP-derived credentials.
        // Otherwise, some TermService/Winlogon paths will attempt to use empty fields and fail.
        client_data.fInheritAutoLogon = BOOL(0);
        client_data.fUsingSavedCreds = false;
        client_data.fPromptForPassword = false;
        client_data.fNoAudioPlayback = true;
        // Advertise an explicit custom protocol name.
        client_data.ProtocolType = WTS_PROTOCOL_TYPE_NON_RDP;
        copy_wide(&mut client_data.ProtocolName, "RDP");

        // Some TermService/Winlogon code paths rely on the auto-logon credentials embedded in
        // WTS_CLIENT_DATA when fInheritAutoLogon is set. Populate these from the CredSSP-derived
        // credentials when available.
        //
        // The username/password are plaintext per Microsoft docs.
        let connection_id = self.inner.connection_id();
        let credentials = self.connection_credentials(connection_id, "get_client_data")?;

        match credentials {
            Some((winlogon_username, winlogon_domain, password)) => {
                client_data.fInheritAutoLogon = BOOL(1);
                client_data.fUsingSavedCreds = true;
                client_data.fPromptForPassword = false;

                copy_wide(&mut client_data.UserName, &winlogon_username);
                copy_wide(&mut client_data.Domain, &winlogon_domain);
                copy_wide(&mut client_data.Password, &password);

                debug_log_line(&format!(
                    "IWRdsProtocolConnection::GetClientData filled_autologon_creds connection_id={connection_id} user={winlogon_username} domain={winlogon_domain}"
                ));
            }
            None => {
                debug_log_line(&format!(
                    "IWRdsProtocolConnection::GetClientData no_autologon_creds_yet connection_id={connection_id}"
                ));
            }
        }

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let settings = loop {
            if let Some(settings) = self.control_bridge.remote_app_settings(connection_id)? {
                break settings;
            }
            if deadline <= std::time::Instant::now() {
                return Err(windows_core::Error::new(
                    ERROR_SEM_TIMEOUT.to_hresult(),
                    "Client Info is unavailable",
                ));
            }
            thread::sleep(Duration::from_millis(50));
        };
        if settings.client_info_flags != 0 {
            const INFO_MOUSE: u32 = 0x0000_0001;
            const INFO_DISABLECTRLALTDEL: u32 = 0x0000_0002;
            const INFO_MAXIMIZESHELL: u32 = 0x0000_0020;
            const INFO_ENABLEWINDOWSKEY: u32 = 0x0000_0100;
            const INFO_REMOTECONSOLEAUDIO: u32 = 0x0000_2000;
            const INFO_NOAUDIOPLAYBACK: u32 = 0x0008_0000;

            client_data.fMouse = (settings.client_info_flags & INFO_MOUSE) != 0;
            client_data.fDisableCtrlAltDel = (settings.client_info_flags & INFO_DISABLECTRLALTDEL) != 0;
            client_data.fMaximizeShell = (settings.client_info_flags & INFO_MAXIMIZESHELL) != 0;
            client_data.fEnableWindowsKey = (settings.client_info_flags & INFO_ENABLEWINDOWSKEY) != 0;
            client_data.fRemoteConsoleAudio = (settings.client_info_flags & INFO_REMOTECONSOLEAUDIO) != 0;
            client_data.fNoAudioPlayback = (settings.client_info_flags & INFO_NOAUDIOPLAYBACK) != 0;
        }
        client_data.PerformanceFlags = settings.performance_flags;
        client_data.KeyboardLayout = settings.keyboard_layout;
        client_data.ActiveInputLocale = settings.keyboard_layout;
        client_data.KeyboardType = settings.keyboard_type;
        client_data.KeyboardSubType = settings.keyboard_subtype;
        client_data.KeyboardFunctionKey = settings.keyboard_function_key;
        if !settings.ime_file_name.is_empty() {
            copy_wide(&mut client_data.imeFileName, &settings.ime_file_name);
        }
        if settings.desktop_width != 0 && settings.desktop_height != 0 {
            client_data.HRes = settings.desktop_width;
            client_data.VRes = settings.desktop_height;
            client_data.ColorDepth = 5;
        }
        if settings.enabled {
            let (directory, program) = remote_app_initial_program()?;
            if client_data.InitialProgram.len() <= program.encode_utf16().count()
                || client_data.WorkDirectory.len() <= directory.encode_utf16().count()
            {
                return Err(windows_core::Error::new(
                    E_INVALIDARG,
                    "RemoteApp bootstrap path is too long",
                ));
            }
            client_data.fInheritInitialProgram = BOOL(1);
            client_data.fMaximizeShell = false;
            copy_wide(&mut client_data.InitialProgram, &program);
            copy_wide(&mut client_data.WorkDirectory, &directory);
            debug_log_line(&format!(
                "Native RemoteApp bootstrap configured connection_id={connection_id}"
            ));
        }

        debug_log_line("IWRdsProtocolConnection::GetClientData ok");
        Ok(())
    }

    fn GetClientMonitorData(&self, pnummonitors: *mut u32, pprimarymonitor: *mut u32) -> windows_core::Result<()> {
        if !pnummonitors.is_null() {
            // SAFETY: `pnummonitors` is non-null and points to a writable buffer provided by the caller.
            unsafe { *pnummonitors = 1 };
        }

        if !pprimarymonitor.is_null() {
            // SAFETY: `pprimarymonitor` is non-null and points to a writable buffer provided by the caller.
            unsafe { *pprimarymonitor = 0 };
        }

        Ok(())
    }

    fn GetUserCredentials(&self, pusercreds: *mut WTS_USER_CREDENTIAL) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetUserCredentials called connection_id={connection_id}"
        ));

        if pusercreds.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null user credentials pointer"));
        }

        let credentials = self.connection_credentials(connection_id, "get_user_credentials")?;

        let Some((winlogon_username, winlogon_domain, password)) = credentials else {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::GetUserCredentials no_credentials connection_id={connection_id}"
            ));
            return Err(windows_core::Error::new(
                E_NOTIMPL,
                "no CredSSP credentials available yet",
            ));
        };

        // SAFETY: pusercreds is non-null and points to writable memory owned by TermService.
        let creds = unsafe { &mut *pusercreds };
        *creds = WTS_USER_CREDENTIAL::default();
        copy_wide(&mut creds.UserName, &winlogon_username);
        copy_wide(&mut creds.Domain, &winlogon_domain);
        copy_wide(&mut creds.Password, &password);

        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetUserCredentials ok connection_id={connection_id} user={winlogon_username} domain={winlogon_domain}",
        ));

        Ok(())
    }

    fn GetLicenseConnection(&self) -> windows_core::Result<IWRdsProtocolLicenseConnection> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetLicenseConnection called connection_id={}",
            self.inner.connection_id()
        ));
        // Stub license object; see WrdsLicenseConnection for why E_NOTIMPL would break the connection.
        let stub: IWRdsProtocolLicenseConnection = WrdsLicenseConnection.into();
        Ok(stub)
    }

    fn AuthenticateClientToSession(&self, sessionid: *mut WTS_SESSION_ID) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsProtocolConnection::AuthenticateClientToSession called connection_id={connection_id}",
        ));

        // Microsoft docs: this is an `[out]` parameter (WRDS_SESSION_ID*), not an in-out hint.
        // We currently do not implement fast reconnect / session reattachment.
        if sessionid.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null session id pointer"));
        }

        // SAFETY: `sessionid` is non-null and points to a writable out-parameter buffer.
        // Initialize it defensively even when returning E_NOTIMPL to avoid leaking stale data.
        unsafe {
            *sessionid = WTS_SESSION_ID::default();
        }

        debug_log_line(&format!(
            "IWRdsProtocolConnection::AuthenticateClientToSession returning E_NOTIMPL connection_id={connection_id}",
        ));

        Err(windows_core::Error::new(
            E_NOTIMPL,
            "AuthenticateClientToSession is not implemented",
        ))
    }

    fn NotifySessionId(
        &self,
        sessionid: *const WTS_SESSION_ID,
        _sessionhandle: HANDLE_PTR,
    ) -> windows_core::Result<()> {
        if sessionid.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null session id pointer"));
        }

        let wts_session_id = unsafe { (*sessionid).SessionId };
        debug_log_line(&format!(
            "IWRdsProtocolConnection::NotifySessionId called connection_id={} session_id={}",
            self.inner.connection_id(),
            wts_session_id
        ));
        self.inner.notify_session_id(wts_session_id).map_err(transition_error)?;

        if let Err(error) = self.control_bridge.set_capture_session_id_retried(
            self.inner.connection_id(),
            wts_session_id,
            "notify_session_id",
        ) {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::NotifySessionId set_capture_session_id failed connection_id={} session_id={} error={error}",
                self.inner.connection_id(),
                wts_session_id,
            ));
        } else {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::NotifySessionId set_capture_session_id ok connection_id={} session_id={}",
                self.inner.connection_id(),
                wts_session_id,
            ));
        }

        let (wddm_enabled, driver_handle_seen, driver_load_notified, driver_load_session_id) =
            self.idd_readiness_snapshot();
        let driver_load_session = driver_load_session_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned());

        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_IDD_NOTIFY_SESSION_ID connection_id={} session_id={} wddm_enabled={} driver_handle_seen={} driver_load_notified={} driver_load_session={}",
            self.inner.connection_id(),
            wts_session_id,
            wddm_enabled,
            driver_handle_seen,
            driver_load_notified,
            driver_load_session,
        ));
        update_idd_runtime_state(
            &[
                ("session_id", wts_session_id.to_string()),
                ("wddm_idd_enabled", wddm_enabled.to_string()),
                ("driver_loaded", driver_handle_seen.to_string()),
                ("provider_connection_id", self.inner.connection_id().to_string()),
            ],
            "notify_session_id",
        );

        Ok(())
    }

    fn GetInputHandles(
        &self,
        pkeyboardhandle: *mut HANDLE_PTR,
        pmousehandle: *mut HANDLE_PTR,
        pbeephandle: *mut HANDLE_PTR,
    ) -> windows_core::Result<()> {
        let keyboard_null = pkeyboardhandle.is_null();
        let mouse_null = pmousehandle.is_null();
        let beep_null = pbeephandle.is_null();

        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetInputHandles connection_id={} keyboard_null={} mouse_null={} beep_null={}",
            self.inner.connection_id(),
            keyboard_null,
            mouse_null,
            beep_null
        ));

        // Per docs, beep handle is unused and must be NULL.
        if !pbeephandle.is_null() {
            // SAFETY: non-null (checked above), valid TermService-provided pointer.
            unsafe { *pbeephandle = HANDLE_PTR::default() };
        }

        // TermService takes ownership of the returned handles and closes them itself
        // (CConnectionEx::GetTerminalChannels stores them in SmartHANDLE members).
        // Open fresh handles on every call and never close them here:
        // a second close would hit whatever object reuses the handle value.
        if !pkeyboardhandle.is_null() {
            let handle = match ComProtocolConnection::open_keyboard_device_handle() {
                Ok(handle) => {
                    debug_log_line("GetInputHandles: opened keyboard device handle");
                    Some(handle)
                }
                Err(error) => {
                    debug_log_line(&format!(
                        "GetInputHandles: failed to open keyboard device handle: {error}"
                    ));

                    // Some environments deny opening keyboard device objects from the TermService process.
                    // Provide a non-null fallback handle so TermService can still advance the session.
                    match ComProtocolConnection::open_pointer_device_handle() {
                        Ok(handle) => {
                            debug_log_line("GetInputHandles: using pointer device handle as keyboard fallback");
                            Some(handle)
                        }
                        Err(fallback_error) => {
                            debug_log_line(&format!(
                                "GetInputHandles: keyboard fallback (pointer device) failed: {fallback_error}"
                            ));
                            None
                        }
                    }
                }
            };
            let raw = handle.map(handle_to_raw_usize).unwrap_or_default();
            // SAFETY: non-null pointer, TermService-provided output.
            unsafe { *pkeyboardhandle = HANDLE_PTR(raw) };
        }

        if !pmousehandle.is_null() {
            let handle = match ComProtocolConnection::open_pointer_device_handle() {
                Ok(handle) => {
                    debug_log_line("GetInputHandles: opened pointer device handle");
                    Some(handle)
                }
                Err(error) => {
                    debug_log_line(&format!(
                        "GetInputHandles: failed to open pointer device handle: {error}"
                    ));
                    None
                }
            };
            let raw = handle.map(handle_to_raw_usize).unwrap_or_default();
            // SAFETY: non-null pointer, TermService-provided output.
            unsafe { *pmousehandle = HANDLE_PTR(raw) };
        }

        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetInputHandles ok connection_id={}",
            self.inner.connection_id()
        ));

        Ok(())
    }

    fn GetVideoHandle(&self) -> windows_core::Result<HANDLE_PTR> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetVideoHandle called connection_id={}",
            connection_id
        ));

        let driver_handle_raw = self.driver_handle_raw.load(Ordering::SeqCst);
        if driver_handle_raw != 0 {
            let (wddm_enabled, driver_handle_seen, driver_load_notified, driver_load_session_id) =
                self.idd_readiness_snapshot();
            let driver_load_session = driver_load_session_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_owned());

            *self.video_handle_source.lock() = VideoHandleSource::DriverLoadCallback;
            update_idd_runtime_video_source(connection_id, VideoHandleSource::DriverLoadCallback, None);
            debug_log_line(&format!(
                "SESSION_PROOF_PROVIDER_VIDEO_HANDLE_SELECTED connection_id={connection_id} source=driver_load_callback wddm_enabled={wddm_enabled} driver_handle_seen={driver_handle_seen} driver_load_notified={driver_load_notified} driver_load_session={driver_load_session}",
            ));

            return Ok(HANDLE_PTR(driver_handle_raw));
        }

        let (wddm_enabled, driver_handle_seen, driver_load_notified, driver_load_session_id) =
            self.idd_readiness_snapshot();
        let driver_load_session = driver_load_session_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned());

        if provider_video_handle_probe_enabled() {
            if let Some(session_id) = self.inner.session_id() {
                let _ = self.probe_idd_driver_load(
                    session_id,
                    "get_video_handle_pre_callback",
                    1,
                    Duration::from_millis(0),
                );
            } else {
                debug_log_line(&format!(
                    "SESSION_PROOF_PROVIDER_IDD_DIAGNOSTIC_PROBE_SKIPPED source=get_video_handle_pre_callback connection_id={connection_id} session_id=none reason=session_id_unavailable",
                ));
            }
        }

        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_VIDEO_HANDLE_UNAVAILABLE connection_id={connection_id} source=awaiting_on_driver_load wddm_enabled={wddm_enabled} driver_handle_seen={driver_handle_seen} driver_load_notified={driver_load_notified} driver_load_session={driver_load_session}",
        ));

        Ok(HANDLE_PTR(0))
    }

    fn ConnectNotify(&self, sessionid: u32) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsProtocolConnection::ConnectNotify called connection_id={connection_id} session_id={sessionid}"
        ));
        self.inner.connect_notify().map_err(transition_error)?;

        self.control_bridge
            .set_capture_session_id_async(connection_id, sessionid, "connect_notify");
        debug_log_line(&format!(
            "IWRdsProtocolConnection::ConnectNotify set_capture_session_id queued_async connection_id={connection_id} session_id={sessionid}"
        ));
        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_IDD_NOTIFY_DRIVER_LOAD_DEFERRED source=connect_notify connection_id={connection_id} session_id={sessionid} reason=pre_logon_path"
        ));

        Ok(())
    }

    fn IsUserAllowedToLogon(
        &self,
        sessionid: u32,
        usertoken: HANDLE_PTR,
        _pdomainname: &PCWSTR,
        _pusername: &PCWSTR,
    ) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        let domain_name = if _pdomainname.is_null() {
            "<null>".to_owned()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { _pdomainname.to_string() }.unwrap_or_else(|_| "<invalid>".to_owned())
        };

        let user_name = if _pusername.is_null() {
            "<null>".to_owned()
        } else {
            // SAFETY: TermService provides a valid NUL-terminated PCWSTR for the duration of this call.
            unsafe { _pusername.to_string() }.unwrap_or_else(|_| "<invalid>".to_owned())
        };

        // Per docs, UserToken is an input token handle created by TermService/Winlogon.
        // We can allow/deny the logon here, but we must NOT treat it as an out-parameter.
        debug_log_line(&format!(
            "IWRdsProtocolConnection::IsUserAllowedToLogon called connection_id={connection_id} session_id={sessionid} usertoken=0x{:X} user={domain_name}\\{user_name}",
            usertoken.0
        ));

        let has_userinit = session_has_process(sessionid, "userinit.exe");
        let has_explorer = session_has_process(sessionid, "explorer.exe");
        let has_logonui = session_has_process(sessionid, "LogonUI.exe");
        let has_winlogon = session_has_process(sessionid, "winlogon.exe");

        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_LOGON_GATE connection_id={connection_id} session_id={sessionid} usertoken_nonzero={} userinit={} explorer={} logonui={} winlogon={}",
            usertoken.0 != 0,
            has_userinit,
            has_explorer,
            has_logonui,
            has_winlogon,
        ));

        self.cache_session_impersonation_token(sessionid, usertoken);

        // Backup path: some hosts delay or skip NotifySessionId.  IsUserAllowedToLogon already
        // carries the target session ID, so forward it to the companion now to minimize time spent
        // on guessed-session capture.
        let bridge = self.control_bridge.clone();
        thread::spawn(move || {
            match bridge.set_capture_session_id_retried(connection_id, sessionid, "is_user_allowed_to_logon") {
                Ok(()) => {
                    debug_log_line(&format!(
                        "IWRdsProtocolConnection::IsUserAllowedToLogon set_capture_session_id ok connection_id={connection_id} session_id={sessionid}",
                    ));
                }
                Err(error) => {
                    debug_log_line(&format!(
                        "IWRdsProtocolConnection::IsUserAllowedToLogon set_capture_session_id failed connection_id={connection_id} session_id={sessionid} error={error}",
                    ));
                }
            }
        });

        Ok(())
    }

    fn SessionArbitrationEnumeration(
        &self,
        _husertoken: HANDLE_PTR,
        bsinglesessionperuserenabled: BOOL,
        _psessionidarray: *mut u32,
        _pdwsessionidentifiercount: *mut u32,
    ) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::SessionArbitrationEnumeration called connection_id={} single_session={}",
            self.inner.connection_id(),
            bsinglesessionperuserenabled.as_bool()
        ));
        // Return E_NOTIMPL so TermService uses its own default session-arbitration logic.
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
        let connection_id = self.inner.connection_id();
        debug_log_line(&format!(
            "IWRdsProtocolConnection::LogonNotify called connection_id={}",
            connection_id
        ));

        if !_sessionid.is_null() {
            // SAFETY: `_sessionid` is checked non-null and points to a TermService-provided
            // WTS_SESSION_ID valid for this callback invocation.
            let session_id = unsafe { (*_sessionid).SessionId };

            let bridge = self.control_bridge.clone();
            thread::spawn(move || {
                match bridge.set_capture_session_id_retried(connection_id, session_id, "logon_notify") {
                    Ok(()) => {
                        debug_log_line(&format!(
                            "IWRdsProtocolConnection::LogonNotify set_capture_session_id ok connection_id={connection_id} session_id={session_id}",
                        ));
                    }
                    Err(error) => {
                        debug_log_line(&format!(
                            "IWRdsProtocolConnection::LogonNotify set_capture_session_id failed connection_id={connection_id} session_id={session_id} error={error}",
                        ));
                    }
                }
            });

            if provider_video_handle_probe_enabled() {
                let _ = self.probe_idd_driver_load(session_id, "logon_notify", 20, Duration::from_millis(200));
            } else {
                debug_log_line(&format!(
                    "SESSION_PROOF_PROVIDER_IDD_NOTIFY_DRIVER_LOAD_DEFERRED source=logon_notify connection_id={connection_id} session_id={session_id} reason=awaiting_on_driver_load",
                ));
            }
        }

        self.inner.logon_notify().map_err(transition_error)?;
        Ok(())
    }

    fn PreDisconnect(&self, disconnectreason: u32) -> windows_core::Result<()> {
        // Do not enumerate sessions here: TermService holds its session lock during this callback,
        // so `WTSEnumerateSessionsW` RPCs back into TermService and deadlocks it.
        debug_log_line(&format!(
            "IWRdsProtocolConnection::PreDisconnect called connection_id={} reason={}",
            self.inner.connection_id(),
            disconnectreason,
        ));
        Ok(())
    }

    fn DisconnectNotify(&self) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::DisconnectNotify called connection_id={}",
            self.inner.connection_id()
        ));
        self.inner.disconnect_notify().map_err(transition_error)?;

        // Arbitration can reattach this connection to another session; Close owns transport and callback teardown.
        self.close_device_handles();
        Ok(())
    }

    fn Close(&self) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::Close called connection_id={}",
            self.inner.connection_id()
        ));

        if let Err(error) = self.control_bridge.close_connection(self.inner.connection_id()) {
            warn!(%error, connection_id = self.inner.connection_id(), "Failed to notify companion service on close");
        }

        *self.ready_notified.lock() = false;
        self.close_device_handles();
        let result = self.inner.close().map_err(transition_error);
        self.release_connection_callback();
        result
    }

    fn GetProtocolStatus(&self, pprotocolstatus: *mut WTS_PROTOCOL_STATUS) -> windows_core::Result<()> {
        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetProtocolStatus called connection_id={}",
            self.inner.connection_id()
        ));

        if pprotocolstatus.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null protocol status pointer"));
        }

        // SAFETY: `pprotocolstatus` is non-null (checked above) and points to a writable buffer.
        let status = unsafe { &mut *pprotocolstatus };
        *status = WTS_PROTOCOL_STATUS::default();

        // Keep ProtocolType/Length aligned with non-RDP provider mode and use
        // a conservative baseline for the opaque `Specific` counters.
        status.Output.ProtocolType = WTS_PROTOCOL_TYPE_NON_RDP;
        status.Output.Length =
            u16::try_from(size_of::<windows::Win32::System::RemoteDesktop::WTS_PROTOCOL_COUNTERS>()).unwrap_or(0);
        status.Output.Specific = 0;
        status.Input.ProtocolType = WTS_PROTOCOL_TYPE_NON_RDP;
        status.Input.Length = status.Output.Length;
        status.Input.Specific = status.Output.Specific;

        debug_log_line(&format!(
            "IWRdsProtocolConnection::GetProtocolStatus returned connection_id={} output={{type={},len={},specific={}}} input={{type={},len={},specific={}}}",
            self.inner.connection_id(),
            status.Output.ProtocolType,
            status.Output.Length,
            status.Output.Specific,
            status.Input.ProtocolType,
            status.Input.Length,
            status.Input.Specific,
        ));

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
        if szendpointname.is_null() {
            return Err(windows_core::Error::new(
                E_POINTER,
                "null virtual channel endpoint pointer",
            ));
        }

        // SAFETY: TermService supplies a valid NUL-terminated endpoint name.
        let endpoint_name = unsafe { szendpointname.to_string() }?;
        let is_static = bstatic.as_bool();
        if !native_endpoint_enabled(&endpoint_name, is_static) {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::CreateVirtualChannel denied connection_id={} endpoint={endpoint_name} static={is_static} priority={requestedpriority}",
                self.inner.connection_id(),
            ));
            debug!(%endpoint_name, requestedpriority, "Native virtual channel endpoint is not enabled");
            return Err(windows_core::Error::new(
                E_NOTIMPL,
                "native virtual channel endpoint is not enabled",
            ));
        }
        self.open_native_channel(&endpoint_name, is_static).map_err(|error| {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::CreateVirtualChannel failed connection_id={} endpoint={endpoint_name} static={is_static} error={error}",
                self.inner.connection_id(),
            ));
            if is_static {
                // TermService requests these while it sets up the session, and tolerates only this refusal.
                windows_core::Error::new(E_NOTIMPL, error.message())
            } else {
                error
            }
        })
    }

    fn QueryProperty(
        &self,
        _querytype: &GUID,
        _ulnumentriesin: u32,
        _ulnumentriesout: u32,
        _ppropertyentriesin: *const WTS_PROPERTY_VALUE,
        _ppropertyentriesout: *mut WTS_PROPERTY_VALUE,
    ) -> windows_core::Result<()> {
        if *_querytype == WTS_QUERY_ALLOWED_INITIAL_APP
            && (_ulnumentriesin != 3 || !matches!(_ulnumentriesout, 3 | 4) || _ppropertyentriesin.is_null())
        {
            return Err(windows_core::Error::new(
                E_INVALIDARG,
                "invalid initial-application query",
            ));
        }
        if _ulnumentriesout == 0 {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::QueryProperty called connection_id={} querytype={:?} in={} out=0",
                self.inner.connection_id(),
                _querytype,
                _ulnumentriesin,
            ));
            return Ok(());
        }

        if _ppropertyentriesout.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null property output pointer"));
        }

        let out_len = usize::try_from(_ulnumentriesout).unwrap_or(0);
        if out_len == 0 {
            return Ok(());
        }

        // SAFETY: pointer is non-null (checked above) and points to at least one output entry.
        let requested_type = unsafe { (*_ppropertyentriesout).Type };
        let initial_application = if *_querytype == WTS_QUERY_ALLOWED_INITIAL_APP {
            // SAFETY: The caller supplies three readable input entries.
            let input = unsafe { core::slice::from_raw_parts(_ppropertyentriesin, 3) };
            if input[2].Type != WTS_VALUE_TYPE_ULONG {
                return Err(windows_core::Error::new(
                    E_INVALIDARG,
                    "invalid initial-application reserved property",
                ));
            }
            Some((property_string(&input[0])?, property_string(&input[1])?))
        } else {
            None
        };

        let is_known_querytype = *_querytype == PROPERTY_TYPE_GET_FAST_RECONNECT
            || *_querytype == PROPERTY_TYPE_GET_FAST_RECONNECT_USER_SID
            || *_querytype == PROPERTY_TYPE_ENABLE_UNIVERSAL_APPS_FOR_CUSTOM_SHELL
            || *_querytype == PROPERTY_TYPE_CONNECTION_GUID
            || *_querytype == PROPERTY_TYPE_SUPPRESS_LOGON_UI
            || *_querytype == PROPERTY_TYPE_CAPTURE_PROTECTED_CONTENT
            || *_querytype == PROPERTY_TYPE_LICENSE_GUID
            || *_querytype == WTS_QUERY_AUDIOENUM_DLL;

        debug_log_line(&format!(
            "IWRdsProtocolConnection::QueryProperty called connection_id={} querytype={:?} in={} out={} out0_type_in={requested_type}",
            self.inner.connection_id(),
            _querytype,
            _ulnumentriesin,
            _ulnumentriesout,
        ));

        // SAFETY: TermService provides `_ulnumentriesout` writable entries.
        let out = unsafe { core::slice::from_raw_parts_mut(_ppropertyentriesout, out_len) };
        if let Some((application, parameters)) = initial_application {
            let (_, program) = remote_app_initial_program()?;
            let enabled = self
                .control_bridge
                .remote_app_settings(self.inner.connection_id())?
                .is_some_and(|settings| settings.enabled);
            let authenticated = self.cached_credentials.lock().is_some();
            let bootstrap = parameters.is_empty()
                && (application.eq_ignore_ascii_case("rdpinit.exe") || application.eq_ignore_ascii_case(&program));
            let published = if enabled && authenticated && !bootstrap {
                self.published_applications
                    .resolve(&application, &parameters)
                    .map_err(|error| {
                        debug_log_line(&format!(
                            "Published application authorization failed connection_id={} error={error}",
                            self.inner.connection_id()
                        ));
                        error
                    })?
            } else {
                None
            };
            let allowed = enabled && authenticated && (bootstrap || published.is_some());
            let (program, parameters) = match published.as_ref() {
                Some(published) => (published.program.as_str(), published.parameters.as_str()),
                None if allowed => (program.as_str(), ""),
                None => ("", ""),
            };
            write_property_string(&mut out[0], program)?;
            write_property_string(&mut out[1], parameters)?;
            for entry in &mut out[2..] {
                *entry = WTS_PROPERTY_VALUE {
                    Type: WTS_VALUE_TYPE_ULONG,
                    ..WTS_PROPERTY_VALUE::default()
                };
            }
            out[2].u.ulVal = u32::from(allowed);
            if let Some(alias) = out.get_mut(3) {
                alias.u.ulVal = u32::from(published.is_some());
            }
            debug_log_line(&format!(
                "Native initial-application decision connection_id={} allowed={allowed}",
                self.inner.connection_id()
            ));
            return Ok(());
        }

        for entry in out.iter_mut() {
            *entry = WTS_PROPERTY_VALUE::default();
        }

        // `wtsdefs.h` documents QueryProperty GUIDs and the expected output types.
        // TermService appears to treat invalid `Type` values as a hard failure.
        let first = &mut out[0];

        if *_querytype == PROPERTY_TYPE_GET_REMOTEAPP_FEATURE_FLAGS
            || *_querytype == PROPERTY_TYPE_GET_REMOTEAPP_HD_SUPPORT_LEVEL
        {
            first.Type = WTS_VALUE_TYPE_ULONG;
            first.u.ulVal = 0;
            return Ok(());
        }

        if *_querytype == PROPERTY_TYPE_RAIL_CLIENT_CAPABILITIES || *_querytype == PROPERTY_TYPE_WINDOW_CAPABILITYSET {
            let settings = self
                .control_bridge
                .remote_app_settings(self.inner.connection_id())?
                .filter(|settings| settings.enabled)
                .ok_or_else(|| windows_core::Error::new(E_NOTIMPL, "RemoteApp is not enabled"))?;
            if *_querytype == PROPERTY_TYPE_RAIL_CLIENT_CAPABILITIES {
                let support = settings
                    .client_rail_support_level
                    .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "RemoteApp capabilities are unavailable"))?;
                first.Type = WTS_VALUE_TYPE_ULONG;
                first.u.ulVal = support;
            } else {
                let capability = settings
                    .window_capability_set
                    .filter(|bytes| bytes.len() == 11)
                    .ok_or_else(|| {
                        windows_core::Error::new(E_UNEXPECTED, "Window List capabilities are unavailable")
                    })?;
                // SAFETY: CConnectionEx::GetConnectionProperty frees this buffer with LocalFree.
                let memory = unsafe { LocalAlloc(LMEM_FIXED, capability.len()) }?;
                // SAFETY: The allocation has exactly the source length.
                unsafe { core::ptr::copy_nonoverlapping(capability.as_ptr(), memory.0.cast(), capability.len()) };
                first.Type = WTS_VALUE_TYPE_BINARY;
                first.u.bVal.size = 11;
                first.u.bVal.pbVal = windows_core::PSTR(memory.0.cast());
            }
            return Ok(());
        }

        if *_querytype == PROPERTY_TYPE_GET_FAST_RECONNECT {
            let fast_reconnect_mode = provider_fast_reconnect_mode();
            first.Type = WTS_VALUE_TYPE_ULONG;
            first.u.ulVal = fast_reconnect_mode;
            debug_log_line(&format!(
                "IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_GET_FAST_RECONNECT -> {fast_reconnect_mode}",
            ));
            debug_log_line(&format!(
                "SESSION_PROOF_PROVIDER_FAST_RECONNECT connection_id={} mode={}",
                self.inner.connection_id(),
                fast_reconnect_mode,
            ));
            return Ok(());
        }

        if *_querytype == PROPERTY_TYPE_CONNECTION_GUID {
            let enabled = self
                .control_bridge
                .remote_app_settings(self.inner.connection_id())?
                .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "connection mode is unavailable"))?
                .enabled;
            if !enabled {
                return Err(windows_core::Error::new(
                    E_NOTIMPL,
                    "use the default desktop terminal type",
                ));
            }
            // SAFETY: CConnectionEx::GetConnectionGUIDFromProtocol frees this buffer with LocalFree, not CoTaskMemFree.
            let memory = unsafe { LocalAlloc(LMEM_FIXED, size_of::<GUID>()) }?;
            // SAFETY: The fixed allocation is aligned and large enough for one GUID.
            unsafe { memory.0.cast::<GUID>().write(TERMINAL_TYPE_RDP_REMOTEAPP) };
            first.Type = WTS_VALUE_TYPE_BINARY;
            first.u.bVal.size = 16;
            first.u.bVal.pbVal = windows_core::PSTR(memory.0.cast());
            return Ok(());
        }

        if *_querytype == PROPERTY_TYPE_SUPPRESS_LOGON_UI {
            debug_log_line("IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_SUPPRESS_LOGON_UI -> E_NOTIMPL");
            return Err(E_NOTIMPL.into());
        }

        if *_querytype == PROPERTY_TYPE_CAPTURE_PROTECTED_CONTENT {
            debug_log_line(
                "IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_CAPTURE_PROTECTED_CONTENT -> E_NOTIMPL",
            );
            return Err(E_NOTIMPL.into());
        }

        if *_querytype == PROPERTY_TYPE_LICENSE_GUID {
            debug_log_line("IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_LICENSE_GUID -> E_NOTIMPL");
            return Err(E_NOTIMPL.into());
        }

        if *_querytype == PROPERTY_TYPE_GET_FAST_RECONNECT_USER_SID {
            if provider_fast_reconnect_mode() == FAST_RECONNECT_DISABLED {
                let empty_w: [u16; 1] = [0];
                let bytes = size_of::<u16>();

                // SAFETY: CoTaskMemAlloc returns a valid pointer or null; TermService is responsible for freeing.
                let mem = unsafe { CoTaskMemAlloc(bytes) };
                if mem.is_null() {
                    return Err(windows_core::Error::new(E_OUTOFMEMORY, "CoTaskMemAlloc failed"));
                }

                // SAFETY: `mem` is at least `bytes` bytes.
                unsafe {
                    core::ptr::copy_nonoverlapping(empty_w.as_ptr(), mem.cast::<u16>(), empty_w.len());
                }

                first.Type = WTS_VALUE_TYPE_STRING;
                first.u.strVal.size = 1;
                first.u.strVal.pstrVal = PWSTR(mem.cast());

                debug_log_line(
                    "IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_GET_FAST_RECONNECT_USER_SID -> '' (fast reconnect disabled)",
                );
                return Ok(());
            }

            let _connection_id = self.inner.connection_id();
            let sid = self
                .cached_connection_credentials()
                .and_then(|(username, domain, _password)| {
                    let (winlogon_username, winlogon_domain) = normalize_winlogon_credentials(&username, &domain);
                    lookup_account_sid_string(&winlogon_username, &winlogon_domain).ok()
                })
                .unwrap_or_default();

            let sid_w: Vec<u16> = sid.encode_utf16().chain(core::iter::once(0)).collect();
            let bytes = sid_w.len().saturating_mul(size_of::<u16>());

            // SAFETY: CoTaskMemAlloc returns a valid pointer or null; TermService is responsible for freeing.
            let mem = unsafe { CoTaskMemAlloc(bytes) };
            if mem.is_null() {
                return Err(windows_core::Error::new(E_OUTOFMEMORY, "CoTaskMemAlloc failed"));
            }

            // SAFETY: `mem` is at least `bytes` bytes, and `sid_w` has the same length in u16s.
            unsafe {
                core::ptr::copy_nonoverlapping(sid_w.as_ptr(), mem.cast::<u16>(), sid_w.len());
            }

            first.Type = WTS_VALUE_TYPE_STRING;
            first.u.strVal.size = u32::try_from(sid_w.len()).unwrap_or(0);
            first.u.strVal.pstrVal = PWSTR(mem.cast());

            debug_log_line(&format!(
                "IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_GET_FAST_RECONNECT_USER_SID -> {sid}",
            ));
            return Ok(());
        }

        if *_querytype == PROPERTY_TYPE_ENABLE_UNIVERSAL_APPS_FOR_CUSTOM_SHELL {
            // `wtsdefs.h`: 0 = don't enable, 1 = enable.
            // `userinitext.dll!ShouldLaunchShellAppRuntime` starts `ShellAppRuntime.exe` when this is
            // nonzero, which claims `Progman` before `userinit.exe` starts `Explorer.EXE`.
            let connection_id = self.inner.connection_id();
            let has_credentials = self.cached_connection_credentials().is_some();
            let enable_universal_apps = 0;

            first.Type = WTS_VALUE_TYPE_ULONG;
            first.u.ulVal = enable_universal_apps;
            debug_log_line(&format!(
                "IWRdsProtocolConnection::QueryProperty PROPERTY_TYPE_ENABLE_UNIVERSAL_APPS_FOR_CUSTOM_SHELL -> {enable_universal_apps} creds_available={has_credentials}",
            ));
            debug_log_line(&format!(
                "SESSION_PROOF_PROVIDER_UNIVERSAL_APPS connection_id={connection_id} value={enable_universal_apps} creds_available={has_credentials}",
            ));
            return Ok(());
        }

        if *_querytype == WTS_QUERY_AUDIOENUM_DLL {
            // `wtsdefs.h`: used by audio enumeration / Media Foundation components.
            // We don't provide an audio-enum helper DLL; return an empty string.
            let empty_w: [u16; 1] = [0];
            let bytes = size_of::<u16>();

            // SAFETY: CoTaskMemAlloc returns a valid pointer or null; TermService is responsible for freeing.
            let mem = unsafe { CoTaskMemAlloc(bytes) };
            if mem.is_null() {
                return Err(windows_core::Error::new(E_OUTOFMEMORY, "CoTaskMemAlloc failed"));
            }

            // SAFETY: `mem` is at least `bytes` bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(empty_w.as_ptr(), mem.cast::<u16>(), empty_w.len());
            }

            first.Type = WTS_VALUE_TYPE_STRING;
            first.u.strVal.size = 1;
            first.u.strVal.pstrVal = PWSTR(mem.cast());

            debug_log_line("IWRdsProtocolConnection::QueryProperty WTS_QUERY_AUDIOENUM_DLL -> ''");
            return Ok(());
        }

        if !is_known_querytype {
            debug_log_line(&format!(
                "IWRdsProtocolConnection::QueryProperty unknown querytype={_querytype:?} requested_type={requested_type} -> E_NOTIMPL",
            ));
        }

        Err(windows_core::Error::new(
            E_NOTIMPL,
            "QueryProperty querytype is not implemented",
        ))
    }

    fn GetShadowConnection(&self) -> windows_core::Result<IWRdsProtocolShadowConnection> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "shadow connection is not implemented",
        ))
    }

    fn NotifyCommandProcessCreated(&self, sessionid: u32) -> windows_core::Result<()> {
        let connection_id = self.inner.connection_id();
        let has_userinit = session_has_process(sessionid, "userinit.exe");
        let has_explorer = session_has_process(sessionid, "explorer.exe");
        let has_logonui = session_has_process(sessionid, "LogonUI.exe");
        let has_winlogon = session_has_process(sessionid, "winlogon.exe");

        if self.wddm_idd_enabled.load(Ordering::SeqCst)
            && self.driver_handle_raw.load(Ordering::SeqCst) == 0
            && (has_userinit || has_explorer)
        {
            if provider_video_handle_probe_enabled() {
                let _ = self.probe_idd_driver_load(
                    sessionid,
                    "notify_command_process_created",
                    1,
                    Duration::from_millis(0),
                );
            } else {
                debug_log_line(&format!(
                    "SESSION_PROOF_PROVIDER_IDD_NOTIFY_DRIVER_LOAD_DEFERRED source=notify_command_process_created connection_id={connection_id} session_id={sessionid} reason=awaiting_on_driver_load userinit={has_userinit} explorer={has_explorer}",
                ));
            }
        } else if self.wddm_idd_enabled.load(Ordering::SeqCst) && self.driver_handle_raw.load(Ordering::SeqCst) == 0 {
            debug_log_line(&format!(
                "SESSION_PROOF_PROVIDER_IDD_NOTIFY_DRIVER_LOAD_DEFERRED source=notify_command_process_created connection_id={connection_id} session_id={sessionid} reason=pre_shell_state userinit={has_userinit} explorer={has_explorer} logonui={has_logonui} winlogon={has_winlogon}",
            ));
        }
        let (video_handle_open, video_handle_nonzero, video_source) = {
            let driver_handle_raw = self.driver_handle_raw.load(Ordering::SeqCst);
            let slot = self.video_handle.lock();
            let source_slot = self.video_handle_source.lock();
            let source = if driver_handle_raw != 0 {
                VideoHandleSource::DriverLoadCallback
            } else {
                *source_slot
            };
            let cached_handle_raw = slot.map(handle_to_raw_usize).unwrap_or_default();
            let effective_raw = if driver_handle_raw != 0 {
                driver_handle_raw
            } else {
                cached_handle_raw
            };
            (driver_handle_raw != 0 || slot.is_some(), effective_raw != 0, source)
        };

        debug_log_line(&format!(
            "IWRdsProtocolConnection::NotifyCommandProcessCreated called connection_id={} session_id={sessionid} userinit={} explorer={} logonui={} winlogon={}",
            connection_id, has_userinit, has_explorer, has_logonui, has_winlogon,
        ));
        debug_log_line(&format!(
            "SESSION_PROOF_PROVIDER_VIDEO_HANDLE_STATE connection_id={connection_id} session_id={sessionid} opened={video_handle_open} handle_nonzero={video_handle_nonzero} source={}",
            video_source.as_str(),
        ));
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

/// Whether an endpoint name can be submitted to the companion for authorization.
///
/// TermService attaches its RDPDR consumer only when RDPSND is served too.
/// The companion restricts dynamic opens to fixed endpoints and cameras announced on the same connection.
fn native_endpoint_enabled(endpoint_name: &str, is_static: bool) -> bool {
    if is_static {
        matches!(endpoint_name, "RDPDR" | "RDPSND" | "RAIL")
    } else {
        !endpoint_name.is_empty()
            && endpoint_name.len() <= 256
            && endpoint_name.is_ascii()
            && !endpoint_name.as_bytes().contains(&0)
            && !matches!(endpoint_name, "RDPDR" | "RDPSND" | "RAIL")
    }
}

fn io_error_to_windows_error(error: std::io::Error, context: &'static str) -> windows_core::Error {
    windows_core::Error::new(E_UNEXPECTED, format!("{context}: {error}"))
}

fn with_provider_identity<T: Send>(
    operation: impl FnOnce() -> windows_core::Result<T> + Send,
) -> windows_core::Result<T> {
    // Native RPC calls can impersonate the user; a fresh worker opens provider-only pipes without changing that token.
    thread::scope(|scope| {
        thread::Builder::new()
            .name("ironrdp-provider-identity".to_owned())
            .spawn_scoped(scope, operation)
            .map_err(|error| io_error_to_windows_error(error, "spawn provider worker"))?
            .join()
            .map_err(|_| windows_core::Error::new(E_UNEXPECTED, "provider worker panicked"))?
    })
}

#[expect(clippy::as_conversions)]
fn handle_to_raw_usize(handle: HANDLE) -> usize {
    handle.0 as usize
}

/// Native virtual channel requests, as TermService issues them through `CreateVirtualChannel`.
#[cfg(test)]
mod native_channel_tests {
    use core::ffi::c_void;
    use core::sync::atomic::{AtomicU32, Ordering};
    use core::time::Duration;
    use std::fs::File;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _};
    use std::sync::{Arc, mpsc};
    use std::thread;

    use ironrdp_wtsprotocol_ipc::{
        DEFAULT_MAX_FRAME_SIZE, ProviderCommand, ServiceEvent, read_json_message, write_json_message,
    };
    use parking_lot::Mutex;
    use windows::Win32::Foundation::{E_NOTIMPL, E_UNEXPECTED, ERROR_PIPE_CONNECTED, HANDLE};
    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows::Win32::System::Com::{IAgileObject, IAgileObject_Impl};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeHandleStateW, NAMED_PIPE_MODE, PIPE_READMODE_MESSAGE,
        PIPE_TYPE_BYTE, PIPE_TYPE_MESSAGE, PIPE_WAIT,
    };
    use windows::Win32::System::RemoteDesktop::{
        IWRdsProtocolConnection, IWRdsProtocolConnectionCallback, IWRdsProtocolConnectionCallback_Impl,
        IWRdsProtocolListener, IWRdsProtocolListenerCallback, IWRdsProtocolListenerCallback_Impl,
        WRDS_CONNECTION_SETTINGS, WTS_PROPERTY_VALUE, WTS_SMALL_RECT,
    };
    use windows_core::{AgileReference, HRESULT, HSTRING, implement, s};

    use super::{
        ComInitGuard, ComProtocolConnection, ComProtocolListener, ProviderControlBridge, native_endpoint_enabled,
    };

    const SESSION_ID: u32 = 5;
    const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

    enum Reply {
        Event(ServiceEvent),
        HangUp,
    }

    /// A control pipe server that answers one provider command the way the companion service does.
    struct FakeCompanion {
        bridge: ProviderControlBridge,
        commands: mpsc::Receiver<ProviderCommand>,
    }

    impl FakeCompanion {
        fn replying(reply: Reply) -> Self {
            let name = unique_name("IronRdpProviderTestControl");
            let mut pipe = server_pipe(&name, false);
            let (commands_tx, commands) = mpsc::channel();

            thread::spawn(move || {
                // SAFETY: The handle belongs to the live pipe server instance owned by `pipe`.
                if let Err(error) = unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) } {
                    assert_eq!(
                        error.code(),
                        HRESULT::from_win32(ERROR_PIPE_CONNECTED.0),
                        "wait for the provider to connect"
                    );
                }
                let command = read_json_message::<ProviderCommand>(&mut pipe, DEFAULT_MAX_FRAME_SIZE)
                    .expect("read the provider command");
                commands_tx.send(command).expect("report the provider command");
                if let Reply::Event(event) = reply {
                    write_json_message(&mut pipe, &event).expect("write the reply");
                    // Returns once the provider has read the reply.
                    pipe.sync_all().expect("flush the reply");
                }
            });

            Self {
                bridge: ProviderControlBridge {
                    pipe_name: Some(name),
                    optional_connection: false,
                },
                commands,
            }
        }

        fn command(&self) -> ProviderCommand {
            self.commands
                .recv_timeout(COMMAND_TIMEOUT)
                .expect("the provider sends a command")
        }
    }

    fn unique_name(prefix: &str) -> String {
        static NEXT: AtomicU32 = AtomicU32::new(0);

        format!(
            "{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn server_pipe(name: &str, message_mode: bool) -> File {
        let mode = if message_mode {
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE
        } else {
            PIPE_TYPE_BYTE
        };

        // SAFETY: The name is a valid NUL-terminated string for the duration of the call.
        let handle = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(format!(r"\\.\pipe\{name}")),
                PIPE_ACCESS_DUPLEX,
                mode | PIPE_WAIT,
                1,
                64 * 1024,
                64 * 1024,
                0,
                None,
            )
        };
        assert!(
            !handle.is_invalid(),
            "create the pipe: {}",
            windows_core::Error::from_thread()
        );

        // SAFETY: The handle was just created and nothing else owns it.
        unsafe { File::from_raw_handle(handle.0) }
    }

    fn connection(bridge: ProviderControlBridge, session_id: Option<u32>) -> IWRdsProtocolConnection {
        let inner = Arc::new(crate::ProtocolConnection::new(
            1,
            Arc::new(crate::SessionRegistry::default()),
        ));
        if let Some(session_id) = session_id {
            inner.accept_connection().expect("accept the connection");
            inner.notify_session_id(session_id).expect("assign the session");
        }

        ComProtocolConnection::new(inner, Arc::new(Mutex::new(None)), bridge).into()
    }

    fn open_command(endpoint_name: &str, static_channel: bool) -> ProviderCommand {
        ProviderCommand::OpenVirtualChannel {
            connection_id: 1,
            session_id: SESSION_ID,
            endpoint_name: endpoint_name.to_owned(),
            static_channel,
        }
    }

    fn no_companion() -> ProviderControlBridge {
        ProviderControlBridge {
            pipe_name: None,
            optional_connection: true,
        }
    }

    #[implement(IWRdsProtocolListenerCallback, IAgileObject)]
    struct NativeListenerCallback {
        connected: mpsc::Sender<AgileReference<IWRdsProtocolConnection>>,
        ready: mpsc::Sender<AgileReference<IWRdsProtocolConnection>>,
        broken: mpsc::Sender<()>,
        dropped: mpsc::Sender<()>,
        close_on_connected: bool,
    }

    impl IAgileObject_Impl for NativeListenerCallback_Impl {}

    impl IWRdsProtocolListenerCallback_Impl for NativeListenerCallback_Impl {
        fn OnConnected(
            &self,
            pconnection: windows_core::Ref<'_, IWRdsProtocolConnection>,
            _settings: *const WRDS_CONNECTION_SETTINGS,
        ) -> windows_core::Result<IWRdsProtocolConnectionCallback> {
            let connection = pconnection.ok()?;
            if self.close_on_connected {
                // SAFETY: The provider passed a live connection to the native listener callback.
                unsafe { connection.Close()? };
            }
            let connection = AgileReference::new(connection)?;
            self.connected
                .send(connection.clone())
                .map_err(|error| windows_core::Error::new(E_UNEXPECTED, error.to_string()))?;
            Ok(NativeConnectionCallback {
                connection,
                ready: self.ready.clone(),
                broken: self.broken.clone(),
                dropped: self.dropped.clone(),
                close_on_drop: None,
            }
            .into())
        }
    }

    #[implement(IWRdsProtocolConnectionCallback, IAgileObject)]
    struct NativeConnectionCallback {
        connection: AgileReference<IWRdsProtocolConnection>,
        ready: mpsc::Sender<AgileReference<IWRdsProtocolConnection>>,
        broken: mpsc::Sender<()>,
        dropped: mpsc::Sender<()>,
        close_on_drop: Option<mpsc::Sender<Result<(), HRESULT>>>,
    }

    impl Drop for NativeConnectionCallback {
        fn drop(&mut self) {
            if let Some(closed) = &self.close_on_drop {
                let result = self.connection.resolve().and_then(|connection| {
                    // SAFETY: The callback owns the live connection returned by the provider.
                    unsafe { connection.Close() }
                });
                let _ = closed.send(result.map_err(|error| error.code()));
            }
            let _ = self.dropped.send(());
        }
    }

    impl IAgileObject_Impl for NativeConnectionCallback_Impl {}

    impl IWRdsProtocolConnectionCallback_Impl for NativeConnectionCallback_Impl {
        fn OnReady(&self) -> windows_core::Result<()> {
            self.ready
                .send(self.connection.clone())
                .map_err(|error| windows_core::Error::new(E_UNEXPECTED, error.to_string()))
        }

        fn BrokenConnection(&self, _reason: u32, _source: u32) -> windows_core::Result<()> {
            let connection = self.connection.resolve()?;
            // SAFETY: The callback owns the live connection returned by the provider.
            unsafe { connection.Close()? };
            self.broken
                .send(())
                .map_err(|error| windows_core::Error::new(E_UNEXPECTED, error.to_string()))
        }

        fn StopScreenUpdates(&self) -> windows_core::Result<()> {
            Err(windows_core::Error::new(E_NOTIMPL, "unexpected stop screen updates"))
        }

        fn RedrawWindow(&self, _rect: *const WTS_SMALL_RECT) -> windows_core::Result<()> {
            Err(windows_core::Error::new(E_NOTIMPL, "unexpected redraw window"))
        }

        fn GetConnectionId(&self) -> windows_core::Result<u32> {
            Ok(1)
        }
    }

    struct NativeListenerProbe {
        listener: IWRdsProtocolListener,
        companion: thread::JoinHandle<()>,
        connected: mpsc::Receiver<AgileReference<IWRdsProtocolConnection>>,
        ready: mpsc::Receiver<AgileReference<IWRdsProtocolConnection>>,
        broken: mpsc::Receiver<()>,
        dropped: mpsc::Receiver<()>,
        notifications: mpsc::Sender<ServiceEvent>,
    }

    impl NativeListenerProbe {
        fn start(close_on_connected: bool) -> Self {
            let pipe_name = unique_name("IronRdpListenerCloseTest");
            let mut pipe = server_pipe(&pipe_name, false);
            let mut listener = ComProtocolListener::new(
                crate::ProtocolListener::new(Arc::new(crate::SessionRegistry::default())),
                "IronRdpListenerCloseTest".to_owned(),
            );
            listener.control_bridge = ProviderControlBridge {
                pipe_name: Some(pipe_name.clone()),
                optional_connection: false,
            };
            let listener: IWRdsProtocolListener = listener.into();
            let (notifications, incoming) = mpsc::channel();
            let companion = thread::spawn(move || {
                let mut offered = false;
                loop {
                    // SAFETY: `pipe` owns the live named pipe server handle.
                    if let Err(error) = unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) } {
                        assert_eq!(error.code(), HRESULT::from_win32(ERROR_PIPE_CONNECTED.0));
                    }
                    let command = read_json_message::<ProviderCommand>(&mut pipe, DEFAULT_MAX_FRAME_SIZE)
                        .expect("read listener command");
                    let stopping = matches!(command, ProviderCommand::StopListen { .. });
                    let reply = match command {
                        ProviderCommand::StartListen { .. }
                        | ProviderCommand::StopListen { .. }
                        | ProviderCommand::CloseConnection { .. } => ServiceEvent::Ack,
                        ProviderCommand::WaitForIncoming { listener_name, .. } if !offered => {
                            offered = true;
                            ServiceEvent::IncomingConnection {
                                listener_name,
                                connection_id: 1,
                                peer_addr: None,
                            }
                        }
                        ProviderCommand::WaitForIncoming { .. } => match incoming.try_recv() {
                            Ok(event) => event,
                            Err(mpsc::TryRecvError::Empty) => ServiceEvent::NoIncoming,
                            Err(mpsc::TryRecvError::Disconnected) => panic!("notification sender dropped before stop"),
                        },
                        ProviderCommand::AcceptConnection { connection_id } => {
                            ServiceEvent::ConnectionReady { connection_id }
                        }
                        ProviderCommand::GetConnectionCredentials { connection_id } => {
                            ServiceEvent::ConnectionCredentials {
                                connection_id,
                                username: "test-user".to_owned(),
                                domain: String::new(),
                                password: "test-password".to_owned(),
                            }
                        }
                        other => panic!("unexpected listener command: {other:?}"),
                    };
                    write_json_message(&mut pipe, &reply).expect("reply to listener command");
                    pipe.sync_all().expect("wait for the listener to read the reply");
                    if stopping {
                        break;
                    }
                    drop(pipe);
                    pipe = server_pipe(&pipe_name, false);
                }
            });
            let (connected_tx, connected) = mpsc::channel();
            let (ready_tx, ready) = mpsc::channel();
            let (broken_tx, broken) = mpsc::channel();
            let (dropped_tx, dropped) = mpsc::channel();
            let callback: IWRdsProtocolListenerCallback = NativeListenerCallback {
                connected: connected_tx,
                ready: ready_tx,
                broken: broken_tx,
                dropped: dropped_tx,
                close_on_connected,
            }
            .into();

            // SAFETY: The listener and callback are live COM interfaces in an initialized apartment.
            unsafe { listener.StartListen(&callback) }.expect("start the listener");
            Self {
                listener,
                companion,
                connected,
                ready,
                broken,
                dropped,
                notifications,
            }
        }

        fn stop(self) {
            // SAFETY: The probe owns the live listener in an initialized apartment.
            unsafe { self.listener.StopListen() }.expect("stop the listener");
            self.companion.join().expect("companion completes");
        }
    }

    // oracle: spec IWRdsProtocolListenerCallback::OnConnected, pCallback: the protocol must release it
    // when the connection is closed, independently of a later transport failure notification.
    #[test]
    fn server_close_releases_callback_while_listener_keeps_running() {
        let _com = ComInitGuard::new();
        let probe = NativeListenerProbe::start(false);
        let connection = probe
            .ready
            .recv_timeout(COMMAND_TIMEOUT)
            .expect("the listener dispatches an incoming connection")
            .resolve()
            .expect("resolve the connection in this apartment");
        // SAFETY: The connection was received from the listener and is still owned by this test.
        unsafe { connection.Close() }.expect("close the connection without a broken notification");
        let released = probe.dropped.recv_timeout(COMMAND_TIMEOUT);

        probe.stop();
        assert_eq!(released, Ok(()), "Close must release the callback before StopListen");
    }

    // oracle: spec IWRdsProtocolListenerCallback::OnConnected, pCallback: closing a connection releases
    // its callback even when the native listener closes it before returning that callback.
    #[test]
    fn close_during_on_connected_does_not_retain_returned_callback() {
        let _com = ComInitGuard::new();
        let probe = NativeListenerProbe::start(true);
        let _connection = probe
            .connected
            .recv_timeout(COMMAND_TIMEOUT)
            .expect("the native listener closes the incoming connection");
        let released = probe.dropped.recv_timeout(COMMAND_TIMEOUT);

        probe.stop();
        assert_eq!(
            released,
            Ok(()),
            "registration must not retain an already closed callback"
        );
    }

    // oracle: mutant releasing the last callback reference under its mutex deadlocks reentrant Close.
    #[test]
    fn callback_destruction_can_reenter_close() {
        let _com = ComInitGuard::new();
        let callback_slot = Arc::new(Mutex::new(None));
        let inner = Arc::new(crate::ProtocolConnection::new(
            1,
            Arc::new(crate::SessionRegistry::default()),
        ));
        let connection: IWRdsProtocolConnection =
            ComProtocolConnection::new(inner, Arc::clone(&callback_slot), no_companion()).into();
        let connection = AgileReference::new(&connection).expect("share the connection across apartments");
        let (ready, _ready) = mpsc::channel();
        let (broken, _broken) = mpsc::channel();
        let (dropped_tx, dropped) = mpsc::channel();
        let (reentered_tx, reentered) = mpsc::channel();
        let callback: IWRdsProtocolConnectionCallback = NativeConnectionCallback {
            connection: connection.clone(),
            ready,
            broken,
            dropped: dropped_tx,
            close_on_drop: Some(reentered_tx),
        }
        .into();
        *callback_slot.lock() = Some(AgileReference::new(&callback).expect("retain the native callback"));
        drop(callback);
        let (completed_tx, completed) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _com = ComInitGuard::new();
            let connection = connection.resolve().expect("resolve the connection");
            // SAFETY: The worker owns a live COM connection in an initialized apartment.
            let result = unsafe { connection.Close() };
            completed_tx
                .send(result.map_err(|error| error.code()))
                .expect("report Close completion");
        });
        let result = completed.recv_timeout(COMMAND_TIMEOUT);
        if result.is_ok() {
            worker.join().expect("Close worker completes");
        }
        assert_eq!(
            result,
            Ok(Ok(())),
            "Close must not deadlock during callback destruction"
        );
        // A nested Close can reject an already closed connection, but it must return.
        let _reentrant_result = reentered
            .recv_timeout(COMMAND_TIMEOUT)
            .expect("callback destruction returns from reentrant Close");
        assert_eq!(dropped.recv_timeout(COMMAND_TIMEOUT), Ok(()));
    }

    // oracle: spec IWRdsProtocolConnectionCallback::BrokenConnection notifies RDS of transport loss;
    // IWRdsProtocolListenerCallback::OnConnected requires releasing the callback after RDS closes the connection.
    #[test]
    fn transport_loss_notifies_native_callback_and_allows_reentrant_close() {
        let _com = ComInitGuard::new();
        let probe = NativeListenerProbe::start(false);
        let _connection = probe.ready.recv_timeout(COMMAND_TIMEOUT).expect("connection is ready");
        probe
            .notifications
            .send(ServiceEvent::ConnectionBroken {
                connection_id: 1,
                reason: "test transport closed".to_owned(),
            })
            .expect("report transport loss");
        let notified = probe.broken.recv_timeout(COMMAND_TIMEOUT);
        let released = probe.dropped.recv_timeout(COMMAND_TIMEOUT);

        probe.stop();
        assert_eq!(notified, Ok(()), "active connections must retain their native callback");
        assert_eq!(
            released,
            Ok(()),
            "native Close must release the callback without deadlocking"
        );
    }

    fn remote_app_provider(enabled: bool, authenticated: bool) -> (ComProtocolConnection, FakeCompanion) {
        let companion = FakeCompanion::replying(Reply::Event(ServiceEvent::RemoteAppSettings {
            connection_id: 1,
            settings: Some(super::RemoteAppSettings {
                enabled,
                client_rail_support_level: Some(1),
                window_capability_set: Some(vec![24, 0, 11, 0, 2, 0, 0, 0, 3, 12, 0]),
                ..super::RemoteAppSettings::default()
            }),
        }));
        let inner = Arc::new(crate::ProtocolConnection::new(
            1,
            Arc::new(crate::SessionRegistry::default()),
        ));
        let connection = ComProtocolConnection::new(inner, Arc::new(Mutex::new(None)), companion.bridge.clone());
        if authenticated {
            *connection.cached_credentials.lock() =
                Some(("test-user".to_owned(), String::new(), "test-password".to_owned()));
        }
        (connection, companion)
    }

    fn remote_app_connection(enabled: bool, authenticated: bool) -> (IWRdsProtocolConnection, FakeCompanion) {
        let (connection, companion) = remote_app_provider(enabled, authenticated);
        (connection.into(), companion)
    }

    // oracle: reference termsrv.dll SHA256 16A660526148D0C26FB64D2EA0141AA7A3C6CB5E5CF5413CBE5822085755E560,
    // CConnectionEx::GetConnectionGUIDFromProtocol reads TERMINAL_TYPE_RDP_REMOTEAPP and frees it with LocalFree.
    #[test]
    fn remote_app_connection_type_identifies_the_native_remote_app_terminal() {
        let (connection, _companion) = remote_app_connection(true, false);
        let mut output = WTS_PROPERTY_VALUE::default();
        // SAFETY: The native query has no inputs and one writable output entry.
        unsafe {
            connection.QueryProperty(
                super::PROPERTY_TYPE_CONNECTION_GUID,
                &[],
                core::slice::from_mut(&mut output),
            )
        }
        .unwrap();
        assert_eq!(output.Type, 3);
        // SAFETY: Type 3 identifies the binary member, which the caller owns.
        unsafe {
            assert_eq!(output.u.bVal.size, 16);
            assert!(!output.u.bVal.pbVal.is_null());
            let actual = core::slice::from_raw_parts(output.u.bVal.pbVal.0, 16).to_vec();
            let freed = super::LocalFree(Some(super::HLOCAL(output.u.bVal.pbVal.0.cast())));
            assert!(freed.is_invalid());
            assert_eq!(
                actual,
                [
                    0xce, 0xc3, 0xdc, 0xed, 0x7e, 0x6e, 0x4b, 0x4f, 0x84, 0x39, 0x3d, 0x9a, 0xd4, 0xc9, 0x44, 0x0f
                ]
            );
        }
    }

    // oracle: mutant classifying a desktop connection as RemoteApp replaces its native shell semantics.
    #[test]
    fn desktop_connection_type_keeps_the_native_default() {
        let (connection, _companion) = remote_app_connection(false, true);
        let mut output = WTS_PROPERTY_VALUE::default();
        // SAFETY: The native query has no inputs and one writable output entry.
        let error = unsafe {
            connection.QueryProperty(
                super::PROPERTY_TYPE_CONNECTION_GUID,
                &[],
                core::slice::from_mut(&mut output),
            )
        }
        .unwrap_err();
        assert_eq!(error.code(), E_NOTIMPL);
    }

    // oracle: mutant missing Client Info must not classify an unknown connection as RemoteApp.
    #[test]
    fn unknown_connection_type_is_not_reported_as_remote_app() {
        let companion = FakeCompanion::replying(Reply::Event(ServiceEvent::RemoteAppSettings {
            connection_id: 1,
            settings: None,
        }));
        let connection = connection(companion.bridge, None);
        let mut output = WTS_PROPERTY_VALUE::default();
        // SAFETY: The native query has no inputs and one writable output entry.
        let error = unsafe {
            connection.QueryProperty(
                super::PROPERTY_TYPE_CONNECTION_GUID,
                &[],
                core::slice::from_mut(&mut output),
            )
        }
        .unwrap_err();
        assert_eq!(error.code(), E_UNEXPECTED);
    }

    // oracle: reference userinitext.dll ShouldLaunchShellAppRuntime and rdpserverbase.dll
    // CRDPWDUMXStack::GetConnectionProperty: enabling PROPERTY_TYPE_ENABLE_UNIVERSAL_APPS_FOR_CUSTOM_SHELL
    // launches ShellAppRuntime.exe, which cloaks Progman and prevents Explorer.EXE from starting the desktop shell.
    #[test]
    fn custom_shell_universal_apps_property_remains_disabled_after_authentication() {
        let (connection, _companion) = remote_app_connection(false, true);
        let mut output = WTS_PROPERTY_VALUE::default();
        // SAFETY: The native query has no inputs and one writable output entry.
        unsafe {
            connection.QueryProperty(
                super::PROPERTY_TYPE_ENABLE_UNIVERSAL_APPS_FOR_CUSTOM_SHELL,
                &[],
                core::slice::from_mut(&mut output),
            )
        }
        .unwrap();
        assert_eq!(output.Type, super::WTS_VALUE_TYPE_ULONG);
        // SAFETY: The ULONG tag identifies the active union member.
        assert_eq!(unsafe { output.u.ulVal }, 0);
    }

    fn string_buffer_property(buffer: &mut [u16]) -> WTS_PROPERTY_VALUE {
        let mut property = WTS_PROPERTY_VALUE {
            Type: super::WTS_VALUE_TYPE_STRING,
            ..Default::default()
        };
        property.u.strVal.size = u32::try_from(size_of_val(buffer)).unwrap();
        property.u.strVal.pstrVal = super::PWSTR(buffer.as_mut_ptr());
        property
    }

    // oracle: reference termsrv.dll SHA256 16A660526148D0C26FB64D2EA0141AA7A3C6CB5E5CF5413CBE5822085755E560,
    // ApplyTSAllowPolicyToInitialProgram supplies four outputs and byte-sized caller-owned string buffers.
    #[test]
    fn native_initial_application_query_uses_caller_owned_byte_buffers() {
        let (connection, _companion) = remote_app_connection(true, true);
        let mut application = [0xD800; 257];
        let name: Vec<_> = "rdpinit.exe".encode_utf16().chain(Some(0)).collect();
        application[..name.len()].copy_from_slice(&name);
        let mut input = [
            string_buffer_property(&mut application),
            WTS_PROPERTY_VALUE {
                Type: super::WTS_VALUE_TYPE_STRING,
                ..Default::default()
            },
            WTS_PROPERTY_VALUE {
                Type: super::WTS_VALUE_TYPE_ULONG,
                ..Default::default()
            },
        ];
        input[2].u.ulVal = 10;
        let mut command_buffer = [0xA55A; 262];
        let mut parameter_buffer = [0xA55A; 262];
        let mut output = [
            string_buffer_property(&mut command_buffer[..261]),
            string_buffer_property(&mut parameter_buffer[..261]),
            WTS_PROPERTY_VALUE {
                Type: super::WTS_VALUE_TYPE_ULONG,
                ..Default::default()
            },
            WTS_PROPERTY_VALUE {
                Type: super::WTS_VALUE_TYPE_ULONG,
                ..Default::default()
            },
        ];
        // SAFETY: All entries and their advertised byte buffers remain valid throughout the call.
        unsafe { connection.QueryProperty(super::WTS_QUERY_ALLOWED_INITIAL_APP, &input, &mut output) }.unwrap();
        // SAFETY: Successful string and ULONG output entries retain their declared types.
        unsafe {
            assert_eq!(output[0].u.strVal.pstrVal.0, command_buffer.as_mut_ptr());
            assert_eq!(output[1].u.strVal.pstrVal.0, parameter_buffer.as_mut_ptr());
            assert_ne!(output[2].u.ulVal, 0);
        }
        let end = command_buffer.iter().position(|v| *v == 0).unwrap();
        let program = String::from_utf16(&command_buffer[..end]).unwrap();
        assert!(std::path::Path::new(&program).is_absolute());
        assert!(program.to_ascii_lowercase().ends_with(r"\system32\rdpinit.exe"));
        assert_eq!(parameter_buffer[0], 0);
        assert_eq!(command_buffer[261], 0xA55A);
        assert_eq!(parameter_buffer[261], 0xA55A);
    }

    fn allowed_initial_application(application: &str, parameters: &str, enabled: bool, authenticated: bool) -> bool {
        let (connection, _companion) = remote_app_connection(enabled, authenticated);
        let decision = application_decision(&connection, application, parameters).unwrap();
        assert!(decision.parameters.is_empty());
        if decision.allowed {
            assert!(
                decision
                    .program
                    .to_ascii_lowercase()
                    .ends_with(r"\system32\rdpinit.exe"),
                "{}",
                decision.program
            );
        }
        decision.allowed
    }

    struct ApplicationDecision {
        allowed: bool,
        expanded_alias: bool,
        program: String,
        parameters: String,
    }

    fn application_decision(
        connection: &IWRdsProtocolConnection,
        application: &str,
        parameters: &str,
    ) -> windows_core::Result<ApplicationDecision> {
        let mut application: Vec<_> = application.encode_utf16().chain(Some(0)).collect();
        let mut parameters: Vec<_> = parameters.encode_utf16().chain(Some(0)).collect();
        let input = [
            string_buffer_property(&mut application),
            string_buffer_property(&mut parameters),
            WTS_PROPERTY_VALUE {
                Type: super::WTS_VALUE_TYPE_ULONG,
                ..Default::default()
            },
        ];
        let mut command_buffer = [0; 261];
        let mut parameter_buffer = [0; 261];
        let mut output = [
            string_buffer_property(&mut command_buffer),
            string_buffer_property(&mut parameter_buffer),
            WTS_PROPERTY_VALUE::default(),
            WTS_PROPERTY_VALUE::default(),
        ];
        // SAFETY: Three input entries and four output entries remain live for the native call.
        unsafe { connection.QueryProperty(super::WTS_QUERY_ALLOWED_INITIAL_APP, &input, &mut output) }?;
        assert_eq!(output[2].Type, super::WTS_VALUE_TYPE_ULONG);
        assert_eq!(output[3].Type, super::WTS_VALUE_TYPE_ULONG);
        let command_end = command_buffer.iter().position(|v| *v == 0).unwrap();
        let parameter_end = parameter_buffer.iter().position(|v| *v == 0).unwrap();
        // SAFETY: The ULONG tags identify the active union members.
        unsafe {
            Ok(ApplicationDecision {
                allowed: output[2].u.ulVal != 0,
                expanded_alias: output[3].u.ulVal != 0,
                program: String::from_utf16(&command_buffer[..command_end]).unwrap(),
                parameters: String::from_utf16(&parameter_buffer[..parameter_end]).unwrap(),
            })
        }
    }

    struct PublishedAppFixture {
        path: String,
    }

    impl PublishedAppFixture {
        fn new(setting: u32, required: &str, descriptor: Option<&str>) -> Self {
            use windows::Win32::System::Registry::{
                HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_CREATE_KEY_DISPOSITION, REG_CREATED_NEW_KEY, REG_DWORD,
                REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW,
            };
            use windows_core::{PCWSTR, w};

            let path = format!(r"Software\{}", unique_name("IronRdpPublishedApps"));
            let mut root = HKEY::default();
            let mut disposition = REG_CREATE_KEY_DISPOSITION::default();
            // SAFETY: The temporary root is process-unique; all handle outputs are writable.
            unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(&path),
                    None,
                    PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_WRITE,
                    None,
                    &mut root,
                    Some(&mut disposition),
                )
            }
            .ok()
            .unwrap();
            assert_eq!(
                disposition, REG_CREATED_NEW_KEY,
                "never overwrite an existing registry fixture"
            );
            let fixture = Self { path };
            let mut app = HKEY::default();
            // SAFETY: `root` is live, and `app` is a writable handle output.
            let opened = unsafe {
                RegCreateKeyExW(
                    root,
                    w!("App"),
                    None,
                    PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_WRITE,
                    None,
                    &mut app,
                    None,
                )
            };
            // SAFETY: The child handle does not borrow the parent handle.
            unsafe { RegCloseKey(root) }.ok().unwrap();
            opened.ok().unwrap();
            let configured = (|| {
                for (name, value) in [
                    ("Path", Some(r"C:\Program Files\IronRdp Fixture\app.exe")),
                    ("RequiredCommandLine", Some(required)),
                    ("SecurityDescriptor", descriptor),
                ] {
                    if let Some(value) = value {
                        let bytes: Vec<_> = value.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
                        // SAFETY: The value name and bytes remain live for the registry write.
                        unsafe { RegSetValueExW(app, &HSTRING::from(name), None, REG_SZ, Some(&bytes)) }.ok()?;
                    }
                }
                // SAFETY: The byte array represents one DWORD.
                unsafe {
                    RegSetValueExW(
                        app,
                        w!("CommandLineSetting"),
                        None,
                        REG_DWORD,
                        Some(&setting.to_le_bytes()),
                    )
                }
                .ok()
            })();
            // SAFETY: The fixture owns the child handle and no longer needs it.
            unsafe { RegCloseKey(app) }.ok().unwrap();
            configured.unwrap();
            fixture
        }

        fn connection(&self, enabled: bool, authenticated: bool) -> (IWRdsProtocolConnection, FakeCompanion) {
            let (mut connection, companion) = remote_app_provider(enabled, authenticated);
            connection.published_applications = super::PublishedApplications::for_test(
                windows::Win32::System::Registry::HKEY_CURRENT_USER,
                self.path.clone(),
            );
            (connection.into(), companion)
        }

        fn set_value(&self, name: &str, value_type: windows::Win32::System::Registry::REG_VALUE_TYPE, bytes: &[u8]) {
            use windows::Win32::System::Registry::{
                HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegOpenKeyExW, RegSetValueExW,
            };

            let mut app = HKEY::default();
            // SAFETY: The fixture owns this application key, and the handle output is writable.
            unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(format!(r"{}\App", self.path)),
                    None,
                    KEY_SET_VALUE,
                    &mut app,
                )
            }
            .ok()
            .unwrap();
            // SAFETY: The name and bytes remain live for the write.
            let result = unsafe { RegSetValueExW(app, &HSTRING::from(name), None, value_type, Some(bytes)) };
            // SAFETY: This helper owns the handle and no longer needs it.
            unsafe { RegCloseKey(app) }.ok().unwrap();
            result.ok().unwrap();
        }
    }

    impl Drop for PublishedAppFixture {
        fn drop(&mut self) {
            use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW};

            // SAFETY: This exact subtree was newly created and is owned only by this fixture.
            unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(&self.path)) }
                .ok()
                .expect("delete the owned published-app fixture");
        }
    }

    // oracle: spec Win32_TSPublishedApplication Path and CommandLineSetting 0 identify the executable and forbid arguments.
    // oracle: reference rdpcorets.dll VerifyAppAllowed returns the expanded alias and quotes paths containing spaces.
    #[test]
    fn published_app_alias_resolves_without_client_arguments() {
        let fixture = PublishedAppFixture::new(0, "", None);
        let (connection, _companion) = fixture.connection(true, true);
        let decision = application_decision(&connection, "||App", "/untrusted").unwrap();
        assert!(decision.allowed);
        assert!(decision.expanded_alias);
        assert_eq!(decision.program, r#""C:\Program Files\IronRdp Fixture\app.exe""#);
        assert!(decision.parameters.is_empty());
    }

    // oracle: spec Win32_TSPublishedApplication CommandLineSetting 1 allows the client's command-line arguments.
    #[test]
    fn published_app_allows_configured_client_arguments() {
        let fixture = PublishedAppFixture::new(1, "", None);
        let (connection, _companion) = fixture.connection(true, true);
        let decision = application_decision(&connection, "||App", r#"/open "a b.txt""#).unwrap();
        assert!(decision.allowed);
        assert_eq!(decision.parameters, r#"/open "a b.txt""#);
    }

    // oracle: spec Win32_TSPublishedApplication CommandLineSetting 2 always uses RequiredCommandLine.
    #[test]
    fn published_app_uses_required_arguments_instead_of_client_arguments() {
        let fixture = PublishedAppFixture::new(2, "/configured", None);
        let (connection, _companion) = fixture.connection(true, true);
        let decision = application_decision(&connection, "||App", "/untrusted").unwrap();
        assert!(decision.allowed);
        assert_eq!(decision.parameters, "/configured");
    }

    // oracle: mutant publishing one alias must not authorize an absent alias.
    #[test]
    fn unpublished_app_alias_is_denied() {
        let fixture = PublishedAppFixture::new(0, "", None);
        let (connection, _companion) = fixture.connection(true, true);
        let decision = application_decision(&connection, "||Absent", "").unwrap();
        assert!(!decision.allowed);
        assert!(!decision.expanded_alias);
        assert!(decision.program.is_empty());
    }

    // oracle: mutant a registry separator must not let an alias escape the published-app registry root.
    #[test]
    fn published_app_alias_cannot_select_another_registry_path() {
        let fixture = PublishedAppFixture::new(0, "", None);
        let (connection, _companion) = fixture.connection(true, true);
        assert!(!application_decision(&connection, r"||..\App", "").unwrap().allowed);
    }

    // oracle: mutant publishing an alias must not authorize launch before authenticated credentials exist.
    #[test]
    fn published_app_requires_authentication() {
        let fixture = PublishedAppFixture::new(0, "", None);
        let (connection, _companion) = fixture.connection(true, false);
        assert!(!application_decision(&connection, "||App", "").unwrap().allowed);
    }

    // oracle: mutant publishing an alias must not change the shell of a desktop connection.
    #[test]
    fn published_app_requires_remote_app_mode() {
        let fixture = PublishedAppFixture::new(0, "", None);
        let (connection, _companion) = fixture.connection(false, true);
        assert!(!application_decision(&connection, "||App", "").unwrap().allowed);
    }

    // oracle: spec Win32_TSPublishedApplication CommandLineSetting permits only 0, 1 or 2.
    #[test]
    fn published_app_rejects_an_unknown_argument_policy() {
        let fixture = PublishedAppFixture::new(3, "", None);
        let (connection, _companion) = fixture.connection(true, true);
        assert_eq!(
            application_decision(&connection, "||App", "").err().unwrap().code(),
            super::E_INVALIDARG
        );
    }

    // oracle: reference Windows Server 2025 RemoteDesktop Utility.psm1 Get-SecurityDescriptorFromUserGroup
    // emits O:WDG:WDD:ARP(A;CIOI;CCLCSWLORCGR;;;SID) for each permitted group.
    #[test]
    fn published_app_user_group_is_checked_as_the_rpc_caller() {
        thread::spawn(|| {
            let fixture = PublishedAppFixture::new(0, "", Some("O:WDG:WDD:ARP(A;CIOI;CCLCSWLORCGR;;;WD)"));
            let (connection, _companion) = fixture.connection(true, true);
            // SAFETY: Impersonation is confined to this dedicated test thread.
            unsafe { windows::Win32::Security::ImpersonateSelf(super::SecurityImpersonation) }.unwrap();
            let result = application_decision(&connection, "||App", "");
            // SAFETY: This test restores its thread before inspecting the result.
            unsafe { super::RevertToSelf() }.unwrap();
            assert!(result.unwrap().allowed);
        })
        .join()
        .unwrap();
    }

    // oracle: spec Win32_TSPublishedApplication SecurityDescriptor controls access to the application.
    // The null SID belongs to no caller, so this publication must not authorize the test user.
    #[test]
    fn published_app_denies_a_caller_outside_its_user_groups() {
        thread::spawn(|| {
            let fixture = PublishedAppFixture::new(0, "", Some("O:WDG:WDD:ARP(A;CIOI;CCLCSWLORCGR;;;S-1-0-0)"));
            let (connection, _companion) = fixture.connection(true, true);
            // SAFETY: Impersonation is confined to this dedicated test thread.
            unsafe { windows::Win32::Security::ImpersonateSelf(super::SecurityImpersonation) }.unwrap();
            let result = application_decision(&connection, "||App", "");
            // SAFETY: This test restores its thread before inspecting the result.
            unsafe { super::RevertToSelf() }.unwrap();
            assert!(!result.unwrap().allowed);
        })
        .join()
        .unwrap();
    }

    // oracle: mutant substituting the privileged process token bypasses published-app user restrictions.
    #[test]
    fn published_app_user_restrictions_require_an_rpc_caller_token() {
        thread::spawn(|| {
            let fixture = PublishedAppFixture::new(0, "", Some("O:WDG:WDD:ARP(A;CIOI;CCLCSWLORCGR;;;WD)"));
            let (connection, _companion) = fixture.connection(true, true);
            assert_eq!(
                application_decision(&connection, "||App", "").err().unwrap().code(),
                HRESULT::from_win32(super::ERROR_NO_TOKEN.0)
            );
        })
        .join()
        .unwrap();
    }

    // oracle: mutant treating a mistyped SecurityDescriptor as absent silently removes user restrictions.
    #[test]
    fn published_app_rejects_a_mistyped_security_descriptor() {
        let fixture = PublishedAppFixture::new(0, "", None);
        fixture.set_value(
            "SecurityDescriptor",
            windows::Win32::System::Registry::REG_DWORD,
            &0u32.to_le_bytes(),
        );
        let (connection, _companion) = fixture.connection(true, true);
        assert!(application_decision(&connection, "||App", "").is_err());
    }

    // oracle: mutant accepting an empty REG_EXPAND_SZ descriptor silently removes user restrictions.
    #[test]
    fn published_app_rejects_an_expandable_empty_security_descriptor() {
        let fixture = PublishedAppFixture::new(0, "", None);
        fixture.set_value(
            "SecurityDescriptor",
            windows::Win32::System::Registry::REG_EXPAND_SZ,
            &[0, 0],
        );
        let (connection, _companion) = fixture.connection(true, true);
        assert_eq!(
            application_decision(&connection, "||App", "").err().unwrap().code(),
            HRESULT::from_win32(windows::Win32::Foundation::ERROR_UNSUPPORTED_TYPE.0)
        );
    }

    // oracle: mutant expanding a REG_EXPAND_SZ policy accepts a different type from Windows publication REG_SZ.
    #[test]
    fn published_app_rejects_an_expandable_nonempty_security_descriptor() {
        let fixture = PublishedAppFixture::new(0, "", None);
        let bytes: Vec<_> = "O:WDG:WDD:ARP(A;CIOI;CCLCSWLORCGR;;;WD)"
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        fixture.set_value(
            "SecurityDescriptor",
            windows::Win32::System::Registry::REG_EXPAND_SZ,
            &bytes,
        );
        let (connection, _companion) = fixture.connection(true, true);
        assert_eq!(
            application_decision(&connection, "||App", "").err().unwrap().code(),
            HRESULT::from_win32(windows::Win32::Foundation::ERROR_UNSUPPORTED_TYPE.0)
        );
    }

    // oracle: mutant truncating a malformed descriptor at its first NUL treats it as an unrestricted publication.
    #[test]
    fn published_app_rejects_an_embedded_security_descriptor_terminator() {
        let fixture = PublishedAppFixture::new(0, "", None);
        fixture.set_value(
            "SecurityDescriptor",
            windows::Win32::System::Registry::REG_SZ,
            &[0, 0, b'x', 0, 0, 0],
        );
        let (connection, _companion) = fixture.connection(true, true);
        assert!(application_decision(&connection, "||App", "").is_err());
    }

    // oracle: mutant an invalid SDDL value must not be interpreted as an unrestricted publication.
    #[test]
    fn published_app_rejects_a_malformed_security_descriptor() {
        thread::spawn(|| {
            let fixture = PublishedAppFixture::new(0, "", Some("not an SDDL descriptor"));
            let (connection, _companion) = fixture.connection(true, true);
            // SAFETY: Impersonation is confined to this dedicated test thread.
            unsafe { windows::Win32::Security::ImpersonateSelf(super::SecurityImpersonation) }.unwrap();
            let result = application_decision(&connection, "||App", "");
            // SAFETY: This test restores its thread before inspecting the result.
            unsafe { super::RevertToSelf() }.unwrap();
            assert!(result.is_err());
        })
        .join()
        .unwrap();
    }

    // oracle: mutant mapping GENERIC_WRITE to the launch read permission grants access without a read ACE.
    #[test]
    fn published_app_write_access_does_not_grant_read_access() {
        thread::spawn(|| {
            let fixture = PublishedAppFixture::new(0, "", Some("O:WDG:WDD:ARP(A;CIOI;GW;;;WD)"));
            let (connection, _companion) = fixture.connection(true, true);
            // SAFETY: Impersonation is confined to this dedicated test thread.
            unsafe { windows::Win32::Security::ImpersonateSelf(super::SecurityImpersonation) }.unwrap();
            let result = application_decision(&connection, "||App", "");
            // SAFETY: This test restores its thread before inspecting the result.
            unsafe { super::RevertToSelf() }.unwrap();
            assert!(!result.unwrap().allowed);
        })
        .join()
        .unwrap();
    }

    // oracle: mutant authorizing an arbitrary initial executable bypasses the native RemoteApp shell.
    #[test]
    fn remote_app_bootstrap_rejects_other_executables() {
        assert!(!allowed_initial_application("cmd.exe", "", true, true));
    }

    // oracle: mutant passing bootstrap parameters through permits an uncontrolled native launch.
    #[test]
    fn remote_app_bootstrap_rejects_parameters() {
        assert!(!allowed_initial_application("rdpinit.exe", "/untrusted", true, true));
    }

    // oracle: mutant starting RemoteApp in a desktop connection changes the authenticated user's shell.
    #[test]
    fn remote_app_bootstrap_requires_a_remote_app_connection() {
        assert!(!allowed_initial_application("rdpinit.exe", "", false, true));
    }

    // oracle: mutant authorizing a bootstrap before credentials exist bypasses authenticated session startup.
    #[test]
    fn remote_app_bootstrap_requires_authenticated_credentials() {
        assert!(!allowed_initial_application("rdpinit.exe", "", true, false));
    }

    // oracle: spec WTS_QUERY_ALLOWED_INITIAL_APP: a nonzero third ULONG output authorizes the returned program.
    #[test]
    fn remote_app_bootstrap_allows_only_the_trusted_program_without_parameters() {
        assert!(allowed_initial_application("rdpinit.exe", "", true, true));
    }

    // oracle: spec WTS_CLIENT_DATA InitialProgram: fInheritInitialProgram must be TRUE for this field to be used.
    #[test]
    fn remote_app_client_data_selects_the_native_bootstrap() {
        let (connection, companion) = remote_app_connection(true, true);
        let mut data = super::WTS_CLIENT_DATA::default();
        // SAFETY: The structure is a valid writable output buffer.
        unsafe { connection.GetClientData(&mut data) }.unwrap();
        assert!(data.fInheritInitialProgram.as_bool());
        let path =
            String::from_utf16(&data.InitialProgram[..data.InitialProgram.iter().position(|v| *v == 0).unwrap()])
                .unwrap();
        assert!(std::path::Path::new(&path).is_absolute());
        assert!(path.to_ascii_lowercase().ends_with(r"\system32\rdpinit.exe"), "{path}");
        assert_eq!(
            companion.command(),
            ProviderCommand::GetRemoteAppSettings { connection_id: 1 }
        );
    }

    // oracle: mutant setting InitialProgram for a desktop connection replaces Explorer.
    #[test]
    fn desktop_client_data_keeps_the_default_shell() {
        let (connection, _companion) = remote_app_connection(false, true);
        let mut data = super::WTS_CLIENT_DATA::default();
        // SAFETY: The structure is a valid writable output buffer.
        unsafe { connection.GetClientData(&mut data) }.unwrap();
        assert!(!data.fInheritInitialProgram.as_bool());
        assert_eq!(data.InitialProgram[0], 0);
    }

    // oracle: spec MS-RDPBCGR §2.2.1.3.2 and §2.2.1.11.1.1.1: WTS_CLIENT_DATA carries client performance flags and keyboard layout.
    #[test]
    fn client_data_forwards_performance_flags_and_keyboard_layout() {
        let companion = FakeCompanion::replying(Reply::Event(ServiceEvent::RemoteAppSettings {
            connection_id: 1,
            settings: Some(super::RemoteAppSettings {
                enabled: false,
                client_rail_support_level: None,
                window_capability_set: None,
                client_info_flags: 0x0008_0123,
                performance_flags: 0x0000_0086,
                keyboard_layout: 0x0001_1009,
                keyboard_type: 4,
                keyboard_subtype: 2,
                keyboard_function_key: 12,
                ime_file_name: "ime.dll".to_owned(),
                desktop_width: 1920,
                desktop_height: 1080,
            }),
        }));
        let inner = Arc::new(crate::ProtocolConnection::new(
            1,
            Arc::new(crate::SessionRegistry::default()),
        ));
        let connection = ComProtocolConnection::new(inner, Arc::new(Mutex::new(None)), companion.bridge);
        *connection.cached_credentials.lock() =
            Some(("test-user".to_owned(), String::new(), "test-password".to_owned()));
        let connection: IWRdsProtocolConnection = connection.into();
        let mut data = super::WTS_CLIENT_DATA::default();
        // SAFETY: The structure is a valid writable output buffer.
        unsafe { connection.GetClientData(&mut data) }.unwrap();
        assert_eq!(data.PerformanceFlags, 0x0000_0086);
        assert_eq!(data.KeyboardLayout, 0x0001_1009);
        assert_eq!(data.ActiveInputLocale, 0x0001_1009);
        assert_eq!(data.KeyboardType, 4);
        assert_eq!(data.KeyboardSubType, 2);
        assert_eq!(data.KeyboardFunctionKey, 12);
        assert_eq!(data.HRes, 1920);
        assert_eq!(data.VRes, 1080);
        assert_eq!(data.ColorDepth, 5);
    }

    // oracle: reference rdpshell.exe CRdpShell::GetIconCacheDimensions requires the complete 11-byte Window capability.
    // TermService's CConnectionEx::GetConnectionProperty frees the returned binary property with LocalFree.
    #[test]
    fn remote_app_window_property_includes_the_capability_header() {
        let (connection, _companion) = remote_app_connection(true, true);
        let mut output = WTS_PROPERTY_VALUE::default();
        // SAFETY: No inputs are required and output points to one writable entry.
        unsafe {
            connection.QueryProperty(
                super::PROPERTY_TYPE_WINDOW_CAPABILITYSET,
                &[],
                core::slice::from_mut(&mut output),
            )
        }
        .unwrap();
        assert_eq!(output.Type, 3);
        // SAFETY: Type 3 identifies the binary union member returned by QueryProperty.
        unsafe {
            assert_eq!(output.u.bVal.size, 11);
            assert_eq!(
                core::slice::from_raw_parts(output.u.bVal.pbVal.0, 11),
                &[24, 0, 11, 0, 2, 0, 0, 0, 3, 12, 0]
            );
            assert!(super::LocalFree(Some(super::HLOCAL(output.u.bVal.pbVal.0.cast()))).is_invalid());
        }
    }

    // oracle: reference native RAIL startup trace, 2026-10-27T02:35:58Z: QueryProperty runs impersonating the user.
    #[test]
    fn remote_app_property_reads_do_not_use_the_callers_impersonation_token() {
        thread::spawn(|| {
            let (connection, companion) = remote_app_connection(true, true);
            let mut output = WTS_PROPERTY_VALUE::default();
            let result = with_restricted_thread_token(|| {
                // SAFETY: The native query has no inputs and one writable output entry.
                unsafe {
                    connection.QueryProperty(
                        super::PROPERTY_TYPE_RAIL_CLIENT_CAPABILITIES,
                        &[],
                        core::slice::from_mut(&mut output),
                    )
                }
            });
            if result.is_err() {
                companion.bridge.remote_app_settings(1).unwrap();
            }
            result.unwrap();
            assert_eq!(output.Type, super::WTS_VALUE_TYPE_ULONG);
            // SAFETY: The ULONG tag identifies the active union member.
            assert_eq!(unsafe { output.u.ulVal }, 1);
        })
        .join()
        .unwrap();
    }

    // oracle: reference native rdpshell startup trace, 2026-10-27T02:57:58Z:
    // CreateVirtualChannel("RAIL") runs impersonating the user, but both pipes require the provider identity.
    #[test]
    fn native_channel_open_preserves_the_callers_impersonation_token() {
        thread::spawn(|| {
            let channel_name = unique_name("IronRdpNativeDvc");
            let _channel = server_pipe(&channel_name, true);
            let companion = FakeCompanion::replying(Reply::Event(ServiceEvent::VirtualChannelReady {
                pipe_name: format!(r"\\.\pipe\{channel_name}"),
            }));
            let connection = connection(companion.bridge.clone(), Some(SESSION_ID));
            let handle = with_restricted_thread_token(|| {
                // SAFETY: The COM object and NUL-terminated endpoint are valid for this call.
                unsafe { connection.CreateVirtualChannel(s!("RAIL"), true, 0) }
            })
            .expect("an authorized native channel opens while the caller impersonates");
            // SAFETY: The call transfers a new pipe handle to its caller.
            let _client = unsafe { File::from_raw_handle(core::ptr::with_exposed_provenance_mut::<c_void>(handle)) };
            assert_eq!(companion.command(), open_command("RAIL", true));
        })
        .join()
        .unwrap();
    }

    fn with_restricted_thread_token<T>(operation: impl FnOnce() -> T) -> T {
        use std::os::windows::io::OwnedHandle;

        use windows::Win32::Security::{
            CreateRestrictedToken, CreateWellKnownSid, DISABLE_MAX_PRIVILEGE, ImpersonateSelf, IsTokenRestricted, PSID,
            SID_AND_ATTRIBUTES, WinRestrictedCodeSid,
        };

        let mut sid = [0u32; 17];
        let mut sid_size = u32::try_from(size_of_val(&sid)).unwrap();
        let sid_ptr = PSID(sid.as_mut_ptr().cast());
        let mut token = HANDLE::default();
        let mut restricted = HANDLE::default();
        // SAFETY: The SID and handle outputs are writable, and impersonation affects only this dedicated thread.
        unsafe {
            CreateWellKnownSid(WinRestrictedCodeSid, None, Some(sid_ptr), &mut sid_size).unwrap();
            ImpersonateSelf(super::SecurityImpersonation).unwrap();
            super::OpenThreadToken(
                super::GetCurrentThread(),
                super::TOKEN_DUPLICATE | super::TOKEN_QUERY | super::TOKEN_IMPERSONATE,
                true,
                &mut token,
            )
            .unwrap();
            let token = OwnedHandle::from_raw_handle(token.0);
            CreateRestrictedToken(
                HANDLE(token.as_raw_handle()),
                DISABLE_MAX_PRIVILEGE,
                None,
                None,
                Some(&[SID_AND_ATTRIBUTES {
                    Sid: sid_ptr,
                    Attributes: 0,
                }]),
                &mut restricted,
            )
            .unwrap();
            let restricted = OwnedHandle::from_raw_handle(restricted.0);
            super::SetThreadToken(None, Some(HANDLE(restricted.as_raw_handle()))).unwrap();
            let result = operation();
            let mut after = HANDLE::default();
            let token_result = super::OpenThreadToken(super::GetCurrentThread(), super::TOKEN_QUERY, true, &mut after);
            super::RevertToSelf().unwrap();
            token_result.unwrap();
            let after = OwnedHandle::from_raw_handle(after.0);
            assert!(IsTokenRestricted(HANDLE(after.as_raw_handle())).is_ok());
            result
        }
    }

    // oracle: mutant accepting another connection's capabilities crosses the per-connection authorization boundary.
    #[test]
    fn remote_app_settings_reject_a_mismatched_connection_id() {
        let companion = FakeCompanion::replying(Reply::Event(ServiceEvent::RemoteAppSettings {
            connection_id: 2,
            settings: None,
        }));
        assert!(companion.bridge.remote_app_settings(1).is_err());
    }

    #[test]
    fn only_supported_endpoint_names_reach_companion_authorization() {
        // oracle: spec MS-RDPEFS, MS-RDPEA and MS-RDPEWA section 2.1 name the fixed endpoints; MS-RDPECAM section 2.1 permits device channel names of up to 256 ANSI characters.
        let served = [
            ("Devolutions::Now::Agent", false),
            ("WebAuthN_Channel", false),
            ("RDCamera_Device_Enumerator", false),
            ("Camera_Device_0", false),
            ("RDPDR", true),
            ("RDPSND", true),
            ("RAIL", true),
            ("railenc", false),
        ];
        // oracle: mutant forwarding malformed names or unsupported static channels bypasses the endpoint boundary.
        let not_served = [
            ("Devolutions::Now::Agent", true),
            ("RDPDR", false),
            ("RDPSND", false),
            ("RAIL", false),
            ("cliprdr", true),
            ("Camera_Device_0", true),
            ("", false),
            ("Camera\0Device", false),
            ("\u{e9}", false),
        ];

        for (endpoint_name, is_static) in served {
            assert!(
                native_endpoint_enabled(endpoint_name, is_static),
                "{endpoint_name} static={is_static}"
            );
        }
        for (endpoint_name, is_static) in not_served {
            assert!(
                !native_endpoint_enabled(endpoint_name, is_static),
                "{endpoint_name} static={is_static}"
            );
        }
        assert!(native_endpoint_enabled(&"c".repeat(256), false));
        assert!(!native_endpoint_enabled(&"c".repeat(257), false));
    }

    #[test]
    fn a_static_channel_the_companion_refuses_is_not_implemented() {
        let companion = FakeCompanion::replying(Reply::Event(ServiceEvent::Error {
            message: "native static channel was not offered by the client".to_owned(),
        }));
        let connection = connection(companion.bridge.clone(), Some(SESSION_ID));

        // SAFETY: The COM object and NUL-terminated endpoint are valid for this call.
        let error = unsafe { connection.CreateVirtualChannel(s!("RDPDR"), true, 0) }.unwrap_err();

        // oracle: mutant forwarding a static request as a dynamic one makes the companion open the wrong kind of channel.
        assert_eq!(companion.command(), open_command("RDPDR", true));
        // oracle: regression TermService tolerates E_NOTIMPL for RDPDR and RDPSND at logon; no other code was tried there.
        assert_eq!(error.code(), E_NOTIMPL);
    }

    #[test]
    fn a_dynamic_channel_the_companion_refuses_is_unexpected() {
        let companion = FakeCompanion::replying(Reply::Event(ServiceEvent::Error {
            message: "no NOW channel was offered".to_owned(),
        }));
        let connection = connection(companion.bridge.clone(), Some(SESSION_ID));

        // SAFETY: The COM object and NUL-terminated endpoint are valid for this call.
        let error = unsafe { connection.CreateVirtualChannel(s!("Devolutions::Now::Agent"), false, 0) }.unwrap_err();

        // oracle: mutant marking every request static makes the companion look for a static channel that a dynamic endpoint never has.
        assert_eq!(companion.command(), open_command("Devolutions::Now::Agent", false));
        // oracle: regression a refused NOW channel fails with E_UNEXPECTED, as it did before static endpoints were served.
        assert_eq!(error.code(), E_UNEXPECTED);
    }

    #[test]
    fn a_static_channel_without_a_companion_is_not_implemented() {
        let connection = connection(no_companion(), Some(SESSION_ID));

        // SAFETY: The COM object and NUL-terminated endpoint are valid for this call.
        let error = unsafe { connection.CreateVirtualChannel(s!("RDPDR"), true, 0) }.unwrap_err();

        // oracle: regression TermService tolerates E_NOTIMPL for RDPDR and RDPSND at logon; no other code was tried there.
        assert_eq!(error.code(), E_NOTIMPL);
    }

    #[test]
    fn a_static_channel_whose_companion_hangs_up_is_not_implemented() {
        let companion = FakeCompanion::replying(Reply::HangUp);
        let connection = connection(companion.bridge, Some(SESSION_ID));

        // SAFETY: The COM object and NUL-terminated endpoint are valid for this call.
        let error = unsafe { connection.CreateVirtualChannel(s!("RDPDR"), true, 0) }.unwrap_err();

        // oracle: regression TermService tolerates E_NOTIMPL for RDPDR and RDPSND at logon; no other code was tried there.
        assert_eq!(error.code(), E_NOTIMPL);
    }

    #[test]
    fn a_static_channel_requested_before_the_session_is_assigned_is_not_implemented() {
        let connection = connection(no_companion(), None);

        // SAFETY: The COM object and NUL-terminated endpoint are valid for this call.
        let error = unsafe { connection.CreateVirtualChannel(s!("RDPDR"), true, 0) }.unwrap_err();

        // oracle: regression TermService tolerates E_NOTIMPL for RDPDR and RDPSND at logon; no other code was tried there.
        assert_eq!(error.code(), E_NOTIMPL);
    }

    #[test]
    fn an_offered_static_channel_is_returned_as_a_message_mode_pipe() {
        let channel_name = unique_name("IronRdpNativeDvc");
        let _channel = server_pipe(&channel_name, true);
        let companion = FakeCompanion::replying(Reply::Event(ServiceEvent::VirtualChannelReady {
            pipe_name: format!(r"\\.\pipe\{channel_name}"),
        }));
        let connection = connection(companion.bridge.clone(), Some(SESSION_ID));

        // SAFETY: The COM object and NUL-terminated endpoint are valid for this call.
        let handle =
            unsafe { connection.CreateVirtualChannel(s!("RDPSND"), true, 0) }.expect("an offered channel opens");
        // SAFETY: The call returned a new pipe handle that nothing else owns.
        let client = unsafe { File::from_raw_handle(core::ptr::with_exposed_provenance_mut::<c_void>(handle)) };

        let mut state = NAMED_PIPE_MODE(0);
        // SAFETY: The handle is a live pipe client end and `state` is a valid output location.
        unsafe { GetNamedPipeHandleStateW(HANDLE(client.as_raw_handle()), Some(&mut state), None, None, None, None) }
            .ok()
            .expect("query the pipe state");

        // oracle: spec WTSVirtualChannelRead remarks; a message-mode read keeps each consumer message intact.
        assert_eq!(state & PIPE_READMODE_MESSAGE, PIPE_READMODE_MESSAGE);
        assert_eq!(companion.command(), open_command("RDPSND", true));
    }

    // oracle: spec MS-RDPECAM sections 2.1 and 3.3.5.3 use the device name announced by the client.
    #[test]
    fn a_camera_device_authorized_by_the_companion_opens_in_the_assigned_session() {
        let channel_name = unique_name("IronRdpNativeDvc");
        let _channel = server_pipe(&channel_name, true);
        let companion = FakeCompanion::replying(Reply::Event(ServiceEvent::VirtualChannelReady {
            pipe_name: format!(r"\\.\pipe\{channel_name}"),
        }));
        let connection = connection(companion.bridge.clone(), Some(SESSION_ID));

        // SAFETY: The COM object and NUL-terminated endpoint are valid for this call.
        let handle = unsafe { connection.CreateVirtualChannel(s!("Camera_Device_0"), false, 0) }
            .expect("the authorized camera device opens");
        // SAFETY: The call returned a new pipe handle that nothing else owns.
        let _client = unsafe { File::from_raw_handle(core::ptr::with_exposed_provenance_mut::<c_void>(handle)) };
        assert_eq!(companion.command(), open_command("Camera_Device_0", false));
    }

    // oracle: mutant treating a valid dynamic name as authorization opens a channel the companion refused.
    #[test]
    fn an_unannounced_dynamic_endpoint_is_not_opened() {
        let companion = FakeCompanion::replying(Reply::Event(ServiceEvent::Error {
            message: "native camera channel was not announced".to_owned(),
        }));
        let connection = connection(companion.bridge.clone(), Some(SESSION_ID));

        // SAFETY: The COM object and NUL-terminated endpoint are valid for this call.
        let error = unsafe { connection.CreateVirtualChannel(s!("AUDIO_PLAYBACK_DVC"), false, 0) }.unwrap_err();
        assert_eq!(error.code(), E_UNEXPECTED);
        assert_eq!(companion.command(), open_command("AUDIO_PLAYBACK_DVC", false));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{
        VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE, VirtualChannelBridgeEndpoint, VirtualChannelBridgeTx,
        VirtualChannelRouteKind, bridge_pipe_path, drain_length_prefixed_pipe_frames, sanitize_pipe_segment,
    };

    #[test]
    fn native_channel_without_transport_is_not_implemented() {
        use std::sync::Arc;

        use parking_lot::Mutex;
        use windows::Win32::Foundation::E_NOTIMPL;
        use windows::Win32::System::RemoteDesktop::IWRdsProtocolConnection;
        use windows_core::s;

        let inner = Arc::new(crate::ProtocolConnection::new(
            1,
            Arc::new(crate::SessionRegistry::default()),
        ));
        let connection: IWRdsProtocolConnection = super::ComProtocolConnection::new(
            inner,
            Arc::new(Mutex::new(None)),
            super::ProviderControlBridge {
                pipe_name: None,
                optional_connection: true,
            },
        )
        .into();

        // oracle: mutant a missing provider transport must not fall through to WTS client RPCs.
        // SAFETY: The COM object and NUL-terminated endpoint are valid for this call.
        let error = unsafe { connection.CreateVirtualChannel(s!("cliprdr"), true, 0) }.unwrap_err();
        assert_eq!(error.code(), E_NOTIMPL);
    }

    #[test]
    fn sanitize_pipe_segment_normalizes_name() {
        assert_eq!(sanitize_pipe_segment("ClipRdr"), "cliprdr");
        assert_eq!(
            sanitize_pipe_segment("Microsoft::Windows::RDS::Graphics"),
            "microsoft__windows__rds__graphics"
        );
        assert_eq!(sanitize_pipe_segment(""), "channel");
    }

    #[test]
    fn bridge_pipe_path_uses_svc_and_dvc_suffixes() {
        let static_endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name: "cliprdr".to_owned(),
            static_channel: true,
            route_kind: VirtualChannelRouteKind::IronRdpStatic,
        };

        let dynamic_endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name: "Microsoft::Windows::RDS::Graphics".to_owned(),
            static_channel: false,
            route_kind: VirtualChannelRouteKind::IronRdpDynamicEndpoint,
        };

        assert_eq!(
            bridge_pipe_path("IronRdpVcBridge", &static_endpoint),
            r"\\.\pipe\IronRdpVcBridge.svc.cliprdr"
        );
        assert_eq!(
            bridge_pipe_path(r"\\.\pipe\Bridge", &dynamic_endpoint),
            r"\\.\pipe\Bridge.dvc.microsoft__windows__rds__graphics"
        );
    }

    #[test]
    fn drain_length_prefixed_frames_forwards_payloads() {
        let endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name: "ECHO".to_owned(),
            static_channel: false,
            route_kind: VirtualChannelRouteKind::IronRdpDynamicEndpoint,
        };
        let (outbound_tx, outbound_rx) = mpsc::sync_channel(4);
        let bridge_tx = VirtualChannelBridgeTx {
            endpoint: endpoint.clone(),
            outbound_tx,
        };

        let mut framed = Vec::new();
        framed.extend_from_slice(&(3u32).to_le_bytes());
        framed.extend_from_slice(b"abc");
        framed.extend_from_slice(&(2u32).to_le_bytes());
        framed.extend_from_slice(b"de");

        drain_length_prefixed_pipe_frames(&endpoint, &bridge_tx, &mut framed).expect("framed payload should parse");

        assert!(framed.is_empty());
        assert_eq!(outbound_rx.try_recv().expect("first payload"), b"abc");
        assert_eq!(outbound_rx.try_recv().expect("second payload"), b"de");
    }

    #[test]
    fn drain_length_prefixed_frames_rejects_oversized_frame() {
        let endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name: "ECHO".to_owned(),
            static_channel: false,
            route_kind: VirtualChannelRouteKind::IronRdpDynamicEndpoint,
        };
        let (outbound_tx, _outbound_rx) = mpsc::sync_channel(1);
        let bridge_tx = VirtualChannelBridgeTx {
            endpoint: endpoint.clone(),
            outbound_tx,
        };

        let mut framed = Vec::new();
        let oversized_len =
            u32::try_from(VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE).expect("max frame size should fit in u32") + 1;
        framed.extend_from_slice(&oversized_len.to_le_bytes());

        let error = drain_length_prefixed_pipe_frames(&endpoint, &bridge_tx, &mut framed)
            .expect_err("oversized frame should fail");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}
