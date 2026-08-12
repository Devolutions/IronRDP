//! Windows WebAuthn API backend for MS-RDPEWA.

use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use ironrdp_rdpewa::{
    Attachment, Attestation, DeviceInfo, E_ABORT, E_FAIL, E_INVALIDARG, RdpewaClientHandler, RdpewaHandlerError,
    RdpewaResponse, RdpewaResponseSender, RdpewaResult, S_OK, UserVerification, WebAuthnDispatch,
    WebAuthnOperationRequest, WebAuthnOperationResponse, WebAuthnResponsePayload, WebAuthnSubcommand,
};
use tracing::{debug, info, warn};
use windows::Win32::Foundation::HWND;
use windows::Win32::Networking::WindowsWebServices::{
    WEBAUTHN_ASSERTION, WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_ANY,
    WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_DIRECT, WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_INDIRECT,
    WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE, WEBAUTHN_AUTHENTICATOR_ATTACHMENT_ANY,
    WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM, WEBAUTHN_AUTHENTICATOR_ATTACHMENT_PLATFORM,
    WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS, WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_CURRENT_VERSION,
    WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS, WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_CURRENT_VERSION,
    WEBAUTHN_CLIENT_DATA, WEBAUTHN_CLIENT_DATA_CURRENT_VERSION, WEBAUTHN_COSE_CREDENTIAL_PARAMETER,
    WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION, WEBAUTHN_COSE_CREDENTIAL_PARAMETERS, WEBAUTHN_CREDENTIAL_EX,
    WEBAUTHN_CREDENTIAL_EX_CURRENT_VERSION, WEBAUTHN_CREDENTIAL_LIST, WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
    WEBAUTHN_HASH_ALGORITHM_SHA_256, WEBAUTHN_RP_ENTITY_INFORMATION, WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION,
    WEBAUTHN_USER_ENTITY_INFORMATION, WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION,
    WEBAUTHN_USER_VERIFICATION_REQUIREMENT_ANY, WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
    WEBAUTHN_USER_VERIFICATION_REQUIREMENT_PREFERRED, WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED,
    WebAuthNAuthenticatorGetAssertion, WebAuthNAuthenticatorMakeCredential, WebAuthNCancelCurrentOperation,
    WebAuthNFreeAssertion, WebAuthNFreeCredentialAttestation, WebAuthNGetApiVersionNumber, WebAuthNGetCancellationId,
    WebAuthNIsUserVerifyingPlatformAuthenticatorAvailable,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
use windows::core::{GUID, HRESULT, PCWSTR};

use crate::ctap::{
    encode_get_assertion_response, encode_make_credential_response, pack_ctap_response, parse_get_assertion,
    parse_make_credential,
};

/// Process-wide cancel slot so CancelCurOp on a recreated `WebAuthN_Channel` can abort an in-flight
/// ceremony started by an earlier channel instance.
fn shared_cancel_guid() -> Arc<Mutex<Option<GUID>>> {
    static SLOT: OnceLock<Arc<Mutex<Option<GUID>>>> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(Mutex::new(None))).clone()
}

/// Windows Hello / security-key backend for [`ironrdp_rdpewa::RdpewaClient`].
pub struct WindowsRdpewaBackend {
    /// Parent window handle stored as integer so the backend is `Send`.
    parent_hwnd: isize,
    cancel_guid: Arc<Mutex<Option<GUID>>>,
}

impl WindowsRdpewaBackend {
    /// Create a backend that parents WebAuthn UI to `parent_hwnd`.
    ///
    /// Pass the ActiveX control HWND (or another top-level window handle).
    #[must_use]
    pub fn new(parent_hwnd: isize) -> Self {
        Self {
            parent_hwnd,
            cancel_guid: shared_cancel_guid(),
        }
    }

    fn platform_device_info(transports: u32, resident_key: Option<bool>) -> DeviceInfo {
        DeviceInfo {
            max_msg_size: 2048,
            max_serialized_large_blob_array: 1024,
            provider_type: String::from("Platform"),
            provider_name: String::from("IronRDPWebAuthnProvider"),
            device_path: String::new(),
            manufacturer: String::from("Microsoft"),
            product: String::from("Windows WebAuthn"),
            aa_guid: [0; 16],
            uv_status: WEBAUTHN_USER_VERIFICATION_REQUIREMENT_ANY,
            uv_retries: 0,
            transports,
            resident_key,
        }
    }
}

