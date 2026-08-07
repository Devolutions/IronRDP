use ironrdp_pdu::{PduResult, encode_err};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    ClientDriveQuerySecurityResponse, ClientDriveSetSecurityResponse, DeviceIoResponse, NtStatus, SecurityInformation,
    ServerDriveQuerySecurityRequest, ServerDriveSetSecurityRequest,
};
use ironrdp_svc::SvcMessage;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, ERROR_INVALID_SECURITY_DESCR, ERROR_NO_TOKEN,
    ERROR_NOT_ALL_ASSIGNED, ERROR_PRIVILEGE_NOT_HELD, ERROR_SUCCESS, GetLastError, HANDLE, HLOCAL, LUID, LocalFree,
    SetLastError, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo};
use windows::Win32::Security::{
    ACL, AdjustTokenPrivileges, GetSecurityDescriptorDacl, GetSecurityDescriptorGroup, GetSecurityDescriptorLength,
    GetSecurityDescriptorOwner, GetSecurityDescriptorSacl, ImpersonateSelf, IsValidSecurityDescriptor,
    LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, OBJECT_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, RevertToSelf,
    SE_BACKUP_NAME, SE_PRIVILEGE_ENABLED, SE_RESTORE_NAME, SE_SECURITY_NAME, SE_SELF_RELATIVE, SecurityImpersonation,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};

use super::backend::WindowsRdpdrBackend;
use super::file::file_for_request;
use super::handles::FileHandle;

const MAX_SECURITY_DESCRIPTOR_SIZE: usize = 1024 * 1024;
const SECURITY_DESCRIPTOR_RELATIVE_SIZE: usize = 20;
const ACL_HEADER_SIZE: usize = 8;
const ACE_HEADER_SIZE: usize = 4;
const SID_HEADER_SIZE: usize = 8;
pub(super) const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;

pub(crate) fn query(backend: &WindowsRdpdrBackend, req: ServerDriveQuerySecurityRequest) -> PduResult<Vec<SvcMessage>> {
    let (status, security_descriptor) = match query_inner(backend, &req) {
        Ok(security_descriptor) => (NtStatus::SUCCESS, Some(security_descriptor)),
        Err(status) => (status, None),
    };

    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQuerySecurityResponse(
        ClientDriveQuerySecurityResponse {
            device_io_response: DeviceIoResponse::new(req.device_io_request, status),
            security_descriptor,
        },
    ))])
}

pub(crate) fn set(backend: &WindowsRdpdrBackend, req: ServerDriveSetSecurityRequest) -> PduResult<Vec<SvcMessage>> {
    let status = set_inner(backend, &req).map_or_else(|status| status, |_| NtStatus::SUCCESS);
    let response = ClientDriveSetSecurityResponse::new(&req, status).map_err(|error| encode_err!(error))?;

    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveSetSecurityResponse(
        response,
    ))])
}

fn query_inner(backend: &WindowsRdpdrBackend, req: &ServerDriveQuerySecurityRequest) -> Result<Vec<u8>, NtStatus> {
    if req.security_information.is_empty() {
        return Err(NtStatus::INVALID_PARAMETER);
    }

    let file = file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)?;
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let required_privileges = query_required_privileges(req.security_information);
    let _security_privilege = ScopedSecurityPrivilege::enable(&required_privileges)?;

    // SAFETY: the file handle remains open for this synchronous query and
    // `descriptor` is writable storage for the LocalAlloc-owned result.
    let status = unsafe {
        GetSecurityInfo(
            file.handle.as_raw(),
            SE_FILE_OBJECT,
            object_security_information(req.security_information),
            None,
            None,
            None,
            None,
            Some(core::ptr::addr_of_mut!(descriptor)),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(from_win32_error(status));
    }

    let descriptor = LocalSecurityDescriptor(descriptor);
    if descriptor.0.0.is_null() {
        return Err(NtStatus::UNSUCCESSFUL);
    }

    // SAFETY: GetSecurityInfo returned a valid, self-relative security
    // descriptor whose LocalAlloc ownership is retained by `descriptor`.
    let length = unsafe { GetSecurityDescriptorLength(descriptor.0) };
    let length = usize::try_from(length).map_err(|_| NtStatus::UNSUCCESSFUL)?;
    if length == 0 || length > MAX_SECURITY_DESCRIPTOR_SIZE {
        return Err(NtStatus::UNSUCCESSFUL);
    }

    // SAFETY: the API-reported descriptor length is bounded above and the
    // LocalAlloc allocation remains live until `descriptor` is dropped.
    Ok(unsafe { core::slice::from_raw_parts(descriptor.0.0.cast::<u8>(), length) }.to_vec())
}

