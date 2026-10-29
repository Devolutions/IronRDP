//! PNP Device Info Subprotocol messages carried on the `PNPDR` channel ([MS-RDPEPNP] section 2.2.1).
//!
//! [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/

use alloc::string::String;
use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_fixed_part_size,
    ensure_size, invalid_field_err, unsupported_value_err,
};
use ironrdp_dvc::DvcEncode;

use super::{Guid, read_utf16_units, string_from_utf16, utf16_size, write_utf16};

/// `IRPDR_ID_VERSION`: Client or Server Version message.
pub const IRPDR_ID_VERSION: u32 = 0x0000_0065;
/// `IRPDR_ID_REDIRECT_DEVICES`: Client Device Addition message.
pub const IRPDR_ID_REDIRECT_DEVICES: u32 = 0x0000_0066;
/// `IRPDR_ID_SERVER_LOGON`: Authenticated Client message.
pub const IRPDR_ID_SERVER_LOGON: u32 = 0x0000_0067;
/// `IRPDR_ID_UNREDIRECT_DEVICE`: Client Device Removal message.
pub const IRPDR_ID_UNREDIRECT_DEVICE: u32 = 0x0000_0068;

/// Version message capability: the sender supports dynamic addition of devices.
pub const CAPABILITY_DYNAMIC_DEVICE_ADDITION: u32 = 0x0000_0001;

/// `CustomFlag` value: the server may choose not to redirect the device.
pub const CUSTOM_FLAG_OPTIONAL: u32 = 0x0000_0001;
/// `CustomFlag` value: the server must redirect the device.
pub const CUSTOM_FLAG_REDIRECTABLE: u32 = 0x0000_0002;

/// `PNP_DEVCAPS_LOCKSUPPORTED`: the device supports locking.
pub const PNP_DEVCAPS_LOCKSUPPORTED: u32 = 0x0000_0001;
/// `PNP_DEVCAPS_EJECTSUPPORTED`: the device supports ejecting.
pub const PNP_DEVCAPS_EJECTSUPPORTED: u32 = 0x0000_0002;
/// `PNP_DEVCAPS_REMOVABLE`: the device is removable.
pub const PNP_DEVCAPS_REMOVABLE: u32 = 0x0000_0004;
/// `PNP_DEVCAPS_SURPRISEREMOVALOK`: the device can be removed unexpectedly.
pub const PNP_DEVCAPS_SURPRISEREMOVALOK: u32 = 0x0000_0008;

const HEADER_SIZE: usize = 4 /* Size */ + 4 /* PacketId */;

/// A PNP Device Info Subprotocol message with its `PNP_INFO_HEADER` ([MS-RDPEPNP] section 2.2.1.1).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PnpInfoPdu {
    /// Server Version or Client Version message (sections 2.2.1.2.1 and 2.2.1.2.2).
    Version(VersionPdu),
    /// Client Device Addition message (section 2.2.1.3.1).
    RedirectDevices(DeviceAdditionPdu),
    /// Authenticated Client message (section 2.2.1.2.3).
    ServerLogon,
    /// Client Device Removal message (section 2.2.1.3.2).
    UnredirectDevice(DeviceRemovalPdu),
}

impl PnpInfoPdu {
    const NAME: &'static str = "PNP_INFO_PDU";

    fn packet_id(&self) -> u32 {
        match self {
            Self::Version(_) => IRPDR_ID_VERSION,
            Self::RedirectDevices(_) => IRPDR_ID_REDIRECT_DEVICES,
            Self::ServerLogon => IRPDR_ID_SERVER_LOGON,
            Self::UnredirectDevice(_) => IRPDR_ID_UNREDIRECT_DEVICE,
        }
    }

    fn payload_size(&self) -> usize {
        match self {
            Self::Version(pdu) => pdu.size(),
            Self::RedirectDevices(pdu) => pdu.size(),
            Self::ServerLogon => 0,
            Self::UnredirectDevice(pdu) => pdu.size(),
        }
    }
}