impl Drop for WindowsRdpewaBackend {
    fn drop(&mut self) {
        // Best-effort: dismiss an open WebAuthn prompt when the session tears down.
        let _ = cancel_guid_slot(&self.cancel_guid, None);
    }
}

impl RdpewaClientHandler for WindowsRdpewaBackend {
    fn api_version(&mut self) -> RdpewaResult<u32> {
        let version = unsafe { WebAuthNGetApiVersionNumber() };
        debug!(version, "WebAuthNGetApiVersionNumber");
        if version == 0 {
            Err(RdpewaHandlerError::fail("WebAuthn API unavailable"))
        } else {
            Ok(version)
        }
    }

    fn is_uvpaa(&mut self) -> RdpewaResult<bool> {
        let available = unsafe { WebAuthNIsUserVerifyingPlatformAuthenticatorAvailable() }
            .map_err(|e| hresult_err(e.code(), "IsUserVerifyingPlatformAuthenticatorAvailable failed"))?;
        Ok(available.as_bool())
    }

    fn cancel_current_operation(&mut self, cancellation_id: &[u8]) -> RdpewaResult<()> {
        cancel_guid_slot(&self.cancel_guid, guid_from_bytes(cancellation_id))
    }

    fn begin_webauthn(
        &mut self,
        request: WebAuthnOperationRequest,
        mut reply: RdpewaResponseSender,
    ) -> RdpewaResult<WebAuthnDispatch> {
        let parent_hwnd = self.parent_hwnd;
        let cancel_guid = Arc::clone(&self.cancel_guid);

        // Remember host-supplied cancel id so CancelCurOp on a later channel recreate can abort.
        if let Some(id) = request.para.cancellation_id.as_deref().and_then(guid_from_bytes) {
            *cancel_guid.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
        }

        thread::Builder::new()
            .name("ironrdp-rdpewa-webauthn".into())
            .spawn(move || {
                let hwnd = resolve_parent_hwnd(parent_hwnd);
                info!(
                    parent_hwnd,
                    resolved_hwnd = hwnd.0 as usize,
                    client_data_json_len = request.client_data_json.len(),
                    raw_request_len = request.raw_request.len(),
                    ?request.subcommand,
                    "Starting native WebAuthn operation"
                );

                // Prefer webauthn.dll oneshot (MSTSC remote-RPC path). It handles hash-only hosts
                // that omit clientDataJSON; public WebAuthN* cannot.
                if !request.raw_request.is_empty() {
                    match run_via_webauthn_dll_oneshot(&request) {
                        Ok(response_pdu) => {
                            let hresult = response_pdu
                                .get(..4)
                                .and_then(|b| b.try_into().ok())
                                .map(u32::from_le_bytes)
                                .unwrap_or(E_FAIL);
                            info!(
                                hresult = format!("0x{hresult:08X}"),
                                response_len = response_pdu.len(),
                                "WebAuthn completed via webauthn.dll oneshot"
                            );
                            reply.send_raw(response_pdu);
                            *cancel_guid.lock().unwrap_or_else(|e| e.into_inner()) = None;
                            return;
                        }
                        Err(err) => {
                            // Fall through to public WebAuthN* only when the host supplied JSON.
                            if request.client_data_json.is_empty() {
                                warn!(error = %err, "webauthn.dll oneshot failed (hash-only; no public API fallback)");
                                reply.send(RdpewaResponse::from_hresult(err.hresult));
                                *cancel_guid.lock().unwrap_or_else(|e| e.into_inner()) = None;
                                return;
                            }
                            warn!(
                                error = %err,
                                "webauthn.dll oneshot failed; falling back to public WebAuthN API"
                            );
                        }
                    }
                } else if request.client_data_json.is_empty() {
                    warn!("missing clientDataJSON and raw RDPEWA request");
                    reply.send(RdpewaResponse::from_hresult(E_INVALIDARG));
                    *cancel_guid.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    return;
                }

                let outcome = run_webauthn_operation(hwnd, &request, &cancel_guid);
                *cancel_guid.lock().unwrap_or_else(|e| e.into_inner()) = None;
                match outcome {
                    Ok(result) => {
                        let payload = WebAuthnResponsePayload {
                            device_info: result.device_info,
                            status: result.status,
                            response: result.response,
                        };
                        reply.send_webauthn(S_OK, &payload);
                    }
                    Err(err) => {
                        warn!(error = %err, "WebAuthn operation failed");
                        reply.send(RdpewaResponse::from_hresult(err.hresult));
                    }
                }
            })
            .map_err(|_| RdpewaHandlerError::fail("failed to spawn WebAuthn worker thread"))?;

        Ok(WebAuthnDispatch::Async)
    }
}