fn set_inner(backend: &WindowsRdpdrBackend, req: &ServerDriveSetSecurityRequest) -> Result<(), NtStatus> {
    if req.security_information.is_empty() {
        return Err(NtStatus::INVALID_PARAMETER);
    }

    let file = file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)?;
    if file.read_only {
        return Err(NtStatus::MEDIA_WRITE_PROTECTED);
    }

    let descriptor = RelativeSecurityDescriptor::new(&req.security_descriptor)?;
    let required_privileges = set_required_privileges(req.security_information);
    let _security_privilege = ScopedSecurityPrivilege::enable(&required_privileges)?;
    let status = descriptor.set_on(&file.handle, req.security_information)?;
    if status != ERROR_SUCCESS {
        return Err(from_win32_error(status));
    }

    Ok(())
}

fn object_security_information(security_information: SecurityInformation) -> OBJECT_SECURITY_INFORMATION {
    OBJECT_SECURITY_INFORMATION(security_information.bits())
}

pub(super) fn enable_for_access_system_security(
    desired_access: u32,
) -> Result<Option<ScopedSecurityPrivilege>, NtStatus> {
    let privileges = (desired_access & ACCESS_SYSTEM_SECURITY != 0)
        .then_some(SecurityPrivilege::Security)
        .into_iter()
        .collect::<Vec<_>>();
    ScopedSecurityPrivilege::enable(&privileges)
}

fn query_required_privileges(security_information: SecurityInformation) -> Vec<SecurityPrivilege> {
    let mut privileges = Vec::new();
    if contains_sacl_information(security_information) {
        privileges.push(SecurityPrivilege::Security);
    }
    if security_information.contains(SecurityInformation::BACKUP) {
        privileges.push(SecurityPrivilege::Backup);
    }
    privileges
}

fn set_required_privileges(security_information: SecurityInformation) -> Vec<SecurityPrivilege> {
    let mut privileges = Vec::new();
    if contains_sacl_information(security_information) {
        privileges.push(SecurityPrivilege::Security);
    }
    if security_information.contains(SecurityInformation::BACKUP) {
        privileges.push(SecurityPrivilege::Restore);
    }
    privileges
}

fn contains_sacl_information(security_information: SecurityInformation) -> bool {
    security_information.intersects(
        SecurityInformation::SACL
            | SecurityInformation::LABEL
            | SecurityInformation::ATTRIBUTE
            | SecurityInformation::SCOPE
            | SecurityInformation::PROCESS_TRUST_LABEL
            | SecurityInformation::PROTECTED_SACL
            | SecurityInformation::UNPROTECTED_SACL,
    )
}

fn from_win32_error(error: WIN32_ERROR) -> NtStatus {
    match error {
        ERROR_ACCESS_DENIED => NtStatus::ACCESS_DENIED,
        ERROR_INVALID_PARAMETER | ERROR_INVALID_SECURITY_DESCR => NtStatus::INVALID_PARAMETER,
        ERROR_PRIVILEGE_NOT_HELD => NtStatus::PRIVILEGE_NOT_HELD,
        _ => NtStatus::UNSUCCESSFUL,
    }
}

struct LocalSecurityDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            // SAFETY: GetSecurityInfo allocated this descriptor with LocalAlloc
            // and this guard owns its sole release.
            let _ = unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
        }
    }
}

struct OwnedTokenHandle(HANDLE);

impl Drop for OwnedTokenHandle {
    fn drop(&mut self) {
        // SAFETY: this is a real token handle opened by OpenThreadToken and
        // owned exclusively by this guard.
        if let Err(error) = unsafe { CloseHandle(self.0) } {
            tracing::warn!(%error, "Failed to close security privilege token");
        }
    }
}

pub(super) struct ScopedSecurityPrivilege {
    token: OwnedTokenHandle,
    previous: Vec<TOKEN_PRIVILEGES>,
    impersonated_self: bool,
}

