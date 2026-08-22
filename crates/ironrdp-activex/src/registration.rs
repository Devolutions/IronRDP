use core::ffi::c_void;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, HMODULE};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
    GetModuleHandleExW,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW,
    RegSetValueExW,
};
use windows::core::PCWSTR;
use windows_core::{Error, HRESULT, Result};

use crate::com::{DllGetClassObject, own_class_id};

const PROGID: &str = "IronRDP.ActiveX.1";
const VERSION_INDEPENDENT_PROGID: &str = "IronRDP.ActiveX";

pub(crate) fn register_server() -> Result<()> {
    let class_id = format_guid(own_class_id());
    let module_path = module_path()?;
    let class_key = format!("CLSID\\{class_id}");

    let class = RegistryKey::create(HKEY_CLASSES_ROOT, &class_key)?;
    class.set_default("IronRDP ActiveX Automation Server")?;
    class.create_child("InprocServer32")?.set_default(&module_path)?;
    class
        .create_child("InprocServer32")?
        .set_value("ThreadingModel", "Apartment")?;
    class.create_child("ProgID")?.set_default(PROGID)?;
    class
        .create_child("VersionIndependentProgID")?
        .set_default(VERSION_INDEPENDENT_PROGID)?;

    let versioned = RegistryKey::create(HKEY_CLASSES_ROOT, PROGID)?;
    versioned.set_default("IronRDP ActiveX Automation Server")?;
    versioned.create_child("CLSID")?.set_default(&class_id)?;

    let unversioned = RegistryKey::create(HKEY_CLASSES_ROOT, VERSION_INDEPENDENT_PROGID)?;
    unversioned.set_default("IronRDP ActiveX Automation Server")?;
    unversioned.create_child("CLSID")?.set_default(&class_id)?;
    unversioned.create_child("CurVer")?.set_default(PROGID)?;

    Ok(())
}

pub(crate) fn unregister_server() -> Result<()> {
    let class_id = format_guid(own_class_id());
    for key in [
        format!("CLSID\\{class_id}"),
        PROGID.to_owned(),
        VERSION_INDEPENDENT_PROGID.to_owned(),
    ] {
        delete_tree_if_present(HKEY_CLASSES_ROOT, &key)?;
    }
    Ok(())
}

struct RegistryKey(HKEY);

impl RegistryKey {
    fn create(parent: HKEY, path: &str) -> Result<Self> {
        let path = wide(path);
        let mut key = HKEY::default();
        let result = unsafe {
            RegCreateKeyExW(
                parent,
                PCWSTR(path.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut key,
                None,
            )
        };
        registry_result(result)?;
        Ok(Self(key))
    }

    fn create_child(&self, path: &str) -> Result<Self> {
        Self::create(self.0, path)
    }

    fn set_default(&self, value: &str) -> Result<()> {
        self.set_value("", value)
    }

    fn set_value(&self, name: &str, value: &str) -> Result<()> {
        let name = wide(name);
        let value = wide(value);
        let bytes = unsafe { core::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * size_of::<u16>()) };
        let result = unsafe { RegSetValueExW(self.0, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)) };
        registry_result(result)
    }
}

impl Drop for RegistryKey {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn delete_tree_if_present(parent: HKEY, path: &str) -> Result<()> {
    let path = wide(path);
    let result = unsafe { RegDeleteTreeW(parent, PCWSTR(path.as_ptr())) };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    registry_result(result)
}

fn module_path() -> Result<String> {
    let mut module = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(DllGetClassObject as *const c_void as *const u16),
            &mut module,
        )
    }?;

    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetModuleFileNameW(Some(module), &mut buffer) };
    if length == 0 || length as usize >= buffer.len() {
        return Err(Error::from_hresult(HRESULT::from_win32(
            unsafe { windows::Win32::Foundation::GetLastError() }.0,
        )));
    }
    String::from_utf16(&buffer[..length as usize]).map_err(|_| Error::from_hresult(windows::Win32::Foundation::E_FAIL))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(core::iter::once(0)).collect()
}

fn registry_result(result: windows::Win32::Foundation::WIN32_ERROR) -> Result<()> {
    if result.is_ok() {
        Ok(())
    } else {
        Err(Error::from_hresult(HRESULT::from_win32(result.0)))
    }
}

fn format_guid(guid: windows_core::GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clsid_is_registered_in_canonical_format() {
        assert_eq!(format_guid(own_class_id()), "{5D3E2B4C-6860-462E-8E9D-0C4D2B094C5F}");
    }
}