fn run_via_webauthn_dll_oneshot(request: &WebAuthnOperationRequest) -> RdpewaResult<Vec<u8>> {
    info!(
        request_len = request.raw_request.len(),
        client_data_json_len = request.client_data_json.len(),
        ?request.subcommand,
        "Forwarding RDPEWA request through webauthn.dll oneshot"
    );

    ironrdp_dvc_com_plugin::process_webauthn_dll_request(&request.raw_request).map_err(|message| {
        warn!(%message, "webauthn.dll oneshot failed");
        RdpewaHandlerError::new(E_FAIL, "webauthn.dll oneshot failed")
    })
}

/// Prefer the configured parent HWND; if unset, fall back to the foreground window so agent/daemon
/// sessions without an ActiveX HWND can still parent Windows Security UI.
fn resolve_parent_hwnd(parent_hwnd: isize) -> HWND {
    if parent_hwnd != 0 {
        return HWND(parent_hwnd as *mut core::ffi::c_void);
    }
    // SAFETY: GetForegroundWindow is a simple system query with no pointer lifetime.
    unsafe { GetForegroundWindow() }
}

fn run_webauthn_operation(
    hwnd: HWND,
    request: &WebAuthnOperationRequest,
    cancel_guid: &Arc<Mutex<Option<GUID>>>,
) -> RdpewaResult<WebAuthnOperationResponse> {
    match request.subcommand {
        WebAuthnSubcommand::MakeCredential => run_make_credential(hwnd, request, cancel_guid),
        WebAuthnSubcommand::GetAssertion => run_get_assertion(hwnd, request, cancel_guid),
    }
}

fn run_make_credential(
    hwnd: HWND,
    request: &WebAuthnOperationRequest,
    cancel_guid: &Arc<Mutex<Option<GUID>>>,
) -> RdpewaResult<WebAuthnOperationResponse> {
    let ctap = parse_make_credential(&request.ctap_cbor).map_err(|m| RdpewaHandlerError::new(E_INVALIDARG, m))?;

    let rp_id = request.rp_id.clone().unwrap_or_else(|| ctap.rp_id.clone());
    let rp_id_w = wide(&rp_id);
    let rp_name_w = wide(ctap.rp_name.as_deref().unwrap_or(rp_id.as_str()));
    let user_name_w = wide(ctap.user_name.as_deref().unwrap_or(""));
    let user_display_w = wide(ctap.user_display_name.as_deref().unwrap_or(""));

    let mut user_id = ctap.user_id.clone();
    let mut client_data_json = request.client_data_json.clone();
    // Hash-only hosts are handled before this path via webauthn.dll oneshot.
    if client_data_json.is_empty() {
        return Err(RdpewaHandlerError::new(E_INVALIDARG, "missing clientDataJSON"));
    }

    let rp_info = WEBAUTHN_RP_ENTITY_INFORMATION {
        dwVersion: WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION,
        pwszId: PCWSTR(rp_id_w.as_ptr()),
        pwszName: PCWSTR(rp_name_w.as_ptr()),
        pwszIcon: PCWSTR::null(),
    };

    let user_info = WEBAUTHN_USER_ENTITY_INFORMATION {
        dwVersion: WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION,
        cbId: u32_len(&user_id)?,
        pbId: user_id.as_mut_ptr(),
        pwszName: PCWSTR(user_name_w.as_ptr()),
        pwszIcon: PCWSTR::null(),
        pwszDisplayName: PCWSTR(user_display_w.as_ptr()),
    };

    let mut cose_params: Vec<WEBAUTHN_COSE_CREDENTIAL_PARAMETER> = ctap
        .algorithms
        .iter()
        .map(|alg| WEBAUTHN_COSE_CREDENTIAL_PARAMETER {
            dwVersion: WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION,
            pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
            lAlg: *alg,
        })
        .collect();
    let cose = WEBAUTHN_COSE_CREDENTIAL_PARAMETERS {
        cCredentialParameters: u32_len_items(cose_params.len())?,
        pCredentialParameters: cose_params.as_mut_ptr(),
    };

    let client_data = WEBAUTHN_CLIENT_DATA {
        dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
        cbClientDataJSON: u32_len(&client_data_json)?,
        pbClientDataJSON: client_data_json.as_mut_ptr(),
        pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
    };

    let mut exclude_ids = ctap.exclude_credential_ids.clone();
    let mut exclude_storage = CredentialListStorage::from_ids(&mut exclude_ids)?;

    let mut cancel_id = resolve_cancel_id(request, cancel_guid)?;

    let mut options = WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS {
        dwVersion: WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_CURRENT_VERSION,
        dwTimeoutMilliseconds: if request.timeout_ms == 0 {
            60_000
        } else {
            request.timeout_ms
        },
        dwAuthenticatorAttachment: attachment_to_win(request.para.attachment),
        bRequireResidentKey: (ctap.resident_key || request.para.require_resident_key).into(),
        dwUserVerificationRequirement: uv_to_win(request.para.user_verification),
        dwAttestationConveyancePreference: attestation_to_win(request.para.attestation),
        pCancellationId: &raw mut cancel_id,
        pExcludeCredentialList: exclude_storage.list_ptr(),
        ..Default::default()
    };

    info!(%rp_id, "WebAuthNAuthenticatorMakeCredential");
    let attestation = unsafe {
        WebAuthNAuthenticatorMakeCredential(
            hwnd,
            &rp_info,
            &user_info,
            &cose,
            &client_data,
            Some(&raw const options),
        )
    }
    .map_err(|e| map_webauthn_error(e.code()))?;

    if attestation.is_null() {
        return Err(RdpewaHandlerError::fail("MakeCredential returned null attestation"));
    }

    let result = unsafe { read_make_credential_result(attestation) };
    unsafe { WebAuthNFreeCredentialAttestation(Some(attestation)) };
    // Keep options/cancel_id alive until after the API returns.
    let _ = &mut options;
    result
}