impl ScopedSecurityPrivilege {
    fn enable(privileges: &[SecurityPrivilege]) -> Result<Option<Self>, NtStatus> {
        if privileges.is_empty() {
            return Ok(None);
        }

        let (token, impersonated_self) = open_current_thread_token()?;

        let mut previous = Vec::with_capacity(privileges.len());
        for privilege in privileges {
            match enable_privilege(token.0, *privilege) {
                Ok(previous_privilege) => previous.push(previous_privilege),
                Err(status) => {
                    restore_privileges(token.0, &previous);
                    if impersonated_self {
                        revert_to_self();
                    }
                    return Err(status);
                }
            }
        }

        Ok(Some(Self {
            token,
            previous,
            impersonated_self,
        }))
    }
}

fn open_current_thread_token() -> Result<(OwnedTokenHandle, bool), NtStatus> {
    // SAFETY: GetCurrentThread takes no arguments and returns a pseudo-handle
    // that must not be closed.
    let thread = unsafe { GetCurrentThread() };
    let mut token = HANDLE::default();

    // SAFETY: `thread` is valid and `token` is writable output storage.
    match unsafe {
        OpenThreadToken(
            thread,
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            false,
            core::ptr::addr_of_mut!(token),
        )
    } {
        Ok(()) => Ok((OwnedTokenHandle(token), false)),
        Err(error) if error.code() == windows::core::HRESULT::from_win32(ERROR_NO_TOKEN.0) => {
            // No impersonation token exists on this thread. Create one from
            // the process token so privilege changes remain thread-local.
            // SAFETY: SecurityImpersonation is a valid impersonation level.
            unsafe { ImpersonateSelf(SecurityImpersonation) }.map_err(|_| NtStatus::PRIVILEGE_NOT_HELD)?;

            // SAFETY: successful ImpersonateSelf installs a thread token and
            // `token` remains writable output storage.
            let opened = unsafe {
                OpenThreadToken(
                    thread,
                    TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                    false,
                    core::ptr::addr_of_mut!(token),
                )
            };
            match opened {
                Ok(()) => Ok((OwnedTokenHandle(token), true)),
                Err(_) => {
                    revert_to_self();
                    Err(NtStatus::PRIVILEGE_NOT_HELD)
                }
            }
        }
        Err(_) => Err(NtStatus::PRIVILEGE_NOT_HELD),
    }
}

