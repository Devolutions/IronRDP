//! The control object of the RemoteFX USB hub filter (TsUsbFlt).
//!
//! A process claims local devices through this object. The filter then has every claimed device
//! re-enumerated with the generic redirection driver (TsUsbGD) in place of its function driver, and
//! only lets the claiming process open it. Closing the control handle releases the claims.

use windows::core::{Error, GUID};

use super::io::DeviceHandle;

const CONTROL_PATH: &str = r"\\.\TerminalServicesUSBFilterDriverControlObject";

/// Claim a list of devices by proprietary ID, replacing the previous claim of this handle.
const IOCTL_CLAIM_CHILD_DEVICES: u32 = 0x0022_4404;
/// Query the proprietary ID of a device, by the PDO names of its hub and of itself.
const IOCTL_QUERY_CHILD_DEVICE_PROPRIETARY_ID: u32 = 0x0022_4408;
/// Query the GUID assigned to this handle: the interface class of the devices it claims, and the
/// file name under which they must be opened.
const IOCTL_QUERY_INTERFACE_GUID: u32 = 0x0022_440c;

/// Device interface class registered by the filter on every hub it filters.
pub(crate) const GUID_DEVINTERFACE_TSUSBBUS_FILTER: GUID = GUID::from_u128(0xed8cf6b1_62d5_4597_bcaa_942b18988098);

/// An open handle to the filter control object, holding this process's claim.
pub(crate) struct FilterControl {
    handle: DeviceHandle,
}

impl FilterControl {
    /// Open the control object.
    ///
    /// This fails when the RemoteFX USB redirection policy does not allow the caller, or when the
    /// filter is not installed.
    pub(crate) fn open() -> Result<Self, Error> {
        DeviceHandle::open_read(CONTROL_PATH).map(|handle| Self { handle })
    }

    pub(crate) fn interface_guid(&self) -> Result<GUID, Error> {
        let mut output = [0u8; 16];
        self.handle.control(IOCTL_QUERY_INTERFACE_GUID, &[], &mut output)?;
        Ok(GUID::from_u128(u128::from_be_bytes(guid_to_be(output))))
    }

    /// The proprietary ID of a device plugged into a filtered hub.
    pub(crate) fn proprietary_id(&self, hub_pdo: &str, device_pdo: &str) -> Result<String, Error> {
        let output = self.handle.control_vec(
            IOCTL_QUERY_CHILD_DEVICE_PROPRIETARY_ID,
            &name_pair_input(hub_pdo, device_pdo),
        )?;
        Ok(utf16_string(&output))
    }

    /// Claim exactly `ids`, releasing every device claimed before and not listed.
    pub(crate) fn claim(&self, ids: &[String]) -> Result<(), Error> {
        self.handle
            .control(IOCTL_CLAIM_CHILD_DEVICES, &claim_input(ids), &mut [])
            .map(|_| ())
    }
}

/// Reorder a little-endian `GUID` structure into the big-endian `u128` form of its text.
fn guid_to_be(bytes: [u8; 16]) -> [u8; 16] {
    let [a0, a1, a2, a3, b0, b1, c0, c1, rest @ ..] = bytes;
    let mut be = [0u8; 16];
    let (head, tail) = be.split_at_mut(8);
    head.copy_from_slice(&[a3, a2, a1, a0, b1, b0, c1, c0]);
    tail.copy_from_slice(&rest);
    be
}

/// Format a GUID the way the driver expects in a file name: lowercase, with braces.
pub(crate) fn braced_guid(guid: &GUID) -> String {
    let value = guid.to_u128();
    format!(
        "{{{:08x}-{:04x}-{:04x}-{:04x}-{:012x}}}",
        value >> 96,
        (value >> 80) & 0xffff,
        (value >> 64) & 0xffff,
        (value >> 48) & 0xffff,
        value & 0xffff_ffff_ffff
    )
}

/// `{u32 hub offset, u32 device offset}` followed by both NUL-terminated names.
fn name_pair_input(hub_pdo: &str, device_pdo: &str) -> Vec<u8> {
    let hub = utf16z_bytes(hub_pdo);
    let device = utf16z_bytes(device_pdo);
    let mut input = Vec::with_capacity(8 + hub.len() + device.len());
    input.extend_from_slice(&0u32.to_le_bytes());
    input.extend_from_slice(&u32::try_from(hub.len()).unwrap_or(u32::MAX).to_le_bytes());
    input.extend_from_slice(&hub);
    input.extend_from_slice(&device);
    input
}

/// Three list offsets, of which only the third, the proprietary IDs, is used, then that list as a
/// multi-string. An empty input releases every device.
fn claim_input(ids: &[String]) -> Vec<u8> {
    if ids.is_empty() {
        return Vec::new();
    }

    let mut input = Vec::new();
    input.extend_from_slice(&u32::MAX.to_le_bytes());
    input.extend_from_slice(&u32::MAX.to_le_bytes());
    input.extend_from_slice(&0u32.to_le_bytes());
    for id in ids {
        input.extend_from_slice(&utf16z_bytes(id));
    }
    input.extend_from_slice(&[0, 0]);
    input
}

fn utf16z_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(core::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// Decode little-endian UTF-16 up to the first NUL.
pub(crate) fn utf16_string(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|unit| u16::from_le_bytes([unit.first().copied().unwrap_or(0), unit.last().copied().unwrap_or(0)]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_pairs_are_offset_from_the_names() {
        let input = name_pair_input("A", "BC");
        assert_eq!(input, [0, 0, 0, 0, 4, 0, 0, 0, b'A', 0, 0, 0, b'B', 0, b'C', 0, 0, 0]);
    }

    #[test]
    fn claims_list_proprietary_ids_only() {
        assert!(claim_input(&[]).is_empty(), "an empty claim releases every device");

        let input = claim_input(&["X".to_owned(), "YZ".to_owned()]);
        assert_eq!(
            input,
            [
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, b'X', 0, 0, 0, b'Y', 0, b'Z', 0, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn guids_round_trip_through_their_structure() {
        // {75b36284-95b7-11dd-a38b-000000000000}, the first GUID the filter hands out.
        let structure = [
            0x84, 0x62, 0xb3, 0x75, 0xb7, 0x95, 0xdd, 0x11, 0xa3, 0x8b, 0, 0, 0, 0, 0, 0,
        ];
        let guid = GUID::from_u128(u128::from_be_bytes(guid_to_be(structure)));
        assert_eq!(braced_guid(&guid), "{75b36284-95b7-11dd-a38b-000000000000}");
    }

    #[test]
    fn wide_strings_stop_at_nul() {
        assert_eq!(utf16_string(&[b'H', 0, b'i', 0, 0, 0, b'x', 0]), "Hi");
        assert_eq!(utf16_string(&[b'H', 0, b'i']), "H");
    }
}