fn run_get_assertion(
    hwnd: HWND,
    request: &WebAuthnOperationRequest,
    cancel_guid: &Arc<Mutex<Option<GUID>>>,
) -> RdpewaResult<WebAuthnOperationResponse> {
    let ctap = parse_get_assertion(&request.ctap_cbor).map_err(|m| RdpewaHandlerError::new(E_INVALIDARG, m))?;

    let rp_id = request.rp_id.clone().unwrap_or_else(|| ctap.rp_id.clone());
    let rp_id_w = wide(&rp_id);

    let mut client_data_json = request.client_data_json.clone();
    // Hash-only hosts are handled before this path via webauthn.dll oneshot.
    if client_data_json.is_empty() {
        return Err(RdpewaHandlerError::new(E_INVALIDARG, "missing clientDataJSON"));
    }

    let client_data = WEBAUTHN_CLIENT_DATA {
        dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
        cbClientDataJSON: u32_len(&client_data_json)?,
        pbClientDataJSON: client_data_json.as_mut_ptr(),
        pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
    };

    let mut allow_ids = ctap.allow_credential_ids.clone();
    let mut allow_storage = CredentialListStorage::from_ids(&mut allow_ids)?;

    let mut cancel_id = resolve_cancel_id(request, cancel_guid)?;

    let mut options = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS {
        dwVersion: WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_CURRENT_VERSION,
        dwTimeoutMilliseconds: if request.timeout_ms == 0 {
            60_000
        } else {
            request.timeout_ms
        },
        dwAuthenticatorAttachment: attachment_to_win(request.para.attachment),
        dwUserVerificationRequirement: uv_to_win(request.para.user_verification),
        pCancellationId: &raw mut cancel_id,
        pAllowCredentialList: allow_storage.list_ptr(),
        ..Default::default()
    };

    info!(%rp_id, "WebAuthNAuthenticatorGetAssertion");
    let assertion = unsafe {
        WebAuthNAuthenticatorGetAssertion(hwnd, PCWSTR(rp_id_w.as_ptr()), &client_data, Some(&raw const options))
    }
    .map_err(|e| map_webauthn_error(e.code()))?;

    if assertion.is_null() {
        return Err(RdpewaHandlerError::fail("GetAssertion returned null assertion"));
    }

    let result = unsafe { read_get_assertion_result(assertion) };
    unsafe { WebAuthNFreeAssertion(assertion) };
    let _ = &mut options;
    result
}