impl Encode for PnpInfoPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let size = self.size();
        ensure_size!(in: dst, size: size);
        dst.write_u32(cast_length!("Size", size)?);
        dst.write_u32(self.packet_id());
        match self {
            Self::Version(pdu) => pdu.encode(dst),
            Self::RedirectDevices(pdu) => pdu.encode(dst),
            Self::ServerLogon => Ok(()),
            Self::UnredirectDevice(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        HEADER_SIZE + self.payload_size()
    }
}

impl<'de> Decode<'de> for PnpInfoPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: HEADER_SIZE);
        let size = usize::try_from(src.read_u32()).map_err(|_| invalid_field_err!("Size", "size overflows usize"))?;
        let packet_id = src.read_u32();
        let Some(payload_size) = size.checked_sub(HEADER_SIZE) else {
            return Err(invalid_field_err!("Size", "size is smaller than the header"));
        };
        ensure_size!(in: src, size: payload_size);
        let mut payload = ReadCursor::new(src.read_slice(payload_size));

        match packet_id {
            IRPDR_ID_VERSION => Ok(Self::Version(VersionPdu::decode(&mut payload)?)),
            IRPDR_ID_REDIRECT_DEVICES => Ok(Self::RedirectDevices(DeviceAdditionPdu::decode(&mut payload)?)),
            IRPDR_ID_SERVER_LOGON => Ok(Self::ServerLogon),
            IRPDR_ID_UNREDIRECT_DEVICE => Ok(Self::UnredirectDevice(DeviceRemovalPdu::decode(&mut payload)?)),
            _ => Err(unsupported_value_err!("PacketId", alloc::format!("{packet_id:#010x}"))),
        }
    }
}

impl DvcEncode for PnpInfoPdu {}

/// Server Version and Client Version message payload ([MS-RDPEPNP] sections 2.2.1.2.1 and 2.2.1.2.2).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionPdu {
    pub major_version: u32,
    pub minor_version: u32,
    pub capabilities: u32,
}

impl VersionPdu {
    const NAME: &'static str = "VERSION_PDU";

    const FIXED_PART_SIZE: usize = 4 /* MajorVersion */ + 4 /* MinorVersion */ + 4 /* Capabilities */;
}

impl Encode for VersionPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.major_version);
        dst.write_u32(self.minor_version);
        dst.write_u32(self.capabilities);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for VersionPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        Ok(Self {
            major_version: src.read_u32(),
            minor_version: src.read_u32(),
            capabilities: src.read_u32(),
        })
    }
}

/// Client Device Addition message payload ([MS-RDPEPNP] section 2.2.1.3.1).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAdditionPdu {
    pub devices: Vec<DeviceDescription>,
}

impl DeviceAdditionPdu {
    const NAME: &'static str = "DEVICE_ADDITION_PDU";

    const FIXED_PART_SIZE: usize = 4 /* DeviceCount */;
}

impl Encode for DeviceAdditionPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u32(cast_length!("DeviceCount", self.devices.len())?);
        for device in &self.devices {
            device.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.devices.iter().map(Encode::size).sum::<usize>()
    }
}

impl<'de> Decode<'de> for DeviceAdditionPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let count = src.read_u32();
        // Descriptions are read one at a time, so a forged count cannot reserve memory.
        let mut devices = Vec::new();
        for _ in 0..count {
            devices.push(DeviceDescription::decode(src)?);
        }
        Ok(Self { devices })
    }
}

/// Client Device Removal message payload ([MS-RDPEPNP] section 2.2.1.3.2).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceRemovalPdu {
    pub client_device_id: u32,
}

impl DeviceRemovalPdu {
    const NAME: &'static str = "DEVICE_REMOVAL_PDU";

    const FIXED_PART_SIZE: usize = 4 /* ClientDeviceID */;
}

impl Encode for DeviceRemovalPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.client_device_id);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for DeviceRemovalPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        Ok(Self {
            client_device_id: src.read_u32(),
        })
    }
}