fn revert_to_self() {
    // SAFETY: this only removes a thread impersonation token previously
    // installed by ImpersonateSelf in this module.
    if let Err(error) = unsafe { RevertToSelf() } {
        tracing::warn!(%error, "Failed to revert RDPDR thread impersonation");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SecurityPrivilege {
    Security,
    Backup,
    Restore,
}

impl SecurityPrivilege {
    fn name(self) -> windows::core::PCWSTR {
        match self {
            Self::Security => SE_SECURITY_NAME,
            Self::Backup => SE_BACKUP_NAME,
            Self::Restore => SE_RESTORE_NAME,
        }
    }
}

fn enable_privilege(token: HANDLE, privilege: SecurityPrivilege) -> Result<TOKEN_PRIVILEGES, NtStatus> {
    let mut luid = LUID::default();
    // SAFETY: the local system name and standard privilege name are valid, and
    // `luid` is writable output storage.
    unsafe { LookupPrivilegeValueW(None, privilege.name(), core::ptr::addr_of_mut!(luid)) }
        .map_err(|_| NtStatus::PRIVILEGE_NOT_HELD)?;
    let requested = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    let mut previous = TOKEN_PRIVILEGES::default();
    let mut previous_size = 0;
    // AdjustTokenPrivileges leaves the last-error value unchanged on a full
    // success, so clear it before checking for ERROR_NOT_ALL_ASSIGNED.
    // SAFETY: SetLastError only updates the calling thread's error state.
    unsafe { SetLastError(ERROR_SUCCESS) };
    // SAFETY: the current thread token is valid; the fixed one-privilege
    // states are exactly the sizes expected for this single adjustment.
    unsafe {
        AdjustTokenPrivileges(
            token,
            false,
            Some(core::ptr::addr_of!(requested)),
            u32::try_from(size_of::<TOKEN_PRIVILEGES>()).expect("token privilege state fits in u32"),
            Some(core::ptr::addr_of_mut!(previous)),
            Some(core::ptr::addr_of_mut!(previous_size)),
        )
    }
    .map_err(|_| NtStatus::PRIVILEGE_NOT_HELD)?;
    // AdjustTokenPrivileges reports a missing assigned privilege through
    // GetLastError even when the function call itself succeeds.
    // SAFETY: GetLastError only reads the calling thread's error state.
    if unsafe { GetLastError() } == ERROR_NOT_ALL_ASSIGNED {
        return Err(NtStatus::PRIVILEGE_NOT_HELD);
    }

    Ok(previous)
}

fn restore_privileges(token: HANDLE, privileges: &[TOKEN_PRIVILEGES]) {
    for privilege in privileges.iter().rev() {
        // SAFETY: `token` is valid and `privilege` is the one-privilege state
        // returned by AdjustTokenPrivileges when the scope enabled the privilege.
        let restore =
            unsafe { AdjustTokenPrivileges(token, false, Some(core::ptr::addr_of!(*privilege)), 0, None, None) };
        if let Err(error) = restore {
            tracing::warn!(%error, "Failed to restore security privilege");
        }
    }
}

impl Drop for ScopedSecurityPrivilege {
    fn drop(&mut self) {
        restore_privileges(self.token.0, &self.previous);
        if self.impersonated_self {
            revert_to_self();
        }
    }
}

struct RelativeSecurityDescriptor<'a> {
    bytes: &'a [u8],
}

impl<'a> RelativeSecurityDescriptor<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, NtStatus> {
        if bytes.len() < SECURITY_DESCRIPTOR_RELATIVE_SIZE || bytes.len() > MAX_SECURITY_DESCRIPTOR_SIZE {
            return Err(NtStatus::INVALID_PARAMETER);
        }
        if bytes[0] != 1 {
            return Err(NtStatus::INVALID_PARAMETER);
        }

        let control = read_u16(bytes, 2)?;
        if control & SE_SELF_RELATIVE.0 == 0 {
            return Err(NtStatus::INVALID_PARAMETER);
        }

        for offset in [4, 8] {
            if let Some(offset) = relative_offset(bytes, offset)? {
                validate_sid(bytes, offset)?;
            }
        }
        for offset in [12, 16] {
            if let Some(offset) = relative_offset(bytes, offset)? {
                validate_acl(bytes, offset)?;
            }
        }

        let descriptor = Self { bytes };
        // SAFETY: all relative component offsets and their variable-length
        // SID/ACL payloads have been bounded to `bytes` before validation.
        if !unsafe { IsValidSecurityDescriptor(descriptor.as_raw()) }.as_bool() {
            return Err(NtStatus::INVALID_PARAMETER);
        }

        Ok(descriptor)
    }

    fn set_on(&self, handle: &FileHandle, security_information: SecurityInformation) -> Result<WIN32_ERROR, NtStatus> {
        let backup = security_information.contains(SecurityInformation::BACKUP);
        let owner = (backup || security_information.contains(SecurityInformation::OWNER))
            .then(|| self.owner())
            .transpose()?;
        let group = (backup || security_information.contains(SecurityInformation::GROUP))
            .then(|| self.group())
            .transpose()?;
        let dacl = self.dacl_for(security_information)?;
        let sacl = (backup || contains_sacl_information(security_information))
            .then(|| self.sacl())
            .transpose()?;

        // SAFETY: `self` validates the self-relative descriptor and all
        // extracted SID/ACL pointers remain valid for the duration of this
        // synchronous handle-relative call.
        Ok(unsafe {
            SetSecurityInfo(
                handle.as_raw(),
                SE_FILE_OBJECT,
                object_security_information(security_information),
                owner,
                group,
                dacl,
                sacl,
            )
        })
    }

    fn owner(&self) -> Result<PSID, NtStatus> {
        let mut owner = PSID::default();
        let mut defaulted = windows::core::BOOL::default();
        // SAFETY: self-relative descriptor was structurally and semantically
        // validated when this value was constructed.
        unsafe { GetSecurityDescriptorOwner(self.as_raw(), core::ptr::addr_of_mut!(owner), &mut defaulted) }
            .map_err(|_| NtStatus::INVALID_PARAMETER)?;
        if owner.0.is_null() {
            return Err(NtStatus::INVALID_PARAMETER);
        }
        Ok(owner)
    }

    fn group(&self) -> Result<PSID, NtStatus> {
        let mut group = PSID::default();
        let mut defaulted = windows::core::BOOL::default();
        // SAFETY: self-relative descriptor was structurally and semantically
        // validated when this value was constructed.
        unsafe { GetSecurityDescriptorGroup(self.as_raw(), core::ptr::addr_of_mut!(group), &mut defaulted) }
            .map_err(|_| NtStatus::INVALID_PARAMETER)?;
        if group.0.is_null() {
            return Err(NtStatus::INVALID_PARAMETER);
        }
        Ok(group)
    }

    fn dacl(&self) -> Result<*const ACL, NtStatus> {
        let mut present = windows::core::BOOL::default();
        let mut dacl = core::ptr::null_mut();
        let mut defaulted = windows::core::BOOL::default();
        // SAFETY: self-relative descriptor was structurally and semantically
        // validated when this value was constructed.
        unsafe {
            GetSecurityDescriptorDacl(
                self.as_raw(),
                core::ptr::addr_of_mut!(present),
                core::ptr::addr_of_mut!(dacl),
                core::ptr::addr_of_mut!(defaulted),
            )
        }
        .map_err(|_| NtStatus::INVALID_PARAMETER)?;
        if !present.as_bool() || dacl.is_null() {
            // SetSecurityInfo interprets a null DACL as "grant everyone full
            // access", which must never be accepted from the remote server.
            return Err(NtStatus::INVALID_PARAMETER);
        }
        Ok(dacl.cast_const())
    }

    fn dacl_for(&self, security_information: SecurityInformation) -> Result<Option<*const ACL>, NtStatus> {
        (security_information.contains(SecurityInformation::BACKUP)
            || security_information.contains(SecurityInformation::DACL))
        .then(|| self.dacl())
        .transpose()
    }

    fn sacl(&self) -> Result<*const ACL, NtStatus> {
        let mut present = windows::core::BOOL::default();
        let mut sacl = core::ptr::null_mut();
        let mut defaulted = windows::core::BOOL::default();
        // SAFETY: self-relative descriptor was structurally and semantically
        // validated when this value was constructed.
        unsafe {
            GetSecurityDescriptorSacl(
                self.as_raw(),
                core::ptr::addr_of_mut!(present),
                core::ptr::addr_of_mut!(sacl),
                core::ptr::addr_of_mut!(defaulted),
            )
        }
        .map_err(|_| NtStatus::INVALID_PARAMETER)?;
        Ok(sacl.cast_const())
    }

    fn as_raw(&self) -> PSECURITY_DESCRIPTOR {
        PSECURITY_DESCRIPTOR(self.bytes.as_ptr().cast_mut().cast())
    }
}

