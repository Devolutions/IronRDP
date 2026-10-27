use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;

use tracing::warn;
use windows::Win32::Foundation::{E_INVALIDARG, ERROR_FILE_NOT_FOUND, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{AccessCheck, GENERIC_MAPPING, PRIVILEGE_SET, PSECURITY_DESCRIPTOR, TOKEN_QUERY};
use windows::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_READ_DATA,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RRF_NOEXPAND, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RRF_ZEROONFAILURE, RegCloseKey,
    RegGetValueW, RegOpenKeyExW,
};
use windows::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};
use windows_core::{BOOL, HSTRING, PCWSTR, w};

pub(crate) struct PublishedApplications {
    hive: HKEY,
    path: String,
}

pub(crate) struct PublishedApplication {
    pub(crate) program: String,
    pub(crate) parameters: String,
}

impl Default for PublishedApplications {
    fn default() -> Self {
        Self {
            hive: HKEY_LOCAL_MACHINE,
            path: r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Terminal Server\TSAppAllowList\Applications"
                .to_owned(),
        }
    }
}

impl PublishedApplications {
    pub(crate) fn resolve(
        &self,
        application: &str,
        parameters: &str,
    ) -> windows_core::Result<Option<PublishedApplication>> {
        let Some(alias) = application.strip_prefix("||") else {
            return Ok(None);
        };
        if alias.is_empty()
            || alias.contains(['\\', '/', '\0'])
            || matches!(alias, "." | "..")
            || !(1..=255).contains(&alias.encode_utf16().count())
        {
            return Ok(None);
        }

        let path = HSTRING::from(format!(r"{}\{alias}", self.path));
        let mut key = HKEY::default();
        // SAFETY: The predefined hive is valid and `key` is a writable handle output.
        let status = unsafe { RegOpenKeyExW(self.hive, &path, None, KEY_READ, &mut key) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        status.ok()?;
        let key = RegistryKey(key);
        let program =
            read_string(key.0, w!("Path"))?.ok_or_else(|| invalid_policy("published application path is missing"))?;
        if !Path::new(&program).is_absolute() || program.contains('"') || program.chars().any(char::is_control) {
            return Err(invalid_policy(
                "published application path must be absolute and unquoted",
            ));
        }
        if let Some(descriptor) = read_string(key.0, w!("SecurityDescriptor"))? {
            if !descriptor.is_empty() && !caller_has_access(&descriptor)? {
                return Ok(None);
            }
        }

        let mut setting = 0u32;
        let mut length = 4;
        // SAFETY: The output is one DWORD and the registry value must have that type.
        unsafe {
            RegGetValueW(
                key.0,
                PCWSTR::null(),
                w!("CommandLineSetting"),
                RRF_RT_REG_DWORD | RRF_ZEROONFAILURE,
                None,
                Some(core::ptr::from_mut(&mut setting).cast()),
                Some(&mut length),
            )
        }
        .ok()?;
        let parameters = match setting {
            0 => String::new(),
            1 => parameters.to_owned(),
            2 => read_string(key.0, w!("RequiredCommandLine"))?
                .ok_or_else(|| invalid_policy("published application required arguments are missing"))?,
            _ => return Err(invalid_policy("invalid published application argument policy")),
        };
        let program = if program.contains(' ') {
            format!("\"{program}\"")
        } else {
            program
        };
        Ok(Some(PublishedApplication { program, parameters }))
    }
}

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        // SAFETY: This object owns the handle returned by RegOpenKeyExW.
        if let Err(error) = unsafe { RegCloseKey(self.0) }.ok() {
            warn!(%error, "Failed to close published application registry key");
        }
    }
}

fn read_string(key: HKEY, name: PCWSTR) -> windows_core::Result<Option<String>> {
    let mut length = 0;
    // SAFETY: The live key and NUL-terminated value name remain valid for both reads.
    let status = unsafe {
        RegGetValueW(
            key,
            PCWSTR::null(),
            name,
            RRF_RT_REG_SZ | RRF_NOEXPAND,
            None,
            None,
            Some(&mut length),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    status.ok()?;
    if !(2..=0x8000).contains(&length) || length % 2 != 0 {
        return Err(invalid_policy("invalid published application registry string length"));
    }
    let units = usize::try_from(length / 2)
        .map_err(|_| invalid_policy("published application registry string length overflow"))?;
    let mut buffer = vec![0u16; units];
    // SAFETY: `length` is the writable byte capacity of the UTF-16 buffer.
    unsafe {
        RegGetValueW(
            key,
            PCWSTR::null(),
            name,
            RRF_RT_REG_SZ | RRF_NOEXPAND | RRF_ZEROONFAILURE,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut length),
        )
    }
    .ok()?;
    let returned_units = usize::try_from(length / 2)
        .map_err(|_| invalid_policy("published application registry string length overflow"))?;
    let value = buffer
        .get(..returned_units)
        .filter(|value| length % 2 == 0 && value.last() == Some(&0))
        .ok_or_else(|| invalid_policy("unterminated published application registry string"))?;
    let value = &value[..value.len() - 1];
    if value.contains(&0) {
        return Err(invalid_policy(
            "published application registry string contains an embedded terminator",
        ));
    }
    String::from_utf16(value)
        .map(Some)
        .map_err(|_| invalid_policy("published application registry string is not valid UTF-16"))
}

fn caller_has_access(sddl: &str) -> windows_core::Result<bool> {
    let mut token = HANDLE::default();
    // SAFETY: Getting the current thread's pseudo-handle requires no preconditions.
    let thread = unsafe { GetCurrentThread() };
    // SAFETY: The handle output is writable; never substitute the privileged process token for the RPC caller.
    unsafe { OpenThreadToken(thread, TOKEN_QUERY, true, &mut token) }?;
    // SAFETY: OpenThreadToken transfers ownership of this handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token.0) };
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: The string is NUL-terminated and the descriptor output is writable.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(sddl),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }?;

    // Windows's RemoteDesktop module grants read-data access to each published-app user group.
    // Use file-rights mapping so write-only or execute-only ACEs do not also grant read access.
    let mapping = GENERIC_MAPPING {
        GenericRead: FILE_GENERIC_READ.0,
        GenericWrite: FILE_GENERIC_WRITE.0,
        GenericExecute: FILE_GENERIC_EXECUTE.0,
        GenericAll: FILE_ALL_ACCESS.0,
    };
    let mut privileges = PRIVILEGE_SET::default();
    let mut privilege_length = u32::try_from(size_of::<PRIVILEGE_SET>()).expect("PRIVILEGE_SET fits in a DWORD");
    let mut granted = 0;
    let mut allowed = BOOL::default();
    // SAFETY: The descriptor, impersonation token and output buffers remain live for the access check.
    let result = unsafe {
        AccessCheck(
            descriptor,
            HANDLE(token.as_raw_handle()),
            FILE_READ_DATA.0,
            &mapping,
            Some(&mut privileges),
            &mut privilege_length,
            &mut granted,
            &mut allowed,
        )
    };
    // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW allocated this buffer with LocalAlloc.
    let remaining = unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    if !remaining.is_invalid() {
        return Err(windows_core::Error::from_thread());
    }
    result?;
    Ok(allowed.as_bool())
}

fn invalid_policy(message: &'static str) -> windows_core::Error {
    windows_core::Error::new(E_INVALIDARG, message)
}

#[cfg(test)]
impl PublishedApplications {
    pub(crate) fn for_test(hive: HKEY, path: String) -> Self {
        Self { hive, path }
    }
}