/// `PNP_DEVICE_DESCRIPTION` ([MS-RDPEPNP] section 2.2.1.3.1.1).
///
/// The `DataSize` field and the length prefixes are derived from the field values.
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDescription {
    /// Unique client-generated ID the server uses to refer to the device.
    pub client_device_id: u32,
    /// Device interfaces exposed by the device.
    pub interface_guids: Vec<Guid>,
    /// Hardware IDs, sent as a multisz string.
    ///
    /// Entries must be non-empty and contain no NUL character.
    pub hardware_ids: Vec<String>,
    /// Compatible IDs, sent as a multisz string.
    ///
    /// Entries must be non-empty and contain no NUL character.
    pub compatibility_ids: Vec<String>,
    /// Device description, sent without a null terminator.
    pub description: String,
    /// [`CUSTOM_FLAG_REDIRECTABLE`], [`CUSTOM_FLAG_OPTIONAL`], or the redirectable value 0.
    pub custom_flag: u32,
    /// Container ID shared by the devices of one piece of physical hardware.
    pub container_id: Option<Guid>,
    /// `PNP_DEVCAPS_*` flags.
    ///
    /// The field follows `ContainerId` on the wire, so it can be sent only with a container ID.
    pub device_caps: Option<u32>,
}

impl DeviceDescription {
    const NAME: &'static str = "PNP_DEVICE_DESCRIPTION";

    const FIXED_PART_SIZE: usize = 4 /* ClientDeviceID */ + 4 /* DataSize */;

    fn data_size(&self) -> usize {
        let container = if self.container_id.is_some() {
            4 /* cbContainerId */ + Guid::FIXED_PART_SIZE
        } else {
            0
        };
        let caps = if self.device_caps.is_some() {
            4 /* cbDeviceCaps */ + 4 /* DeviceCaps */
        } else {
            0
        };
        4 /* cbInterfaceLength */
            + self.interface_guids.len() * Guid::FIXED_PART_SIZE
            + 4 /* cbHardwareIdLength */
            + multi_sz_size(&self.hardware_ids)
            + 4 /* cbCompatIdLength */
            + multi_sz_size(&self.compatibility_ids)
            + 4 /* cbDeviceDescriptionLength */
            + utf16_size(&self.description)
            + 4 /* CustomFlagLength */
            + 4 /* CustomFlag */
            + container
            + caps
    }
}

impl Encode for DeviceDescription {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if self.device_caps.is_some() && self.container_id.is_none() {
            return Err(invalid_field_err!(
                "DeviceCaps",
                "device capabilities require a container ID"
            ));
        }
        for id in self.hardware_ids.iter().chain(&self.compatibility_ids) {
            if id.is_empty() || id.contains('\0') {
                return Err(invalid_field_err!(
                    "HardwareId",
                    "multisz entries must be non-empty and NUL-free"
                ));
            }
        }
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.client_device_id);
        dst.write_u32(cast_length!("DataSize", self.data_size())?);
        dst.write_u32(cast_length!(
            "cbInterfaceLength",
            self.interface_guids.len() * Guid::FIXED_PART_SIZE
        )?);
        for guid in &self.interface_guids {
            guid.encode(dst)?;
        }
        dst.write_u32(cast_length!("cbHardwareIdLength", multi_sz_size(&self.hardware_ids))?);
        write_multi_sz(dst, &self.hardware_ids);
        dst.write_u32(cast_length!(
            "cbCompatIdLength",
            multi_sz_size(&self.compatibility_ids)
        )?);
        write_multi_sz(dst, &self.compatibility_ids);
        dst.write_u32(cast_length!(
            "cbDeviceDescriptionLength",
            utf16_size(&self.description)
        )?);
        write_utf16(dst, &self.description);
        dst.write_u32(4); // CustomFlagLength
        dst.write_u32(self.custom_flag);
        if let Some(container_id) = self.container_id {
            dst.write_u32(16); // cbContainerId
            container_id.encode(dst)?;
        }
        if let Some(device_caps) = self.device_caps {
            dst.write_u32(4); // cbDeviceCaps
            dst.write_u32(device_caps);
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.data_size()
    }
}

impl<'de> Decode<'de> for DeviceDescription {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let client_device_id = src.read_u32();
        let data_size =
            usize::try_from(src.read_u32()).map_err(|_| invalid_field_err!("DataSize", "size overflows usize"))?;
        ensure_size!(in: src, size: data_size);
        let mut data = ReadCursor::new(src.read_slice(data_size));