fn relative_offset(bytes: &[u8], field_offset: usize) -> Result<Option<usize>, NtStatus> {
    let offset = usize::try_from(read_u32(bytes, field_offset)?).map_err(|_| NtStatus::INVALID_PARAMETER)?;
    if offset == 0 {
        return Ok(None);
    }
    if offset < SECURITY_DESCRIPTOR_RELATIVE_SIZE || offset >= bytes.len() {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    Ok(Some(offset))
}

fn validate_sid(bytes: &[u8], offset: usize) -> Result<(), NtStatus> {
    let header = bytes
        .get(offset..offset + SID_HEADER_SIZE)
        .ok_or(NtStatus::INVALID_PARAMETER)?;
    if header[0] != 1 {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    let length = SID_HEADER_SIZE
        .checked_add(
            usize::from(header[1])
                .checked_mul(size_of::<u32>())
                .ok_or(NtStatus::INVALID_PARAMETER)?,
        )
        .ok_or(NtStatus::INVALID_PARAMETER)?;
    bytes
        .get(offset..offset.checked_add(length).ok_or(NtStatus::INVALID_PARAMETER)?)
        .ok_or(NtStatus::INVALID_PARAMETER)?;
    Ok(())
}

fn validate_acl(bytes: &[u8], offset: usize) -> Result<(), NtStatus> {
    let header = bytes
        .get(offset..offset + ACL_HEADER_SIZE)
        .ok_or(NtStatus::INVALID_PARAMETER)?;
    let length = usize::from(u16::from_le_bytes([header[2], header[3]]));
    let ace_count = usize::from(u16::from_le_bytes([header[4], header[5]]));
    if length < ACL_HEADER_SIZE {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    let end = offset.checked_add(length).ok_or(NtStatus::INVALID_PARAMETER)?;
    bytes.get(offset..end).ok_or(NtStatus::INVALID_PARAMETER)?;

    let mut ace_offset = offset + ACL_HEADER_SIZE;
    for _ in 0..ace_count {
        let ace = bytes
            .get(ace_offset..ace_offset + ACE_HEADER_SIZE)
            .ok_or(NtStatus::INVALID_PARAMETER)?;
        let ace_length = usize::from(u16::from_le_bytes([ace[2], ace[3]]));
        if ace_length < ACE_HEADER_SIZE {
            return Err(NtStatus::INVALID_PARAMETER);
        }
        ace_offset = ace_offset.checked_add(ace_length).ok_or(NtStatus::INVALID_PARAMETER)?;
        if ace_offset > end {
            return Err(NtStatus::INVALID_PARAMETER);
        }
    }

    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, NtStatus> {
    let value = bytes
        .get(offset..offset + size_of::<u16>())
        .ok_or(NtStatus::INVALID_PARAMETER)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, NtStatus> {
    let value = bytes
        .get(offset..offset + size_of::<u32>())
        .ok_or(NtStatus::INVALID_PARAMETER)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_rdpdr::pdu::efs::{
        CreateDisposition, CreateOptions, DesiredAccess, DeviceCreateRequest, DeviceIoRequest, FileAttributes,
        MajorFunction, MinorFunction, SharedAccess,
    };
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::windows::factory::RedirectedDrive;
    use crate::windows::file::create_inner;

    #[test]
    fn query_and_set_dacl_use_the_opened_file_handle() {
        let (_temporary_file, backend, file_id) = open_test_file(false, 0x0006_0080);
        let query = ServerDriveQuerySecurityRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: 2,
                major_function: MajorFunction::QuerySecurity,
                minor_function: MinorFunction::from(0),
            },
            security_information: SecurityInformation::OWNER | SecurityInformation::GROUP | SecurityInformation::DACL,
        };

        let descriptor = query_inner(&backend, &query).expect("query DACL through redirected handle");
        RelativeSecurityDescriptor::new(&descriptor).expect("Windows returned a valid self-relative descriptor");
        let set = ServerDriveSetSecurityRequest {
            device_io_request: DeviceIoRequest {
                major_function: MajorFunction::SetSecurity,
                completion_id: 3,
                ..query.device_io_request
            },
            security_information: SecurityInformation::DACL,
            security_descriptor: descriptor.clone(),
        };
        set_inner(&backend, &set).expect("reapply DACL through redirected handle");

        let protection_only_set = ServerDriveSetSecurityRequest {
            security_information: SecurityInformation::PROTECTED_DACL,
            security_descriptor: descriptor,
            ..set
        };
        set_inner(&backend, &protection_only_set)
            .expect("set DACL protection without supplying a DACL pointer to SetSecurityInfo");
    }

    #[test]
    fn query_security_requires_read_control_on_the_opened_handle() {
        let (_temporary_file, backend, file_id) = open_test_file(false, 0x0000_0080);
        let query = ServerDriveQuerySecurityRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: 2,
                major_function: MajorFunction::QuerySecurity,
                minor_function: MinorFunction::from(0),
            },
            security_information: SecurityInformation::OWNER,
        };

        assert_eq!(query_inner(&backend, &query), Err(NtStatus::ACCESS_DENIED));
    }

    #[test]
    fn read_only_drive_rejects_security_mutation_before_parsing_the_descriptor() {
        let (_temporary_file, backend, file_id) = open_test_file(true, 0x0002_0080);
        let request = ServerDriveSetSecurityRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: 2,
                major_function: MajorFunction::SetSecurity,
                minor_function: MinorFunction::from(0),
            },
            security_information: SecurityInformation::DACL,
            security_descriptor: Vec::new(),
        };

        assert_eq!(set_inner(&backend, &request), Err(NtStatus::MEDIA_WRITE_PROTECTED));
    }

    #[test]
    fn security_operations_request_their_required_privileges() {
        assert_eq!(
            query_required_privileges(SecurityInformation::SACL | SecurityInformation::BACKUP),
            vec![SecurityPrivilege::Security, SecurityPrivilege::Backup]
        );
        assert_eq!(
            set_required_privileges(SecurityInformation::SACL | SecurityInformation::BACKUP),
            vec![SecurityPrivilege::Security, SecurityPrivilege::Restore]
        );
        assert_eq!(
            query_required_privileges(
                SecurityInformation::LABEL | SecurityInformation::ATTRIBUTE | SecurityInformation::PROCESS_TRUST_LABEL
            ),
            vec![SecurityPrivilege::Security]
        );
        assert_eq!(
            set_required_privileges(
                SecurityInformation::LABEL | SecurityInformation::ATTRIBUTE | SecurityInformation::PROCESS_TRUST_LABEL
            ),
            vec![SecurityPrivilege::Security]
        );
    }

    #[test]
    fn malformed_self_relative_descriptor_is_rejected_before_windows_parses_it() {
        let mut descriptor = vec![0; SECURITY_DESCRIPTOR_RELATIVE_SIZE];
        descriptor[0] = 1; // Revision
        descriptor[2..4].copy_from_slice(&SE_SELF_RELATIVE.0.to_le_bytes());
        descriptor[4..8].copy_from_slice(
            &u32::try_from(SECURITY_DESCRIPTOR_RELATIVE_SIZE)
                .expect("descriptor header size fits in u32")
                .to_le_bytes(),
        );

        assert!(matches!(
            RelativeSecurityDescriptor::new(&descriptor),
            Err(NtStatus::INVALID_PARAMETER)
        ));
    }

    #[test]
    fn rejects_absent_or_null_dacls() {
        for control in [
            SE_SELF_RELATIVE.0,
            SE_SELF_RELATIVE.0 | windows::Win32::Security::SE_DACL_PRESENT.0,
        ] {
            let mut descriptor = vec![0; SECURITY_DESCRIPTOR_RELATIVE_SIZE];
            descriptor[0] = 1; // Revision
            descriptor[2..4].copy_from_slice(&control.to_le_bytes());
            let descriptor = RelativeSecurityDescriptor::new(&descriptor)
                .expect("a descriptor without an owner, group, or DACL is structurally valid");

            assert_eq!(descriptor.dacl(), Err(NtStatus::INVALID_PARAMETER));
            assert_eq!(
                descriptor
                    .dacl_for(SecurityInformation::PROTECTED_DACL)
                    .expect("DACL protection does not need a DACL payload"),
                None
            );
        }
    }

    #[test]
    fn rejects_absent_owner_or_group_when_requested() {
        let mut bytes = vec![0; SECURITY_DESCRIPTOR_RELATIVE_SIZE];
        bytes[0] = 1; // Revision
        bytes[2..4].copy_from_slice(&SE_SELF_RELATIVE.0.to_le_bytes());
        let descriptor = RelativeSecurityDescriptor::new(&bytes).expect("structurally valid security descriptor");

        assert_eq!(descriptor.owner(), Err(NtStatus::INVALID_PARAMETER));
        assert_eq!(descriptor.group(), Err(NtStatus::INVALID_PARAMETER));
    }

    fn open_test_file(read_only: bool, desired_access: u32) -> (TemporaryFile, WindowsRdpdrBackend, u32) {
        let temporary_file = TemporaryFile::create();
        let root_path = temporary_file
            .0
            .ancestors()
            .last()
            .expect("temporary file has a volume root")
            .to_owned();
        let relative_path = temporary_file
            .0
            .strip_prefix(&root_path)
            .expect("temporary file is beneath its volume root");
        let drive = RedirectedDrive::new(1, "Test", root_path, read_only).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::new(vec![drive]).expect("open redirected root");
        let request = DeviceCreateRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id: 0,
                completion_id: 1,
                major_function: MajorFunction::Create,
                minor_function: MinorFunction::from(0),
            },
            desired_access: DesiredAccess::from_bits_retain(desired_access),
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::empty(),
            create_disposition: CreateDisposition::FILE_OPEN,
            create_options: CreateOptions::empty(),
            path: format!(r"\{}", relative_path.display()),
        };
        let (file_id, _) = create_inner(&mut backend, &request).expect("open temporary file");
        (temporary_file, backend, file_id)
    }

    struct TemporaryFile(PathBuf);

    impl TemporaryFile {
        fn create() -> Self {
            let path = std::env::temp_dir().join(format!(
                "ironrdp-rdpdr-security-{}-{}.tmp",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock is after Unix epoch")
                    .as_nanos()
            ));
            std::fs::write(&path, b"security").expect("create temporary file");
            Self(path)
        }
    }

    impl Drop for TemporaryFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}