unsafe fn read_make_credential_result(
    attestation: *mut windows::Win32::Networking::WindowsWebServices::WEBAUTHN_CREDENTIAL_ATTESTATION,
) -> RdpewaResult<WebAuthnOperationResponse> {
    let att = unsafe { &*attestation };
    let fmt = unsafe { pcwstr_to_string(att.pwszFormatType) }.unwrap_or_else(|| String::from("none"));
    let auth_data = unsafe { slice_from_parts(att.pbAuthenticatorData, att.cbAuthenticatorData) };
    let att_stmt = unsafe { slice_from_parts(att.pbAttestation, att.cbAttestation) };

    let body = encode_make_credential_response(&fmt, auth_data, Some(att_stmt)).map_err(RdpewaHandlerError::fail)?;
    let transports = att.dwUsedTransport;
    let resident_key = Some(att.bResidentKey.as_bool());

    Ok(WebAuthnOperationResponse {
        device_info: WindowsRdpewaBackend::platform_device_info(transports, resident_key),
        status: 0,
        response: pack_ctap_response(0x00, &body),
    })
}

unsafe fn read_get_assertion_result(assertion: *mut WEBAUTHN_ASSERTION) -> RdpewaResult<WebAuthnOperationResponse> {
    let a = unsafe { &*assertion };
    let auth_data = unsafe { slice_from_parts(a.pbAuthenticatorData, a.cbAuthenticatorData) };
    let signature = unsafe { slice_from_parts(a.pbSignature, a.cbSignature) };
    let cred_id = unsafe { slice_from_parts(a.Credential.pbId, a.Credential.cbId) };
    let user_id = unsafe { slice_from_parts(a.pbUserId, a.cbUserId) };
    let user = if user_id.is_empty() { None } else { Some(user_id) };

    let body = encode_get_assertion_response(cred_id, auth_data, signature, user).map_err(RdpewaHandlerError::fail)?;

    Ok(WebAuthnOperationResponse {
        device_info: WindowsRdpewaBackend::platform_device_info(a.dwUsedTransport, None),
        status: 0,
        response: pack_ctap_response(0x00, &body),
    })
}

struct CredentialListStorage {
    _creds: Vec<WEBAUTHN_CREDENTIAL_EX>,
    _ptrs: Vec<*mut WEBAUTHN_CREDENTIAL_EX>,
    list: WEBAUTHN_CREDENTIAL_LIST,
}

impl CredentialListStorage {
    fn from_ids(ids: &mut [Vec<u8>]) -> RdpewaResult<Self> {
        if ids.is_empty() {
            return Ok(Self {
                _creds: Vec::new(),
                _ptrs: Vec::new(),
                list: WEBAUTHN_CREDENTIAL_LIST::default(),
            });
        }

        let mut creds: Vec<WEBAUTHN_CREDENTIAL_EX> = ids
            .iter_mut()
            .map(|id| WEBAUTHN_CREDENTIAL_EX {
                dwVersion: WEBAUTHN_CREDENTIAL_EX_CURRENT_VERSION,
                cbId: u32_len(id).unwrap_or(0),
                pbId: id.as_mut_ptr(),
                pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
                dwTransports: 0,
            })
            .collect();
        let mut ptrs: Vec<*mut WEBAUTHN_CREDENTIAL_EX> = creds.iter_mut().map(|c| c as *mut _).collect();
        let list = WEBAUTHN_CREDENTIAL_LIST {
            cCredentials: u32_len_items(ptrs.len())?,
            ppCredentials: ptrs.as_mut_ptr(),
        };
        Ok(Self {
            _creds: creds,
            _ptrs: ptrs,
            list,
        })
    }

    fn list_ptr(&mut self) -> *mut WEBAUTHN_CREDENTIAL_LIST {
        if self._creds.is_empty() {
            core::ptr::null_mut()
        } else {
            &raw mut self.list
        }
    }
}

fn attachment_to_win(v: Attachment) -> u32 {
    match v {
        Attachment::Platform => WEBAUTHN_AUTHENTICATOR_ATTACHMENT_PLATFORM,
        Attachment::CrossPlatform => WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
        Attachment::Any => WEBAUTHN_AUTHENTICATOR_ATTACHMENT_ANY,
    }
}

fn uv_to_win(v: UserVerification) -> u32 {
    match v {
        UserVerification::Required => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED,
        UserVerification::Preferred => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_PREFERRED,
        UserVerification::Discouraged => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
        UserVerification::Any => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_ANY,
    }
}

