//! Device tree queries through the Configuration Manager.

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_DRP_PHYSICAL_DEVICE_OBJECT_NAME, CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_DevNode_Registry_PropertyW,
    CM_Get_Device_Interface_List_SizeW, CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_PropertyW,
    CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CR_BUFFER_SMALL, CR_SUCCESS,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_InstanceId, DEVPROPTYPE};
use windows::core::{GUID, HSTRING, PCWSTR};

use super::control::utf16_string;

/// Attempts to read a list that grows between its size query and its read.
const LIST_ATTEMPTS: usize = 4;

/// Symbolic link paths of the present device interfaces of `class`.
pub(crate) fn interface_paths(class: &GUID) -> Vec<String> {
    for _ in 0..LIST_ATTEMPTS {
        let mut length = 0u32;
        // SAFETY: `length` is a valid out-pointer and `class` a valid GUID; no device filters the list.
        let status = unsafe {
            CM_Get_Device_Interface_List_SizeW(&mut length, class, PCWSTR::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
        };
        if status != CR_SUCCESS {
            return Vec::new();
        }

        let mut buffer = vec![0u16; usize::try_from(length).unwrap_or(0)];
        // SAFETY: the buffer is valid for writes of its whole length.
        let status = unsafe {
            CM_Get_Device_Interface_ListW(class, PCWSTR::null(), &mut buffer, CM_GET_DEVICE_INTERFACE_LIST_PRESENT)
        };
        match status {
            CR_SUCCESS => return multi_sz(&buffer),
            CR_BUFFER_SMALL => {}
            _ => return Vec::new(),
        }
    }
    Vec::new()
}

/// The device instance ID behind a device interface.
pub(crate) fn interface_instance_id(path: &str) -> Option<String> {
    let path = HSTRING::from(path);
    let mut property_type = DEVPROPTYPE::default();
    let mut size = 0u32;
    // SAFETY: the out-pointers are valid; without a buffer, only the required size is returned.
    let status = unsafe {
        CM_Get_Device_Interface_PropertyW(
            &path,
            &DEVPKEY_Device_InstanceId,
            &mut property_type,
            None,
            &mut size,
            0,
        )
    };
    if status != CR_BUFFER_SMALL {
        return None;
    }

    let mut buffer = vec![0u8; usize::try_from(size).ok()?];
    // SAFETY: the buffer is valid for writes of `size` bytes.
    let status = unsafe {
        CM_Get_Device_Interface_PropertyW(
            &path,
            &DEVPKEY_Device_InstanceId,
            &mut property_type,
            Some(buffer.as_mut_ptr()),
            &mut size,
            0,
        )
    };
    (status == CR_SUCCESS).then(|| utf16_string(&buffer))
}

/// The device node of a device instance.
pub(crate) fn locate(instance_id: &str) -> Option<u32> {
    let instance_id = HSTRING::from(instance_id);
    let mut devinst = 0u32;
    // SAFETY: `devinst` is a valid out-pointer and the ID a NUL-terminated wide string.
    let status = unsafe { CM_Locate_DevNodeW(&mut devinst, &instance_id, CM_LOCATE_DEVNODE_NORMAL) };
    (status == CR_SUCCESS).then_some(devinst)
}

/// The name of the physical device object of a device node, such as `\Device\USBPDO-3`.
pub(crate) fn pdo_name(devinst: u32) -> Option<String> {
    let mut length = 0u32;
    // SAFETY: `length` is a valid out-pointer; without a buffer, only the required size is returned.
    let status = unsafe {
        CM_Get_DevNode_Registry_PropertyW(devinst, CM_DRP_PHYSICAL_DEVICE_OBJECT_NAME, None, None, &mut length, 0)
    };
    if status != CR_BUFFER_SMALL {
        return None;
    }

    let mut buffer = vec![0u8; usize::try_from(length).ok()?];
    // SAFETY: the buffer is valid for writes of `length` bytes.
    let status = unsafe {
        CM_Get_DevNode_Registry_PropertyW(
            devinst,
            CM_DRP_PHYSICAL_DEVICE_OBJECT_NAME,
            None,
            Some(buffer.as_mut_ptr().cast()),
            &mut length,
            0,
        )
    };
    (status == CR_SUCCESS).then(|| utf16_string(&buffer))
}

/// Split a NUL-separated list ending with an empty string.
fn multi_sz(buffer: &[u16]) -> Vec<String> {
    buffer
        .split(|unit| *unit == 0)
        .take_while(|item| !item.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_strings_end_at_the_empty_string() {
        let units: Vec<u16> = "ab\0c\0\0d\0".encode_utf16().collect();
        assert_eq!(multi_sz(&units), ["ab", "c"]);
        assert!(multi_sz(&[0]).is_empty());
    }

    #[test]
    fn unknown_devices_are_not_found() {
        assert_eq!(locate(r"USB\VID_0000&PID_0000\IRONRDP-NO-SUCH-DEVICE"), None);
    }
}