        let interfaces = read_length_prefixed(&mut data, "cbInterfaceLength")?;
        if !interfaces.len().is_multiple_of(Guid::FIXED_PART_SIZE) {
            return Err(invalid_field_err!(
                "cbInterfaceLength",
                "interface array length is not a multiple of the GUID size"
            ));
        }
        let interface_guids = interfaces
            .chunks_exact(Guid::FIXED_PART_SIZE)
            .map(|guid| Guid::decode(&mut ReadCursor::new(guid)))
            .collect::<DecodeResult<Vec<_>>>()?;
        let hardware_ids = read_multi_sz("HardwareId", read_length_prefixed(&mut data, "cbHardwareIdLength")?)?;
        let compatibility_ids = read_multi_sz("CompatibilityID", read_length_prefixed(&mut data, "cbCompatIdLength")?)?;
        let description_units = read_utf16_units(
            "DeviceDescription",
            read_length_prefixed(&mut data, "cbDeviceDescriptionLength")?,
        )?;
        let description = string_from_utf16("DeviceDescription", &description_units)?;

        ensure_size!(in: data, size: 4 /* CustomFlagLength */ + 4 /* CustomFlag */);
        if data.read_u32() != 4 {
            return Err(invalid_field_err!("CustomFlagLength", "must be 4"));
        }
        let custom_flag = data.read_u32();

        // ContainerId and DeviceCaps are optional trailing fields, present when DataSize covers them.
        let container_id = if 4 /* cbContainerId */ + Guid::FIXED_PART_SIZE <= data.len() {
            if data.read_u32() != 16 {
                return Err(invalid_field_err!("cbContainerId", "must be 16"));
            }
            Some(Guid::decode(&mut data)?)
        } else {
            None
        };
        let device_caps = if container_id.is_some() && 4 /* cbDeviceCaps */ + 4 /* DeviceCaps */ <= data.len() {
            if data.read_u32() != 4 {
                return Err(invalid_field_err!("cbDeviceCaps", "must be 4"));
            }
            Some(data.read_u32())
        } else {
            None
        };

        Ok(Self {
            client_device_id,
            interface_guids,
            hardware_ids,
            compatibility_ids,
            description,
            custom_flag,
            container_id,
            device_caps,
        })
    }
}

fn read_length_prefixed<'de>(src: &mut ReadCursor<'de>, field: &'static str) -> DecodeResult<&'de [u8]> {
    ensure_size!(in: src, size: 4);
    let length = usize::try_from(src.read_u32()).map_err(|_| invalid_field_err!(field, "length overflows usize"))?;
    ensure_size!(in: src, size: length);
    Ok(src.read_slice(length))
}

fn multi_sz_size(entries: &[String]) -> usize {
    if entries.is_empty() {
        0
    } else {
        entries
            .iter()
            .map(|entry| utf16_size(entry) + 2 /* NUL */)
            .sum::<usize>()
            + 2 /* final NUL */
    }
}

fn write_multi_sz(dst: &mut WriteCursor<'_>, entries: &[String]) {
    if entries.is_empty() {
        return;
    }
    for entry in entries {
        write_utf16(dst, entry);
        dst.write_u16(0);
    }
    dst.write_u16(0);
}

/// Splits a multisz string, accepting an empty field or a lone NUL as an empty list.
fn read_multi_sz(field: &'static str, bytes: &[u8]) -> DecodeResult<Vec<String>> {
    let units = read_utf16_units(field, bytes)?;
    if units.is_empty() || units == [0] {
        return Ok(Vec::new());
    }
    let Some(strings) = units.strip_suffix(&[0, 0]) else {
        return Err(invalid_field_err!(field, "multisz string is not double-NUL terminated"));
    };
    strings
        .split(|&unit| unit == 0)
        .map(|entry| {
            if entry.is_empty() {
                Err(invalid_field_err!(field, "multisz string has an empty entry"))
            } else {
                string_from_utf16(field, entry)
            }
        })
        .collect()
}