fn attestation_to_win(v: Attestation) -> u32 {
    match v {
        Attestation::None => WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE,
        Attestation::Indirect => WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_INDIRECT,
        Attestation::Direct => WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_DIRECT,
        Attestation::Any => WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_ANY,
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

fn u32_len(bytes: &[u8]) -> RdpewaResult<u32> {
    u32::try_from(bytes.len()).map_err(|_| RdpewaHandlerError::new(E_INVALIDARG, "buffer too large"))
}

fn u32_len_items(n: usize) -> RdpewaResult<u32> {
    u32::try_from(n).map_err(|_| RdpewaHandlerError::new(E_INVALIDARG, "too many items"))
}

unsafe fn slice_from_parts<'a>(ptr: *mut u8, len: u32) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(ptr, len as usize) }
    }
}

unsafe fn pcwstr_to_string(p: PCWSTR) -> Option<String> {
    if p.is_null() {
        None
    } else {
        unsafe { p.to_string().ok() }
    }
}

fn hresult_err(hr: HRESULT, message: &'static str) -> RdpewaHandlerError {
    RdpewaHandlerError::new(hr.0 as u32, message)
}

fn map_webauthn_error(hr: HRESULT) -> RdpewaHandlerError {
    let code = hr.0 as u32;
    // ERROR_CANCELLED / NTE_USER_CANCELLED-ish paths map to E_ABORT.
    if code == 0x8007_04C7 || code == 0x8009_0036 || code == E_ABORT {
        RdpewaHandlerError::new(E_ABORT, "WebAuthn operation cancelled")
    } else if code == 0 {
        RdpewaHandlerError::new(E_FAIL, "WebAuthn operation failed")
    } else {
        RdpewaHandlerError::new(code, "WebAuthn operation failed")
    }
}

/// Prefer a server-supplied 16-byte GUID; otherwise allocate one via the WebAuthn API.
fn resolve_cancel_id(request: &WebAuthnOperationRequest, cancel_guid: &Arc<Mutex<Option<GUID>>>) -> RdpewaResult<GUID> {
    let cancel_id = request
        .para
        .cancellation_id
        .as_deref()
        .and_then(guid_from_bytes)
        .map(Ok)
        .unwrap_or_else(|| {
            unsafe { WebAuthNGetCancellationId() }
                .map_err(|e| hresult_err(e.code(), "WebAuthNGetCancellationId failed"))
        })?;
    *cancel_guid.lock().unwrap_or_else(|e| e.into_inner()) = Some(cancel_id);
    Ok(cancel_id)
}

fn cancel_guid_slot(slot: &Arc<Mutex<Option<GUID>>>, preferred: Option<GUID>) -> RdpewaResult<()> {
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    let guid = preferred.or_else(|| guard.take());
    if preferred.is_some() {
        // Clear the slot when it matches the cancelled operation.
        if let (Some(preferred), Some(current)) = (preferred, *guard)
            && preferred == current
        {
            *guard = None;
        }
    }
    drop(guard);

    if let Some(guid) = guid {
        info!("Cancelling in-flight WebAuthn operation");
        unsafe { WebAuthNCancelCurrentOperation(&guid) }
            .map_err(|e| hresult_err(e.code(), "WebAuthNCancelCurrentOperation failed"))?;
    } else {
        debug!("No in-flight WebAuthn operation to cancel");
    }
    Ok(())
}

fn guid_from_bytes(bytes: &[u8]) -> Option<GUID> {
    if bytes.len() != 16 {
        return None;
    }
    Some(GUID {
        data1: u32::from_le_bytes(bytes[0..4].try_into().ok()?),
        data2: u16::from_le_bytes(bytes[4..6].try_into().ok()?),
        data3: u16::from_le_bytes(bytes[6..8].try_into().ok()?),
        data4: bytes[8..16].try_into().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_from_bytes_rejects_wrong_length() {
        assert!(guid_from_bytes(&[]).is_none());
        assert!(guid_from_bytes(&[0; 15]).is_none());
        assert!(guid_from_bytes(&[0; 17]).is_none());
    }

    #[test]
    fn guid_from_bytes_parses_windows_layout() {
        // {01020304-0506-0708-090A-0B0C0D0E0F10}
        let bytes = [
            0x04, 0x03, 0x02, 0x01, 0x06, 0x05, 0x08, 0x07, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
        ];
        let guid = guid_from_bytes(&bytes).expect("valid GUID bytes");
        assert_eq!(guid.data1, 0x0102_0304);
        assert_eq!(guid.data2, 0x0506);
        assert_eq!(guid.data3, 0x0708);
        assert_eq!(guid.data4, [0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10]);
    }
}
