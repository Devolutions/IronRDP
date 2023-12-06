//! PDUs for [\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension]
//!
//! [\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5

use core::fmt;
use std::fmt::{Debug, Display};
use std::mem::size_of;

use bitflags::bitflags;
use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::utils::{decode_string, encoded_str_len, from_utf16_bytes, write_string_to_cursor, CharacterSet};
use ironrdp_pdu::{
    cast_length, ensure_fixed_part_size, ensure_size, invalid_message_err, read_padding, unsupported_pdu_err,
    write_padding, PduError, PduResult,
};

use super::esc::rpce;
use super::{PacketId, SharedHeader};

#[derive(Debug, PartialEq)]
pub enum VersionAndIdPduKind {
    /// [2.2.2.2] Server Announce Request (DR_CORE_SERVER_ANNOUNCE_REQ)
    ///
    /// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/046047aa-62d8-49f9-bf16-7fe41880aaf4
    ServerAnnounceRequest,
    /// [2.2.2.3] Client Announce Reply (DR_CORE_CLIENT_ANNOUNCE_RSP)
    ///
    /// [2.2.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/d6fe6d1b-c145-4a6f-99aa-4fe3cdcea398
    ClientAnnounceReply,
    /// [2.2.2.6] Server Client ID Confirm (DR_CORE_SERVER_CLIENTID_CONFIRM)
    ///
    /// [2.2.2.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/bbbb9666-6994-4cf6-8e65-0d46eb319c6e
    ServerClientIdConfirm,
}

impl VersionAndIdPduKind {
    fn name(&self) -> &'static str {
        match self {
            VersionAndIdPduKind::ServerAnnounceRequest => "DR_CORE_SERVER_ANNOUNCE_REQ",
            VersionAndIdPduKind::ClientAnnounceReply => "DR_CORE_CLIENT_ANNOUNCE_RSP",
            VersionAndIdPduKind::ServerClientIdConfirm => "DR_CORE_SERVER_CLIENTID_CONFIRM",
        }
    }
}

/// VersionAndIdPDU is a fixed size structure representing multiple PDUs.
///
/// The kind field is used to determine the actual PDU type, see [`VersionAndIdPduKind`].
#[derive(Debug)]
pub struct VersionAndIdPdu {
    /// This field MUST be set to 0x0001 ([`VERSION_MAJOR`]).
    pub version_major: u16,
    pub version_minor: u16,
    pub client_id: u32,
    pub kind: VersionAndIdPduKind,
}

impl VersionAndIdPdu {
    const FIXED_PART_SIZE: usize = (size_of::<u16>() * 2) + size_of::<u32>();

    pub fn new_client_announce_reply(req: VersionAndIdPdu) -> PduResult<Self> {
        if req.kind != VersionAndIdPduKind::ServerAnnounceRequest {
            return Err(invalid_message_err!(
                "VersionAndIdPdu::new_client_announce_reply",
                "VersionAndIdPduKind",
                "invalid value"
            ));
        }

        Ok(Self {
            version_major: VERSION_MAJOR,
            version_minor: VERSION_MINOR_12,
            client_id: req.client_id,
            kind: VersionAndIdPduKind::ClientAnnounceReply,
        })
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(ctx: self.name(), in: dst, size: Self::FIXED_PART_SIZE);
        dst.write_u16(self.version_major);
        dst.write_u16(self.version_minor);
        dst.write_u32(self.client_id);
        Ok(())
    }

    pub fn decode(header: SharedHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let kind = match header.packet_id {
            PacketId::CoreServerAnnounce => VersionAndIdPduKind::ServerAnnounceRequest,
            PacketId::CoreClientidConfirm => VersionAndIdPduKind::ServerClientIdConfirm,
            _ => {
                return Err(invalid_message_err!(
                    "VersionAndIdPdu::decode",
                    "PacketId",
                    "invalid value"
                ))
            }
        };

        ensure_size!(ctx: kind.name(), in: src, size: Self::FIXED_PART_SIZE);
        let version_major = src.read_u16();
        let version_minor = src.read_u16();
        let client_id = src.read_u32();

        Ok(Self {
            version_major,
            version_minor,
            client_id,
            kind,
        })
    }

    pub fn name(&self) -> &'static str {
        self.kind.name()
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [2.2.2.4] Client Name Request (DR_CORE_CLIENT_NAME_REQ)
///
/// [2.2.2.4]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/902497f1-3b1c-4aee-95f8-1668f9b7b7d2
#[derive(Debug)]
pub enum ClientNameRequest {
    Ascii(String),
    Unicode(String),
}

impl ClientNameRequest {
    const NAME: &'static str = "DR_CORE_CLIENT_NAME_REQ";
    const FIXED_PART_SIZE: usize = size_of::<u32>() * 3; // unicode_flag + CodePage + ComputerNameLen

    pub fn new(computer_name: String, kind: ClientNameRequestUnicodeFlag) -> Self {
        match kind {
            ClientNameRequestUnicodeFlag::Ascii => ClientNameRequest::Ascii(computer_name),
            ClientNameRequestUnicodeFlag::Unicode => ClientNameRequest::Unicode(computer_name),
        }
    }

    fn unicode_flag(&self) -> ClientNameRequestUnicodeFlag {
        match self {
            ClientNameRequest::Ascii(_) => ClientNameRequestUnicodeFlag::Ascii,
            ClientNameRequest::Unicode(_) => ClientNameRequestUnicodeFlag::Unicode,
        }
    }

    fn computer_name(&self) -> &str {
        match self {
            ClientNameRequest::Ascii(name) => name,
            ClientNameRequest::Unicode(name) => name,
        }
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.unicode_flag().into());
        dst.write_u32(0); // // CodePage (4 bytes): it MUST be set to 0
        dst.write_u32(encoded_str_len(self.computer_name(), self.unicode_flag().into(), true) as u32);
        write_string_to_cursor(dst, self.computer_name(), self.unicode_flag().into(), true)
    }

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + encoded_str_len(self.computer_name(), self.unicode_flag().into(), true)
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum ClientNameRequestUnicodeFlag {
    Ascii = 0x0,
    Unicode = 0x1,
}

impl From<ClientNameRequestUnicodeFlag> for CharacterSet {
    fn from(val: ClientNameRequestUnicodeFlag) -> Self {
        match val {
            ClientNameRequestUnicodeFlag::Ascii => CharacterSet::Ansi,
            ClientNameRequestUnicodeFlag::Unicode => CharacterSet::Unicode,
        }
    }
}

impl From<ClientNameRequestUnicodeFlag> for u32 {
    fn from(val: ClientNameRequestUnicodeFlag) -> Self {
        val as u32
    }
}

/// [2.2.2.7] Server Core Capability Request (DR_CORE_CAPABILITY_REQ)
/// and [2.2.2.8] Client Core Capability Response (DR_CORE_CAPABILITY_RSP)
///
/// [2.2.2.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/702789c3-b924-4bc2-9280-3221bc7d6797
/// [2.2.2.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/f513bf87-cca0-488a-ac5c-18cf18f4a7e1
#[derive(Debug)]
pub struct CoreCapability {
    pub capabilities: Vec<CapabilityMessage>,
    pub kind: CoreCapabilityKind,
}

impl CoreCapability {
    const FIXED_PART_SIZE: usize = size_of::<u16>() * 2;

    /// Creates a new [`DR_CORE_CAPABILITY_RSP`] with the given `capabilities`.
    ///
    /// [`DR_CORE_CAPABILITY_RSP`]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/f513bf87-cca0-488a-ac5c-18cf18f4a7e1
    pub fn new_response(capabilities: Vec<CapabilityMessage>) -> Self {
        Self {
            capabilities,
            kind: CoreCapabilityKind::ClientCoreCapabilityResponse,
        }
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(ctx: self.name(), in: dst, size: self.size());
        dst.write_u16(cast_length!(
            "CoreCapability",
            "numCapabilities",
            self.capabilities.len()
        )?);
        write_padding!(dst, 2); // 2-bytes padding
        for cap in self.capabilities.iter() {
            cap.encode(dst)?;
        }
        Ok(())
    }

    pub fn decode(header: SharedHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let kind = match header.packet_id {
            PacketId::CoreServerCapability => CoreCapabilityKind::ServerCoreCapabilityRequest,
            PacketId::CoreClientCapability => CoreCapabilityKind::ClientCoreCapabilityResponse,
            _ => {
                return Err(invalid_message_err!(
                    "CoreCapability::decode",
                    "PacketId",
                    "invalid value"
                ))
            }
        };

        ensure_size!(ctx: kind.name(), in: src, size: Self::FIXED_PART_SIZE);

        let num_capabilities = src.read_u16();
        read_padding!(src, 2); // 2-bytes padding
        let mut capabilities = Vec::new();
        for _ in 0..num_capabilities {
            capabilities.push(CapabilityMessage::decode(src)?);
        }

        Ok(Self { capabilities, kind })
    }

    pub fn name(&self) -> &'static str {
        self.kind.name()
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.capabilities.iter().map(|c| c.size()).sum::<usize>()
    }
}

#[derive(Debug, PartialEq)]
pub enum CoreCapabilityKind {
    /// [2.2.2.7] Server Core Capability Request (DR_CORE_CAPABILITY_REQ)
    ///
    /// [2.2.2.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/702789c3-b924-4bc2-9280-3221bc7d6797
    ServerCoreCapabilityRequest,
    /// [2.2.2.8] Client Core Capability Response (DR_CORE_CAPABILITY_RSP)
    ///
    /// [2.2.2.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/f513bf87-cca0-488a-ac5c-18cf18f4a7e1
    ClientCoreCapabilityResponse,
}

impl CoreCapabilityKind {
    fn name(&self) -> &'static str {
        match self {
            CoreCapabilityKind::ServerCoreCapabilityRequest => "DR_CORE_CAPABILITY_REQ",
            CoreCapabilityKind::ClientCoreCapabilityResponse => "DR_CORE_CAPABILITY_RSP",
        }
    }
}

#[derive(Debug)]
pub struct Capabilities(Vec<CapabilityMessage>);

impl Capabilities {
    pub fn new() -> Self {
        let mut this = Self(Vec::new());
        this.add_general(0);
        this
    }

    pub fn clone_inner(&mut self) -> Vec<CapabilityMessage> {
        self.0.clone()
    }

    pub fn add_smartcard(&mut self) {
        self.push(CapabilityMessage::new_smartcard());
        self.increment_special_devices();
    }

    pub fn add_drive(&mut self) {
        self.push(CapabilityMessage::new_drive());
    }

    fn add_general(&mut self, special_type_device_cap: u32) {
        self.push(CapabilityMessage::new_general(special_type_device_cap));
    }

    fn push(&mut self, capability: CapabilityMessage) {
        self.0.push(capability);
    }

    fn increment_special_devices(&mut self) {
        let capabilities = &mut self.0;
        for capability in capabilities.iter_mut() {
            match &mut capability.capability_data {
                CapabilityData::General(general_capability) => {
                    general_capability.special_type_device_cap += 1;
                    break;
                }
                _ => continue,
            }
        }
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::new()
    }
}

/// [2.2.1.2.1] Capability Message (CAPABILITY_SET)
///
/// [2.2.1.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/f1b9dd1d-2c37-4aac-9836-4b0df02369ba
#[derive(Debug, Clone, Copy)]
pub struct CapabilityMessage {
    header: CapabilityHeader,
    capability_data: CapabilityData,
}

impl CapabilityMessage {
    /// Creates a new [`GENERAL_CAPS_SET`].
    ///
    /// `special_type_device_cap`: A 32-bit unsigned integer that
    /// specifies the number of special devices to be redirected
    /// before the user is logged on. Special devices are those
    /// that are safe and/or required to be redirected before a
    /// user logs on (such as smart cards and serial ports).
    ///
    /// [`GENERAL_CAPS_SET`]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/06c7cb30-303d-4fa2-b396-806df8ac1501
    pub fn new_general(special_type_device_cap: u32) -> Self {
        Self {
            header: CapabilityHeader::new_general(),
            capability_data: CapabilityData::General(GeneralCapabilitySet {
                os_type: 0,
                os_version: 0,
                protocol_major_version: 1,
                protocol_minor_version: VERSION_MINOR_12,
                io_code_1: IoCode1::REQUIRED,
                io_code_2: 0,
                extended_pdu: ExtendedPdu::RDPDR_DEVICE_REMOVE_PDUS | ExtendedPdu::RDPDR_CLIENT_DISPLAY_NAME_PDU,
                extra_flags_1: ExtraFlags1::empty(),
                extra_flags_2: 0,
                special_type_device_cap,
            }),
        }
    }

    /// Creates a new [`SMARTCARD_CAPS_SET`].
    ///
    /// [`SMARTCARD_CAPS_SET`]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/e02de60a-4d32-4dc7-ab17-9d591129eb93
    pub fn new_smartcard() -> Self {
        Self {
            header: CapabilityHeader::new_smartcard(),
            capability_data: CapabilityData::Smartcard,
        }
    }

    /// Creates a new [`DRIVE_CAPS_SET`].
    ///
    /// [`DRIVE_CAPS_SET`]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/4f018cd2-60ba-4c7b-adcf-55bd05cea6f8
    pub fn new_drive() -> Self {
        Self {
            header: CapabilityHeader::new_drive(),
            capability_data: CapabilityData::Drive,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
        self.capability_data.encode(dst)
    }

    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let header = CapabilityHeader::decode(src)?;
        let capability_data = CapabilityData::decode(src, &header)?;

        Ok(Self {
            header,
            capability_data,
        })
    }

    fn size(&self) -> usize {
        CapabilityHeader::SIZE + self.capability_data.size()
    }
}

/// [2.2.1.2] Capability Header (CAPABILITY_HEADER)
///
/// [2.2.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/b3c3304a-2e1b-4667-97e9-3bce49544907
#[derive(Debug, Clone, Copy)]
struct CapabilityHeader {
    cap_type: CapabilityType,
    length: u16,
    version: u32,
}

impl CapabilityHeader {
    const SIZE: usize = size_of::<u16>() * 2 + size_of::<u32>();

    fn new_general() -> Self {
        Self {
            cap_type: CapabilityType::General,
            length: (Self::SIZE + GeneralCapabilitySet::SIZE) as u16,
            version: GENERAL_CAPABILITY_VERSION_02,
        }
    }

    fn new_smartcard() -> Self {
        Self {
            cap_type: CapabilityType::Smartcard,
            length: Self::SIZE as u16,
            version: SMARTCARD_CAPABILITY_VERSION_01,
        }
    }

    fn new_drive() -> Self {
        Self {
            cap_type: CapabilityType::Drive,
            length: Self::SIZE as u16,
            version: DRIVE_CAPABILITY_VERSION_02,
        }
    }

    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(in: src, size: Self::SIZE);
        let cap_type: CapabilityType = src.read_u16().try_into()?;
        let length = src.read_u16();
        let version = src.read_u32();

        Ok(Self {
            cap_type,
            length,
            version,
        })
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::SIZE);
        dst.write_u16(self.cap_type.into());
        dst.write_u16(self.length);
        dst.write_u32(self.version);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
enum CapabilityType {
    /// CAP_GENERAL_TYPE
    General = 0x0001,
    /// CAP_PRINTER_TYPE
    Printer = 0x0002,
    /// CAP_PORT_TYPE
    Port = 0x0003,
    /// CAP_DRIVE_TYPE
    Drive = 0x0004,
    /// CAP_SMARTCARD_TYPE
    Smartcard = 0x0005,
}

impl From<CapabilityType> for u16 {
    fn from(cap_type: CapabilityType) -> Self {
        cap_type as u16
    }
}

/// GENERAL_CAPABILITY_VERSION_02
pub const GENERAL_CAPABILITY_VERSION_02: u32 = 0x0000_0002;
/// SMARTCARD_CAPABILITY_VERSION_01
pub const SMARTCARD_CAPABILITY_VERSION_01: u32 = 0x0000_0001;
/// DRIVE_CAPABILITY_VERSION_02
pub const DRIVE_CAPABILITY_VERSION_02: u32 = 0x0000_0002;

impl TryFrom<u16> for CapabilityType {
    type Error = PduError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x0001 => Ok(CapabilityType::General),
            0x0002 => Ok(CapabilityType::Printer),
            0x0003 => Ok(CapabilityType::Port),
            0x0004 => Ok(CapabilityType::Drive),
            0x0005 => Ok(CapabilityType::Smartcard),
            _ => Err(invalid_message_err!("try_from", "CapabilityType", "invalid value")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum CapabilityData {
    General(GeneralCapabilitySet),
    Printer,
    Port,
    Drive,
    Smartcard,
}

impl CapabilityData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            CapabilityData::General(general) => general.encode(dst),
            _ => Ok(()),
        }
    }

    fn decode(src: &mut ReadCursor<'_>, header: &CapabilityHeader) -> PduResult<Self> {
        match header.cap_type {
            CapabilityType::General => Ok(CapabilityData::General(GeneralCapabilitySet::decode(
                src,
                header.version,
            )?)),
            CapabilityType::Printer => Ok(CapabilityData::Printer),
            CapabilityType::Port => Ok(CapabilityData::Port),
            CapabilityType::Drive => Ok(CapabilityData::Drive),
            CapabilityType::Smartcard => Ok(CapabilityData::Smartcard),
        }
    }

    fn size(&self) -> usize {
        match self {
            CapabilityData::General(_) => GeneralCapabilitySet::SIZE,
            CapabilityData::Printer => 0,
            CapabilityData::Port => 0,
            CapabilityData::Drive => 0,
            CapabilityData::Smartcard => 0,
        }
    }
}

/// [2.2.2.7.1] General Capability Set (GENERAL_CAPS_SET)
///
/// [2.2.2.7.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/06c7cb30-303d-4fa2-b396-806df8ac1501
#[derive(Debug, Clone, Copy)]
struct GeneralCapabilitySet {
    /// MUST be ignored.
    os_type: u32,
    /// SHOULD be ignored.
    os_version: u32,
    /// MUST be set to 1.
    protocol_major_version: u16,
    /// MUST be set to one of the values described by the VersionMinor field
    /// of the [Server Client ID Confirm (section 2.2.2.6)] packet.
    ///
    /// [Server Client ID Confirm (section 2.2.2.6)]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/bbbb9666-6994-4cf6-8e65-0d46eb319c6e
    protocol_minor_version: u16,
    /// See [`IoCode1`].
    io_code_1: IoCode1,
    /// MUST be set to 0.
    io_code_2: u32,
    /// See [`ExtendedPdu`].
    extended_pdu: ExtendedPdu,
    /// See [`ExtraFlags1`].
    extra_flags_1: ExtraFlags1,
    /// MUST be set to 0.
    extra_flags_2: u32,
    /// A 32-bit unsigned integer that specifies the number
    /// of special devices to be redirected before the user
    /// is logged on. Special devices are those that are safe
    /// and/or required to be redirected before a user logs
    /// on (such as smart cards and serial ports).
    special_type_device_cap: u32,
}

impl GeneralCapabilitySet {
    #[allow(clippy::manual_bits)]
    const SIZE: usize = size_of::<u32>() * 8 + size_of::<u16>() * 2;

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::SIZE);
        dst.write_u32(self.os_type);
        dst.write_u32(self.os_version);
        dst.write_u16(self.protocol_major_version);
        dst.write_u16(self.protocol_minor_version);
        dst.write_u32(self.io_code_1.bits());
        dst.write_u32(self.io_code_2);
        dst.write_u32(self.extended_pdu.bits());
        dst.write_u32(self.extra_flags_1.bits());
        dst.write_u32(self.extra_flags_2);
        dst.write_u32(self.special_type_device_cap);
        Ok(())
    }

    fn decode(src: &mut ReadCursor<'_>, version: u32) -> PduResult<Self> {
        ensure_size!(in: src, size: Self::SIZE);
        let os_type = src.read_u32();
        let os_version = src.read_u32();
        let protocol_major_version = src.read_u16();
        let protocol_minor_version = src.read_u16();
        let io_code_1 = IoCode1::from_bits_retain(src.read_u32());
        let io_code_2 = src.read_u32();
        let extended_pdu = ExtendedPdu::from_bits_retain(src.read_u32());
        let extra_flags_1 = ExtraFlags1::from_bits_retain(src.read_u32());
        let extra_flags_2 = src.read_u32();
        let special_type_device_cap = if version == GENERAL_CAPABILITY_VERSION_02 {
            src.read_u32()
        } else {
            0
        };

        Ok(Self {
            os_type,
            os_version,
            protocol_major_version,
            protocol_minor_version,
            io_code_1,
            io_code_2,
            extended_pdu,
            extra_flags_1,
            extra_flags_2,
            special_type_device_cap,
        })
    }
}

bitflags! {
    /// A 32-bit unsigned integer that identifies a bitmask of the supported I/O requests for the given device.
    /// If the bit is set, the I/O request is allowed. The requests are identified by the MajorFunction field
    /// in the Device I/O Request (section 2.2.1.4) header. This field MUST be set to a valid combination of
    /// the following values.
    #[derive(Debug, Clone, Copy)]
    struct IoCode1: u32 {
        /// Unused, always set.
        const RDPDR_IRP_MJ_CREATE = 0x0000_0001;
        /// Unused, always set.
        const RDPDR_IRP_MJ_CLEANUP = 0x0000_0002;
        /// Unused, always set.
        const RDPDR_IRP_MJ_CLOSE = 0x0000_0004;
        /// Unused, always set.
        const RDPDR_IRP_MJ_READ = 0x0000_0008;
        /// Unused, always set.
        const RDPDR_IRP_MJ_WRITE = 0x0000_0010;
        /// Unused, always set.
        const RDPDR_IRP_MJ_FLUSH_BUFFERS = 0x0000_0020;
        /// Unused, always set.
        const RDPDR_IRP_MJ_SHUTDOWN = 0x0000_0040;
        /// Unused, always set.
        const RDPDR_IRP_MJ_DEVICE_CONTROL = 0x0000_0080;
        /// Unused, always set.
        const RDPDR_IRP_MJ_QUERY_VOLUME_INFORMATION = 0x0000_0100;
        /// Unused, always set.
        const RDPDR_IRP_MJ_SET_VOLUME_INFORMATION = 0x0000_0200;
        /// Unused, always set.
        const RDPDR_IRP_MJ_QUERY_INFORMATION = 0x0000_0400;
        /// Unused, always set.
        const RDPDR_IRP_MJ_SET_INFORMATION = 0x0000_0800;
        /// Unused, always set.
        const RDPDR_IRP_MJ_DIRECTORY_CONTROL = 0x0000_1000;
        /// Unused, always set.
        const RDPDR_IRP_MJ_LOCK_CONTROL = 0x0000_2000;
        /// Enable Query Security requests (IRP_MJ_QUERY_SECURITY).
        const RDPDR_IRP_MJ_QUERY_SECURITY = 0x0000_4000;
        /// Enable Set Security requests (IRP_MJ_SET_SECURITY).
        const RDPDR_IRP_MJ_SET_SECURITY = 0x0000_8000;

        /// Combination of all the required bits.
        const REQUIRED = Self::RDPDR_IRP_MJ_CREATE.bits()
            | Self::RDPDR_IRP_MJ_CLEANUP.bits()
            | Self::RDPDR_IRP_MJ_CLOSE.bits()
            | Self::RDPDR_IRP_MJ_READ.bits()
            | Self::RDPDR_IRP_MJ_WRITE.bits()
            | Self::RDPDR_IRP_MJ_FLUSH_BUFFERS.bits()
            | Self::RDPDR_IRP_MJ_SHUTDOWN.bits()
            | Self::RDPDR_IRP_MJ_DEVICE_CONTROL.bits()
            | Self::RDPDR_IRP_MJ_QUERY_VOLUME_INFORMATION.bits()
            | Self::RDPDR_IRP_MJ_SET_VOLUME_INFORMATION.bits()
            | Self::RDPDR_IRP_MJ_QUERY_INFORMATION.bits()
            | Self::RDPDR_IRP_MJ_SET_INFORMATION.bits()
            | Self::RDPDR_IRP_MJ_DIRECTORY_CONTROL.bits()
            | Self::RDPDR_IRP_MJ_LOCK_CONTROL.bits();

    }
}

bitflags! {
    /// A 32-bit unsigned integer that specifies extended PDU flags.
    /// This field MUST be set as a bitmask of the following values.
    #[derive(Debug, Clone, Copy)]
    struct ExtendedPdu: u32 {
        /// Allow the client to send Client Drive Device List Remove packets.
        const RDPDR_DEVICE_REMOVE_PDUS = 0x0000_0001;
        /// Unused, always set.
        const RDPDR_CLIENT_DISPLAY_NAME_PDU = 0x0000_0002;
        /// Allow the server to send a Server User Logged On packet.
        const RDPDR_USER_LOGGEDON_PDU = 0x0000_0004;
    }
}

bitflags! {
    /// A 32-bit unsigned integer that specifies extended flags.
    /// The extraFlags1 field MUST be set as a bitmask of the following value.
    #[derive(Debug, Clone, Copy)]
    struct ExtraFlags1: u32 {
        /// Optionally present only in the Client Core Capability Response.
        /// Allows the server to send multiple simultaneous read or write requests
        /// on the same file from a redirected file system.
        const ENABLE_ASYNCIO = 0x0000_0001;
    }
}

/// From VersionMinor in [Server Client ID Confirm (section 2.2.2.6)], [2.2.2.3 Client Announce Reply (DR_CORE_CLIENT_ANNOUNCE_RSP)]
///
/// VERSION_MINOR_12 is what Teleport has successfully been using.
/// There is a version 13 as well, but it's not clear to me what
/// the difference is.
///
/// [Server Client ID Confirm (section 2.2.2.6)]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/bbbb9666-6994-4cf6-8e65-0d46eb319c6e
/// [2.2.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/d6fe6d1b-c145-4a6f-99aa-4fe3cdcea398
const VERSION_MINOR_12: u16 = 0x000C;
const VERSION_MAJOR: u16 = 0x0001;

/// [2.2.2.9] Client Device List Announce Request (DR_CORE_DEVICELIST_ANNOUNCE_REQ)
/// and [2.2.3.1] Client Device List Announce (DR_DEVICELIST_ANNOUNCE)
///
/// [2.2.2.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/10ef9ada-cba2-4384-ab60-7b6290ed4a9a
/// [2.2.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/d8b2bc1c-0207-4c15-abe3-62eaa2afcaf1
#[derive(Debug)]
pub struct ClientDeviceListAnnounce {
    pub device_list: Vec<DeviceAnnounceHeader>,
}

impl ClientDeviceListAnnounce {
    const FIXED_PART_SIZE: usize = size_of::<u32>(); // DeviceCount

    /// Library users should not typically call this directly, use [`Rdpdr::add_drive`] instead.
    pub(crate) fn new_drive(device_id: u32, name: String) -> Self {
        Self {
            device_list: vec![DeviceAnnounceHeader::new_drive(device_id, name)],
        }
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        dst.write_u32(cast_length!(
            "ClientDeviceListAnnounce",
            "DeviceCount",
            self.device_list.len()
        )?);

        for dev in self.device_list.iter() {
            dev.encode(dst)?;
        }

        Ok(())
    }

    pub fn name(&self) -> &'static str {
        "DR_CORE_DEVICELIST_ANNOUNCE_REQ"
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.device_list.iter().map(|d| d.size()).sum::<usize>()
    }
}

#[derive(Debug)]
pub struct Devices(Vec<DeviceAnnounceHeader>);

impl Devices {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn add_smartcard(&mut self, device_id: u32) {
        self.push(DeviceAnnounceHeader::new_smartcard(device_id));
    }

    pub fn add_drive(&mut self, device_id: u32, name: String) {
        self.push(DeviceAnnounceHeader::new_drive(device_id, name));
    }

    /// Returns the [`DeviceType`] for the given device ID.
    pub fn for_device_type(&self, device_id: u32) -> PduResult<DeviceType> {
        if let Some(device_type) = self.0.iter().find(|d| d.device_id == device_id).map(|d| d.device_type) {
            Ok(device_type)
        } else {
            Err(invalid_message_err!(
                "Devices::for_device_type",
                "device_id",
                "no device with that ID"
            ))
        }
    }

    fn push(&mut self, device: DeviceAnnounceHeader) {
        self.0.push(device);
    }

    pub fn clone_inner(&mut self) -> Vec<DeviceAnnounceHeader> {
        self.0.clone()
    }
}

impl Default for Devices {
    fn default() -> Self {
        Self::new()
    }
}

/// [2.2.1.3] Device Announce Header (DEVICE_ANNOUNCE)
///
/// [2.2.1.3]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/32e34332-774b-4ead-8c9d-5d64720d6bf9
#[derive(Debug, Clone)]
pub struct DeviceAnnounceHeader {
    device_type: DeviceType,
    device_id: u32,
    preferred_dos_name: PreferredDosName,
    device_data: Vec<u8>,
}

impl DeviceAnnounceHeader {
    const FIXED_PART_SIZE: usize = size_of::<u32>() * 3 + 8; // DeviceType, DeviceId, DeviceDataLength, PreferredDosName

    pub fn new_smartcard(device_id: u32) -> Self {
        Self {
            device_type: DeviceType::Smartcard,
            device_id,
            // This name is a constant defined by the spec.
            preferred_dos_name: PreferredDosName("SCARD".to_owned()),
            device_data: Vec::new(),
        }
    }

    fn new_drive(device_id: u32, name: String) -> Self {
        // The spec says Unicode but empirically this wants null terminated UTF-8.
        let mut device_data = name.into_bytes();
        device_data.push(0u8);

        Self {
            device_type: DeviceType::Filesystem,
            device_id,
            // "The drive name MUST be specified in the PreferredDosName field; however, if the drive name is larger than the allocated size of the PreferredDosName field,
            // then the drive name MUST be truncated to fit. If the client supports DRIVE_CAPABILITY_VERSION_02 in the Drive Capability Set, then the full name MUST also
            // be specified in the DeviceData field, as a null-terminated Unicode string. If the DeviceDataLength field is nonzero, the content of the PreferredDosName field
            // is ignored."
            //
            // Since we do support DRIVE_CAPABILITY_VERSION_02, we'll put the full name in the DeviceData field.
            preferred_dos_name: PreferredDosName("ignored".to_owned()),
            device_data,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        dst.write_u32(self.device_type.into());
        dst.write_u32(self.device_id);
        self.preferred_dos_name.encode(dst)?;
        dst.write_u32(cast_length!(
            "DeviceAnnounceHeader",
            "DeviceDataLength",
            self.device_data.len()
        )?);
        dst.write_slice(&self.device_data);
        Ok(())
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.device_data.len()
    }
}

/// From ["PreferredDosName"](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/32e34332-774b-4ead-8c9d-5d64720d6bf9):
///
/// PreferredDosName (8 bytes): A string of ASCII characters (with a maximum length of eight characters) that represents the name of the device as it appears on the client. This field MUST be null-terminated, so the maximum device name is 7 characters long. The following characters are considered invalid for the PreferredDosName field:
///
/// <, >, ", /, \, |
///
/// If any of these characters are present, the DR_CORE_DEVICE_ANNOUNC_RSP packet for this device (section 2.2.2.1) will be sent with STATUS_ACCESS_DENIED set in the ResultCode field.
///
/// If DeviceType is set to RDPDR_DTYP_SMARTCARD, the PreferredDosName MUST be set to "SCARD".
///
/// Note A column character, ":", is valid only when present at the end of the PreferredDosName field, otherwise it is also considered invalid.
#[derive(Debug, Clone)]
struct PreferredDosName(String);

impl PreferredDosName {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        write_string_to_cursor(dst, &self.format(), CharacterSet::Ansi, false)
    }

    /// Returns the underlying String with a maximum length of 7 characters plus a null terminator.
    fn format(&self) -> String {
        let mut name: &str = &self.0;
        if name.len() > 7 {
            name = &name[..7];
        }
        format!("{name:\x00<8}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u32)]
pub enum DeviceType {
    /// RDPDR_DTYP_SERIAL
    Serial = 0x0000_0001,
    /// RDPDR_DTYP_PARALLEL
    Parallel = 0x0000_0002,
    /// RDPDR_DTYP_PRINT
    Print = 0x0000_0004,
    /// RDPDR_DTYP_FILESYSTEM
    Filesystem = 0x0000_0008,
    /// RDPDR_DTYP_SMARTCARD
    Smartcard = 0x0000_0020,
}

impl From<DeviceType> for u32 {
    fn from(device_type: DeviceType) -> Self {
        device_type as u32
    }
}

impl TryFrom<u32> for DeviceType {
    type Error = PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x0000_0001 => Ok(DeviceType::Serial),
            0x0000_0002 => Ok(DeviceType::Parallel),
            0x0000_0004 => Ok(DeviceType::Print),
            0x0000_0008 => Ok(DeviceType::Filesystem),
            0x0000_0020 => Ok(DeviceType::Smartcard),
            _ => Err(invalid_message_err!("try_from", "DeviceType", "invalid value")),
        }
    }
}

/// [2.2.2.1] Server Device Announce Response (DR_CORE_DEVICE_ANNOUNCE_RSP)
///
/// [2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/a4c0b619-6e87-4721-bdc4-5d2db7f485f3
#[derive(Debug)]
pub struct ServerDeviceAnnounceResponse {
    pub device_id: u32,
    pub result_code: NtStatus,
}

impl ServerDeviceAnnounceResponse {
    const NAME: &'static str = "DR_CORE_DEVICE_ANNOUNCE_RSP";
    const FIXED_PART_SIZE: usize = size_of::<u32>() * 2; // DeviceId, ResultCode

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.device_id);
        dst.write_u32(self.result_code.into());
        Ok(())
    }

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(ctx: Self::NAME, in: src, size: Self::FIXED_PART_SIZE);
        let device_id = src.read_u32();
        let result_code = NtStatus::from(src.read_u32());

        Ok(Self { device_id, result_code })
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [2.3.1] NTSTATUS Values
///
/// Windows defines an absolutely massive list of potential NTSTATUS values.
/// This enum includes some basic ones for communicating with the RDP server.
///
/// [2.3.1]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-erref/596a1078-e883-4972-9bbc-49e60bebca55
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NtStatus(u32);

impl NtStatus {
    /// STATUS_SUCCESS
    pub const SUCCESS: Self = Self(0x0000_0000);
    /// STATUS_UNSUCCESSFUL
    pub const UNSUCCESSFUL: Self = Self(0xC000_0001);
    /// STATUS_NOT_IMPLEMENTED
    pub const NOT_IMPLEMENTED: Self = Self(0xC000_0002);
    /// STATUS_NO_MORE_FILES
    pub const NO_MORE_FILES: Self = Self(0x8000_0006);
    /// STATUS_OBJECT_NAME_COLLISION
    pub const OBJECT_NAME_COLLISION: Self = Self(0xC000_0035);
    /// STATUS_ACCESS_DENIED
    pub const ACCESS_DENIED: Self = Self(0xC000_0022);
    /// STATUS_NOT_A_DIRECTORY
    pub const NOT_A_DIRECTORY: Self = Self(0xC000_0103);
    /// STATUS_NO_SUCH_FILE
    pub const NO_SUCH_FILE: Self = Self(0xC000_000F);
    /// STATUS_NOT_SUPPORTED
    pub const NOT_SUPPORTED: Self = Self(0xC000_00BB);
    /// STATUS_DIRECTORY_NOT_EMPTY
    pub const DIRECTORY_NOT_EMPTY: Self = Self(0xC000_0101);
}

impl Debug for NtStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            NtStatus::SUCCESS => write!(f, "STATUS_SUCCESS"),
            NtStatus::UNSUCCESSFUL => write!(f, "STATUS_UNSUCCESSFUL"),
            NtStatus::NOT_IMPLEMENTED => write!(f, "STATUS_NOT_IMPLEMENTED"),
            NtStatus::NO_MORE_FILES => write!(f, "STATUS_NO_MORE_FILES"),
            NtStatus::OBJECT_NAME_COLLISION => write!(f, "STATUS_OBJECT_NAME_COLLISION"),
            NtStatus::ACCESS_DENIED => write!(f, "STATUS_ACCESS_DENIED"),
            NtStatus::NOT_A_DIRECTORY => write!(f, "STATUS_NOT_A_DIRECTORY"),
            NtStatus::NO_SUCH_FILE => write!(f, "STATUS_NO_SUCH_FILE"),
            NtStatus::NOT_SUPPORTED => write!(f, "STATUS_NOT_SUPPORTED"),
            NtStatus::DIRECTORY_NOT_EMPTY => write!(f, "STATUS_DIRECTORY_NOT_EMPTY"),
            _ => write!(f, "NtStatus({:#010X})", self.0),
        }
    }
}

impl From<u32> for NtStatus {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<NtStatus> for u32 {
    fn from(status: NtStatus) -> Self {
        status.0
    }
}

/// [2.2.1.4] Device I/O Request (DR_DEVICE_IOREQUEST)
///
/// [2.2.1.4]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/a087ffa8-d0d5-4874-ac7b-0494f63e2d5d
#[derive(Debug, Clone)]
pub struct DeviceIoRequest {
    pub device_id: u32,
    pub file_id: u32,
    pub completion_id: u32,
    pub major_function: MajorFunction,
    pub minor_function: MinorFunction,
}

impl DeviceIoRequest {
    const NAME: &'static str = "DR_DEVICE_IOREQUEST";
    const FIXED_PART_SIZE: usize = size_of::<u32>() * 5; // DeviceId, FileId, CompletionId, MajorFunction, MinorFunction

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.device_id);
        dst.write_u32(self.file_id);
        dst.write_u32(self.completion_id);
        dst.write_u32(self.major_function.into());
        dst.write_u32(self.minor_function.into());
        Ok(())
    }

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(ctx: Self::NAME, in: src, size: Self::FIXED_PART_SIZE);
        let device_id = src.read_u32();
        let file_id = src.read_u32();
        let completion_id = src.read_u32();
        let major_function = MajorFunction::try_from(src.read_u32())?;
        let minor_function = MinorFunction::from(src.read_u32());

        Ok(Self {
            device_id,
            file_id,
            completion_id,
            major_function,
            minor_function,
        })
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// See [`DeviceIoRequest`].
#[derive(Debug, PartialEq, Clone, Copy)]
#[repr(u32)]
pub enum MajorFunction {
    /// IRP_MJ_CREATE
    Create = 0x0000_0000,
    /// IRP_MJ_CLOSE
    Close = 0x0000_0002,
    /// IRP_MJ_READ
    Read = 0x0000_0003,
    /// IRP_MJ_WRITE
    Write = 0x0000_0004,
    /// IRP_MJ_DEVICE_CONTROL
    DeviceControl = 0x0000_000e,
    /// IRP_MJ_QUERY_VOLUME_INFORMATION
    QueryVolumeInformation = 0x0000_000a,
    /// IRP_MJ_SET_VOLUME_INFORMATION
    SetVolumeInformation = 0x0000_000b,
    /// IRP_MJ_QUERY_INFORMATION
    QueryInformation = 0x0000_0005,
    /// IRP_MJ_SET_INFORMATION
    SetInformation = 0x0000_0006,
    /// IRP_MJ_DIRECTORY_CONTROL
    DirectoryControl = 0x0000_000c,
    /// IRP_MJ_LOCK_CONTROL
    LockControl = 0x0000_0011,
}

impl TryFrom<u32> for MajorFunction {
    type Error = PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x0000_0000 => Ok(MajorFunction::Create),
            0x0000_0002 => Ok(MajorFunction::Close),
            0x0000_0003 => Ok(MajorFunction::Read),
            0x0000_0004 => Ok(MajorFunction::Write),
            0x0000_000e => Ok(MajorFunction::DeviceControl),
            0x0000_000a => Ok(MajorFunction::QueryVolumeInformation),
            0x0000_000b => Ok(MajorFunction::SetVolumeInformation),
            0x0000_0005 => Ok(MajorFunction::QueryInformation),
            0x0000_0006 => Ok(MajorFunction::SetInformation),
            0x0000_000c => Ok(MajorFunction::DirectoryControl),
            0x0000_0011 => Ok(MajorFunction::LockControl),
            _ => Err(invalid_message_err!("try_from", "MajorFunction", "unsupported value")),
        }
    }
}

impl From<MajorFunction> for u32 {
    fn from(major_function: MajorFunction) -> Self {
        major_function as u32
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
/// A 32-bit unsigned integer. This field is valid only when the MajorFunction field is
/// set to IRP_MJ_DIRECTORY_CONTROL. If the MajorFunction field is set to another value,
/// the MinorFunction field value SHOULD be 0x00000000; otherwise, the MinorFunction
/// field MUST have one of the following values:
///
/// 1. [`MinorFunction::IRP_MN_QUERY_DIRECTORY`]
/// 2. [`MinorFunction::IRP_MN_NOTIFY_CHANGE_DIRECTORY`]
pub struct MinorFunction(u32);

impl MinorFunction {
    pub const IRP_MN_QUERY_DIRECTORY: Self = Self(0x00000001);
    pub const IRP_MN_NOTIFY_CHANGE_DIRECTORY: Self = Self(0x00000002);
}

impl Debug for MinorFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            MinorFunction::IRP_MN_QUERY_DIRECTORY => write!(f, "IRP_MN_QUERY_DIRECTORY"),
            MinorFunction::IRP_MN_NOTIFY_CHANGE_DIRECTORY => write!(f, "IRP_MN_NOTIFY_CHANGE_DIRECTORY"),
            _ => write!(f, "MinorFunction({:#010X})", self.0),
        }
    }
}

impl From<u32> for MinorFunction {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<MinorFunction> for u32 {
    fn from(minor_function: MinorFunction) -> Self {
        minor_function.0
    }
}

impl From<MinorFunction> for u8 {
    fn from(minor_function: MinorFunction) -> Self {
        minor_function.0 as u8
    }
}

/// [2.2.1.4.5] Device Control Request (DR_CONTROL_REQ)
///
/// [2.2.1.4.5]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/30662c80-ec6e-4ed1-9004-2e6e367bb59f
#[derive(Debug)]
pub struct DeviceControlRequest<T: IoCtlCode> {
    pub header: DeviceIoRequest,
    pub output_buffer_length: u32,
    pub input_buffer_length: u32,
    pub io_control_code: T,
}

impl<T: IoCtlCode> DeviceControlRequest<T>
where
    T::Error: ironrdp_error::Source,
{
    fn headerless_size() -> usize {
        size_of::<u32>() * 3 // OutputBufferLength, InputBufferLength, IoControlCode
    }

    pub fn decode(header: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(ctx: "DeviceControlRequest", in: src, size: Self::headerless_size());
        let output_buffer_length = src.read_u32();
        let input_buffer_length = src.read_u32();
        let io_control_code = T::try_from(src.read_u32()).map_err(|e| {
            error!("Failed to parse IoCtlCode");
            invalid_message_err!("DeviceControlRequest", "IoCtlCode", "invalid IoCtlCode").with_source(e)
        })?;

        // Padding (20 bytes): An array of 20 bytes. Reserved. This field can be set to any value and MUST be ignored.
        read_padding!(src, 20);

        Ok(Self {
            header,
            output_buffer_length,
            input_buffer_length,
            io_control_code,
        })
    }
}

/// A 32-bit unsigned integer. This field is specific to the redirected device.
pub trait IoCtlCode: TryFrom<u32> {}

/// An IoCtlCode that can be used when the IoCtlCode is not known
/// or not important.
#[derive(Debug)]
pub struct AnyIoCtlCode(pub u32);

impl TryFrom<u32> for AnyIoCtlCode {
    type Error = PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(Self(value))
    }
}

impl IoCtlCode for AnyIoCtlCode {}

/// [2.2.1.5.5] Device Control Response (DR_CONTROL_RSP)
///
/// [2.2.1.5.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/a00fbce4-95bb-4e15-8182-be2b5ef9076a
#[derive(Debug)]
pub struct DeviceControlResponse {
    pub device_io_reply: DeviceIoResponse,
    /// A value of `None` represents an empty buffer,
    /// such as can be seen in FreeRDP [here].
    ///
    /// [here]: https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_main.c#L677-L684
    pub output_buffer: Option<Box<dyn rpce::Encode>>,
}

impl DeviceControlResponse {
    const NAME: &'static str = "DR_CONTROL_RSP";

    /// A value of `None` for `output_buffer` represents an empty buffer,
    /// such as can be seen in FreeRDP [here].
    ///
    /// [here]: https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_main.c#L677-L684
    pub fn new<T: IoCtlCode>(
        req: DeviceControlRequest<T>,
        io_status: NtStatus,
        output_buffer: Option<Box<dyn rpce::Encode>>,
    ) -> Self {
        Self {
            device_io_reply: DeviceIoResponse {
                device_id: req.header.device_id,
                completion_id: req.header.completion_id,
                io_status,
            },
            output_buffer,
        }
    }

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.device_io_reply.encode(dst)?;
        if let Some(output_buffer) = &self.output_buffer {
            dst.write_u32(cast_length!(
                "DeviceControlResponse",
                "OutputBufferLength",
                output_buffer.size()
            )?);
            output_buffer.encode(dst)?;
        } else {
            dst.write_u32(0); // OutputBufferLength
        }

        Ok(())
    }

    pub fn size(&self) -> usize {
        self.device_io_reply.size() // DeviceIoResponse
            + 4 // OutputBufferLength
            + if let Some(output_buffer) = &self.output_buffer {
                output_buffer.size() // OutputBuffer
            } else {
                0 // OutputBuffer
            }
    }
}

/// [2.2.1.5] Device I/O Response (DR_DEVICE_IOCOMPLETION)
///
/// [2.2.1.5]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/1c412a84-0776-4984-b35c-3f0445fcae65
#[derive(Debug)]
pub struct DeviceIoResponse {
    pub device_id: u32,
    pub completion_id: u32,
    pub io_status: NtStatus,
}

impl DeviceIoResponse {
    const FIXED_PART_SIZE: usize = size_of::<u32>() * 3; // DeviceId, CompletionId, IoStatus

    pub fn new(device_io_request: DeviceIoRequest, io_status: NtStatus) -> Self {
        Self {
            device_id: device_io_request.device_id,
            completion_id: device_io_request.completion_id,
            io_status,
        }
    }

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(ctx: "DeviceIoResponse", in: src, size: Self::FIXED_PART_SIZE);
        let device_id = src.read_u32();
        let completion_id = src.read_u32();
        let io_status = NtStatus::from(src.read_u32());

        Ok(Self {
            device_id,
            completion_id,
            io_status,
        })
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.device_id);
        dst.write_u32(self.completion_id);
        dst.write_u32(self.io_status.into());
        Ok(())
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [2.2.3.3] Server Drive I/O Request (DR_DRIVE_CORE_DEVICE_IOREQUEST)
///
/// [2.2.3.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/89bb51af-c54d-40fb-81c1-d1bb353c4536
#[derive(Debug)]
pub enum ServerDriveIoRequest {
    ServerCreateDriveRequest(DeviceCreateRequest),
    ServerDriveQueryInformationRequest(ServerDriveQueryInformationRequest),
    DeviceCloseRequest(DeviceCloseRequest),
    ServerDriveQueryDirectoryRequest(ServerDriveQueryDirectoryRequest),
    ServerDriveNotifyChangeDirectoryRequest(ServerDriveNotifyChangeDirectoryRequest),
    ServerDriveQueryVolumeInformationRequest(ServerDriveQueryVolumeInformationRequest),
    DeviceControlRequest(DeviceControlRequest<AnyIoCtlCode>),
    DeviceReadRequest(DeviceReadRequest),
    DeviceWriteRequest(DeviceWriteRequest),
    ServerDriveSetInformationRequest(ServerDriveSetInformationRequest),
    ServerDriveLockControlRequest(ServerDriveLockControlRequest),
}

impl ServerDriveIoRequest {
    pub fn decode(dev_io_req: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        match dev_io_req.major_function {
            MajorFunction::Create => Ok(DeviceCreateRequest::decode(dev_io_req, src)?.into()),
            MajorFunction::Close => Ok(DeviceCloseRequest::decode(dev_io_req).into()),
            MajorFunction::Read => Ok(DeviceReadRequest::decode(dev_io_req, src)?.into()),
            MajorFunction::Write => Ok(DeviceWriteRequest::decode(dev_io_req, src)?.into()),
            MajorFunction::DeviceControl => Ok(DeviceControlRequest::<AnyIoCtlCode>::decode(dev_io_req, src)?.into()),
            MajorFunction::QueryVolumeInformation => {
                Ok(ServerDriveQueryVolumeInformationRequest::decode(dev_io_req, src)?.into())
            }
            MajorFunction::SetVolumeInformation => Err(unsupported_pdu_err!(
                "ServerDriveIoRequest::decode",
                "MajorFunction",
                "SetVolumeInformation".to_owned()
            )), // FreeRDP doesn't implement this
            MajorFunction::QueryInformation => Ok(ServerDriveQueryInformationRequest::decode(dev_io_req, src)?.into()),
            MajorFunction::SetInformation => Ok(ServerDriveSetInformationRequest::decode(dev_io_req, src)?.into()),
            MajorFunction::DirectoryControl => match dev_io_req.minor_function {
                MinorFunction::IRP_MN_QUERY_DIRECTORY => {
                    Ok(ServerDriveQueryDirectoryRequest::decode(dev_io_req, src)?.into())
                }
                MinorFunction::IRP_MN_NOTIFY_CHANGE_DIRECTORY => {
                    Ok(ServerDriveNotifyChangeDirectoryRequest::decode(dev_io_req, src)?.into())
                }
                // If MajorFunction is set to IRP_MJ_DIRECTORY_CONTROL and MinorFunction is set to any other value, we've encountered a server bug.
                _ => Err(invalid_message_err!(
                    "ServerDriveIoRequest::decode",
                    "MinorFunction",
                    "invalid value"
                )),
            },
            MajorFunction::LockControl => Ok(ServerDriveLockControlRequest::decode(dev_io_req, src)?.into()),
        }
    }
}

impl From<DeviceCreateRequest> for ServerDriveIoRequest {
    fn from(req: DeviceCreateRequest) -> Self {
        Self::ServerCreateDriveRequest(req)
    }
}

impl From<ServerDriveQueryInformationRequest> for ServerDriveIoRequest {
    fn from(req: ServerDriveQueryInformationRequest) -> Self {
        Self::ServerDriveQueryInformationRequest(req)
    }
}

impl From<DeviceCloseRequest> for ServerDriveIoRequest {
    fn from(req: DeviceCloseRequest) -> Self {
        Self::DeviceCloseRequest(req)
    }
}

impl From<ServerDriveQueryDirectoryRequest> for ServerDriveIoRequest {
    fn from(req: ServerDriveQueryDirectoryRequest) -> Self {
        Self::ServerDriveQueryDirectoryRequest(req)
    }
}

impl From<ServerDriveNotifyChangeDirectoryRequest> for ServerDriveIoRequest {
    fn from(req: ServerDriveNotifyChangeDirectoryRequest) -> Self {
        Self::ServerDriveNotifyChangeDirectoryRequest(req)
    }
}

impl From<ServerDriveQueryVolumeInformationRequest> for ServerDriveIoRequest {
    fn from(req: ServerDriveQueryVolumeInformationRequest) -> Self {
        Self::ServerDriveQueryVolumeInformationRequest(req)
    }
}

impl From<DeviceControlRequest<AnyIoCtlCode>> for ServerDriveIoRequest {
    fn from(req: DeviceControlRequest<AnyIoCtlCode>) -> Self {
        Self::DeviceControlRequest(req)
    }
}

impl From<DeviceReadRequest> for ServerDriveIoRequest {
    fn from(req: DeviceReadRequest) -> Self {
        Self::DeviceReadRequest(req)
    }
}

impl From<DeviceWriteRequest> for ServerDriveIoRequest {
    fn from(req: DeviceWriteRequest) -> Self {
        Self::DeviceWriteRequest(req)
    }
}

impl From<ServerDriveSetInformationRequest> for ServerDriveIoRequest {
    fn from(req: ServerDriveSetInformationRequest) -> Self {
        Self::ServerDriveSetInformationRequest(req)
    }
}

impl From<ServerDriveLockControlRequest> for ServerDriveIoRequest {
    fn from(req: ServerDriveLockControlRequest) -> Self {
        Self::ServerDriveLockControlRequest(req)
    }
}

/// [2.2.3.3.1] Server Create Drive Request (DR_DRIVE_CREATE_REQ)
/// and [2.2.1.4.1] Device Create Request (DR_CREATE_REQ)
///
/// [2.2.3.3.1]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/95b16fd0-d530-407c-a310-adedc85e9897
/// [2.2.1.4.1]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/5f71f6d2-d9ff-40c2-bdb5-a739447d3c3e
#[derive(Debug)]
pub struct DeviceCreateRequest {
    /// The MajorFunction field in this header MUST be set to IRP_MJ_CREATE.
    pub device_io_request: DeviceIoRequest,
    pub desired_access: DesiredAccess,
    pub allocation_size: u64,
    pub file_attributes: FileAttributes,
    pub shared_access: SharedAccess,
    pub create_disposition: CreateDisposition,
    pub create_options: CreateOptions,
    pub path: String,
}

impl DeviceCreateRequest {
    const FIXED_PART_SIZE: usize = 4  // DesiredAccess
                                 + 8  // AllocationSize
                                 + 4  // FileAttributes
                                 + 4  // SharedAccess
                                 + 4  // CreateDisposition
                                 + 4  // CreateOptions
                                 + 4; // PathLength

    fn decode(dev_io_req: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(ctx: "DeviceCreateRequest", in: src, size: Self::FIXED_PART_SIZE);
        let desired_access = DesiredAccess::from_bits_retain(src.read_u32());
        let allocation_size = src.read_u64();
        let file_attributes = FileAttributes::from_bits_retain(src.read_u32());
        let shared_access = SharedAccess::from_bits_retain(src.read_u32());
        let create_disposition = CreateDisposition::from_bits_retain(src.read_u32());
        let create_options = CreateOptions::from_bits_retain(src.read_u32());
        let path_length: usize = cast_length!("DeviceCreateRequest", "path_length", src.read_u32())?;

        ensure_size!(ctx: "DeviceCreateRequest", in: src, size: path_length);
        let path = from_utf16_bytes(src.read_slice(path_length))
            .trim_end_matches('\0')
            .into();

        Ok(Self {
            device_io_request: dev_io_req,
            desired_access,
            allocation_size,
            file_attributes,
            shared_access,
            create_disposition,
            create_options,
            path,
        })
    }
}

bitflags! {
    /// DesiredAccess can be interpreted as either
    /// [2.2.13.1.1] File_Pipe_Printer_Access_Mask \[MS-SMB2\] or [2.2.13.1.2] Directory_Access_Mask \[MS-SMB2\]
    ///
    /// This implements the combination of the two. For flags where the names and/or functions are distinct between the two,
    /// the names are appended with an "_OR_", and the File_Pipe_Printer_Access_Mask functionality is described on the top line comment,
    /// and the Directory_Access_Mask functionality is described on the bottom (2nd) line comment.
    ///
    /// [2.2.13.1.1]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-smb2/77b36d0f-6016-458a-a7a0-0f4a72ae1534
    /// [2.2.13.1.2]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-smb2/0a5934b1-80f1-4da0-b1bf-5e021c309b71
    #[derive(Debug, Clone)]
    pub struct DesiredAccess: u32 {
        /// This value indicates the right to read data from the file or named pipe.
        ///
        /// This value indicates the right to enumerate the contents of the directory.
        const FILE_READ_DATA_OR_FILE_LIST_DIRECTORY = 0x00000001;
        /// This value indicates the right to write data into the file or named pipe beyond the end of the file.
        ///
        /// This value indicates the right to create a file under the directory.
        const FILE_WRITE_DATA_OR_FILE_ADD_FILE = 0x00000002;
        /// This value indicates the right to append data into the file or named pipe.
        ///
        /// This value indicates the right to add a sub-directory under the directory.
        const FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY = 0x00000004;
        /// This value indicates the right to read the extended attributes of the file or named pipe.
        const FILE_READ_EA = 0x00000008;
        /// This value indicates the right to write or change the extended attributes to the file or named pipe.
        const FILE_WRITE_EA = 0x00000010;
        /// This value indicates the right to traverse this directory if the server enforces traversal checking.
        const FILE_TRAVERSE = 0x00000020;
        /// This value indicates the right to delete entries within a directory.
        const FILE_DELETE_CHILD = 0x00000040;
        /// This value indicates the right to execute the file/directory.
        const FILE_EXECUTE = 0x00000020;
        /// This value indicates the right to read the attributes of the file/directory.
        const FILE_READ_ATTRIBUTES = 0x00000080;
        /// This value indicates the right to change the attributes of the file/directory.
        const FILE_WRITE_ATTRIBUTES = 0x00000100;
        /// This value indicates the right to delete the file/directory.
        const DELETE = 0x00010000;
        /// This value indicates the right to read the security descriptor for the file/directory or named pipe.
        const READ_CONTROL = 0x00020000;
        /// This value indicates the right to change the discretionary access control list (DACL) in the security descriptor for the file/directory or named pipe. For the DACL data pub structure, see ACL in [MS-DTYP].
        const WRITE_DAC = 0x00040000;
        /// This value indicates the right to change the owner in the security descriptor for the file/directory or named pipe.
        const WRITE_OWNER = 0x00080000;
        /// SMB2 clients set this flag to any value. SMB2 servers SHOULD ignore this flag.
        const SYNCHRONIZE = 0x00100000;
        /// This value indicates the right to read or change the system access control list (SACL) in the security descriptor for the file/directory or named pipe. For the SACL data pub structure, see ACL in [MS-DTYP].
        const ACCESS_SYSTEM_SECURITY = 0x01000000;
        /// This value indicates that the client is requesting an open to the file with the highest level of access the client has on this file. If no access is granted for the client on this file, the server MUST fail the open with STATUS_ACCESS_DENIED.
        const MAXIMUM_ALLOWED = 0x02000000;
        /// This value indicates a request for all the access flags that are previously listed except MAXIMUM_ALLOWED and ACCESS_SYSTEM_SECURITY.
        const GENERIC_ALL = 0x10000000;
        /// This value indicates a request for the following combination of access flags listed above: FILE_READ_ATTRIBUTES| FILE_EXECUTE| SYNCHRONIZE| READ_CONTROL.
        const GENERIC_EXECUTE = 0x20000000;
        /// This value indicates a request for the following combination of access flags listed above: FILE_WRITE_DATA| FILE_APPEND_DATA| FILE_WRITE_ATTRIBUTES| FILE_WRITE_EA| SYNCHRONIZE| READ_CONTROL.
        const GENERIC_WRITE = 0x40000000;
        /// This value indicates a request for the following combination of access flags listed above: FILE_READ_DATA| FILE_READ_ATTRIBUTES| FILE_READ_EA| SYNCHRONIZE| READ_CONTROL.
        const GENERIC_READ = 0x80000000;
    }
}

bitflags! {
    /// [2.6] File Attributes \[MS-FSCC\]
    ///
    /// [2.6]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/ca28ec38-f155-4768-81d6-4bfeb8586fc9
    #[derive(Debug, Clone)]
    pub struct FileAttributes: u32 {
        const FILE_ATTRIBUTE_READONLY = 0x00000001;
        const FILE_ATTRIBUTE_HIDDEN = 0x00000002;
        const FILE_ATTRIBUTE_SYSTEM = 0x00000004;
        const FILE_ATTRIBUTE_DIRECTORY = 0x00000010;
        const FILE_ATTRIBUTE_ARCHIVE = 0x00000020;
        const FILE_ATTRIBUTE_NORMAL = 0x00000080;
        const FILE_ATTRIBUTE_TEMPORARY = 0x00000100;
        const FILE_ATTRIBUTE_SPARSE_FILE = 0x00000200;
        const FILE_ATTRIBUTE_REPARSE_POINT = 0x00000400;
        const FILE_ATTRIBUTE_COMPRESSED = 0x00000800;
        const FILE_ATTRIBUTE_OFFLINE = 0x00001000;
        const FILE_ATTRIBUTE_NOT_CONTENT_INDEXED = 0x00002000;
        const FILE_ATTRIBUTE_ENCRYPTED = 0x00004000;
        const FILE_ATTRIBUTE_INTEGRITY_STREAM = 0x00008000;
        const FILE_ATTRIBUTE_NO_SCRUB_DATA = 0x00020000;
        const FILE_ATTRIBUTE_RECALL_ON_OPEN = 0x00040000;
        const FILE_ATTRIBUTE_PINNED = 0x00080000;
        const FILE_ATTRIBUTE_UNPINNED = 0x00100000;
        const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS = 0x00400000;

        const _ = !0;
    }
}

bitflags! {
    /// Specified in [2.2.13] SMB2 CREATE Request
    ///
    /// [2.2.13]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-smb2/e8fb45c1-a03d-44ca-b7ae-47385cfd7997
    #[derive(Debug, Clone)]
    pub struct SharedAccess: u32 {
        const FILE_SHARE_READ = 0x00000001;
        const FILE_SHARE_WRITE = 0x00000002;
        const FILE_SHARE_DELETE = 0x00000004;
    }
}

bitflags! {
    /// Defined in [2.2.13] SMB2 CREATE Request
    ///
    /// See FreeRDP's [drive_file.c] for context about how these should be interpreted.
    ///
    /// [2.2.13]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-smb2/e8fb45c1-a03d-44ca-b7ae-47385cfd7997
    /// [drive_file.c]: https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_file.c#L207
    #[derive(PartialEq, Eq, Debug, Clone)]
    pub struct CreateDisposition: u32 {
        const FILE_SUPERSEDE = 0x00000000;
        const FILE_OPEN = 0x00000001;
        const FILE_CREATE = 0x00000002;
        const FILE_OPEN_IF = 0x00000003;
        const FILE_OVERWRITE = 0x00000004;
        const FILE_OVERWRITE_IF = 0x00000005;
    }
}

bitflags! {
    /// Defined in [2.2.13] SMB2 CREATE Request
    ///
    /// [2.2.13]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-smb2/e8fb45c1-a03d-44ca-b7ae-47385cfd7997
    #[derive(Debug, Clone)]
    pub struct CreateOptions: u32 {
        const FILE_DIRECTORY_FILE = 0x00000001;
        const FILE_WRITE_THROUGH = 0x00000002;
        const FILE_SEQUENTIAL_ONLY = 0x00000004;
        const FILE_NO_INTERMEDIATE_BUFFERING = 0x00000008;
        const FILE_SYNCHRONOUS_IO_ALERT = 0x00000010;
        const FILE_SYNCHRONOUS_IO_NONALERT = 0x00000020;
        const FILE_NON_DIRECTORY_FILE = 0x00000040;
        const FILE_COMPLETE_IF_OPLOCKED = 0x00000100;
        const FILE_NO_EA_KNOWLEDGE = 0x00000200;
        const FILE_RANDOM_ACCESS = 0x00000800;
        const FILE_DELETE_ON_CLOSE = 0x00001000;
        const FILE_OPEN_BY_FILE_ID = 0x00002000;
        const FILE_OPEN_FOR_BACKUP_INTENT = 0x00004000;
        const FILE_NO_COMPRESSION = 0x00008000;
        const FILE_OPEN_REMOTE_INSTANCE = 0x00000400;
        const FILE_OPEN_REQUIRING_OPLOCK = 0x00010000;
        const FILE_DISALLOW_EXCLUSIVE = 0x00020000;
        const FILE_RESERVE_OPFILTER = 0x00100000;
        const FILE_OPEN_REPARSE_POINT = 0x00200000;
        const FILE_OPEN_NO_RECALL = 0x00400000;
        const FILE_OPEN_FOR_FREE_SPACE_QUERY = 0x00800000;
    }
}

/// [2.2.1.5.1] Device Create Response (DR_CREATE_RSP)
///
/// [2.2.1.5.1]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/99e5fca5-b37a-41e4-bc69-8d7da7860f76
#[derive(Debug)]
pub struct DeviceCreateResponse {
    pub device_io_reply: DeviceIoResponse,
    pub file_id: u32,
    pub information: Information,
}

impl DeviceCreateResponse {
    const NAME: &'static str = "DR_CREATE_RSP";

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.device_io_reply.encode(dst)?;
        dst.write_u32(self.file_id);
        dst.write_u8(self.information.bits());
        Ok(())
    }

    pub fn size(&self) -> usize {
        self.device_io_reply.size() // DeviceIoReply
        + 4 // FileId
        + 1 // Information
    }
}

bitflags! {
    /// Defined in [2.2.1.5.1] Device Create Response (DR_CREATE_RSP)
    ///
    /// [2.2.1.5.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/99e5fca5-b37a-41e4-bc69-8d7da7860f76
    #[derive(Debug)]
    pub struct Information: u8 {
        /// A new file was created.
        const FILE_SUPERSEDED = 0x00000000;
        /// An existing file was opened.
        const FILE_OPENED = 0x00000001;
        /// An existing file was overwritten.
        const FILE_OVERWRITTEN = 0x00000003;
    }
}

/// [2.2.3.3.8] Server Drive Query Information Request (DR_DRIVE_QUERY_INFORMATION_REQ)
///
/// Note that Length, Padding, and QueryBuffer fields are all ignored in keeping with the [analogous code in FreeRDP].
///
/// [2.2.3.3.8]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/e43dcd68-2980-40a9-9238-344b6cf94946
/// [analogous code in FreeRDP]: https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_main.c#L384
#[derive(Debug)]
pub struct ServerDriveQueryInformationRequest {
    pub device_io_request: DeviceIoRequest,
    pub file_info_class_lvl: FileInformationClassLevel,
}

impl ServerDriveQueryInformationRequest {
    pub fn decode(dev_io_req: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(ctx: "ServerDriveQueryInformationRequest", in: src, size: 4);
        let file_info_class_lvl = FileInformationClassLevel::from(src.read_u32());

        Ok(Self {
            device_io_request: dev_io_req,
            file_info_class_lvl,
        })
    }
}

/// [2.4] File Information Classes \[MS-FSCC\]
///
/// [2.4]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/4718fc40-e539-4014-8e33-b675af74e3e1
#[derive(PartialEq, Eq, Clone)]
pub struct FileInformationClassLevel(u32);

impl FileInformationClassLevel {
    /// FileBasicInformation
    pub const FILE_BASIC_INFORMATION: Self = Self(4);
    /// FileStandardInformation
    pub const FILE_STANDARD_INFORMATION: Self = Self(5);
    /// FileAttributeTagInformation
    pub const FILE_ATTRIBUTE_TAG_INFORMATION: Self = Self(35);
    /// FileDirectoryInformation
    pub const FILE_DIRECTORY_INFORMATION: Self = Self(1);
    /// FileFullDirectoryInformation
    pub const FILE_FULL_DIRECTORY_INFORMATION: Self = Self(2);
    /// FileBothDirectoryInformation
    pub const FILE_BOTH_DIRECTORY_INFORMATION: Self = Self(3);
    /// FileNamesInformation
    pub const FILE_NAMES_INFORMATION: Self = Self(12);
    /// FileEndOfFileInformation
    pub const FILE_END_OF_FILE_INFORMATION: Self = Self(20);
    /// FileDispositionInformation
    pub const FILE_DISPOSITION_INFORMATION: Self = Self(13);
    /// FileRenameInformation
    pub const FILE_RENAME_INFORMATION: Self = Self(10);
    /// FileAllocationInformation
    pub const FILE_ALLOCATION_INFORMATION: Self = Self(19);
}

impl Display for FileInformationClassLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            FileInformationClassLevel::FILE_BASIC_INFORMATION => write!(f, "FileBasicInformation"),
            FileInformationClassLevel::FILE_STANDARD_INFORMATION => write!(f, "FileStandardInformation"),
            FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION => write!(f, "FileAttributeTagInformation"),
            FileInformationClassLevel::FILE_DIRECTORY_INFORMATION => write!(f, "FileDirectoryInformation"),
            FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION => write!(f, "FileFullDirectoryInformation"),
            FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION => write!(f, "FileBothDirectoryInformation"),
            FileInformationClassLevel::FILE_NAMES_INFORMATION => write!(f, "FileNamesInformation"),
            FileInformationClassLevel::FILE_END_OF_FILE_INFORMATION => write!(f, "FileEndOfFileInformation"),
            FileInformationClassLevel::FILE_DISPOSITION_INFORMATION => write!(f, "FileDispositionInformation"),
            FileInformationClassLevel::FILE_RENAME_INFORMATION => write!(f, "FileRenameInformation"),
            FileInformationClassLevel::FILE_ALLOCATION_INFORMATION => write!(f, "FileAllocationInformation"),
            _ => write!(f, "FileInformationClassLevel({})", self.0),
        }
    }
}

impl Debug for FileInformationClassLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            FileInformationClassLevel::FILE_BASIC_INFORMATION => write!(f, "FileBasicInformation"),
            FileInformationClassLevel::FILE_STANDARD_INFORMATION => write!(f, "FileStandardInformation"),
            FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION => write!(f, "FileAttributeTagInformation"),
            FileInformationClassLevel::FILE_DIRECTORY_INFORMATION => write!(f, "FileDirectoryInformation"),
            FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION => write!(f, "FileFullDirectoryInformation"),
            FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION => write!(f, "FileBothDirectoryInformation"),
            FileInformationClassLevel::FILE_NAMES_INFORMATION => write!(f, "FileNamesInformation"),
            _ => write!(f, "FileInformationClassLevel({})", self.0),
        }
    }
}

impl From<u32> for FileInformationClassLevel {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<FileInformationClassLevel> for u32 {
    fn from(file_info_class_lvl: FileInformationClassLevel) -> Self {
        file_info_class_lvl.0
    }
}

/// [2.2.3.4.8] Client Drive Query Information Response (DR_DRIVE_QUERY_INFORMATION_RSP)
///
/// [2.2.3.4.8]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/37ef4fb1-6a95-4200-9fbf-515464f034a4
#[derive(Debug)]
pub struct ClientDriveQueryInformationResponse {
    pub device_io_response: DeviceIoResponse,
    /// If [`Self::device_io_response`] has an `io_status` besides [`NtStatus::SUCCESS`],
    /// this field can be omitted (set to `None`).
    pub buffer: Option<FileInformationClass>,
}

impl ClientDriveQueryInformationResponse {
    const NAME: &'static str = "DR_DRIVE_QUERY_INFORMATION_RSP";

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.device_io_response.encode(dst)?;
        if let Some(buffer) = &self.buffer {
            dst.write_u32(cast_length!(
                "ClientDriveQueryInformationResponse",
                "buffer.size()",
                buffer.size()
            )?);
            buffer.encode(dst)?;
        } else {
            dst.write_u32(0); // Length = 0
        }
        Ok(())
    }

    pub fn size(&self) -> usize {
        self.device_io_response.size() // DeviceIoResponse
        + 4 // Length
        + if let Some(buffer) = &self.buffer {
            buffer.size() // Buffer
        } else {
            0
        }
    }
}

/// [2.4] File Information Classes \[MS-FSCC\]
///
/// [2.4]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/4718fc40-e539-4014-8e33-b675af74e3e1
#[derive(Debug, Clone)]
pub enum FileInformationClass {
    Basic(FileBasicInformation),
    Standard(FileStandardInformation),
    AttributeTag(FileAttributeTagInformation),
    BothDirectory(FileBothDirectoryInformation),
    FullDirectory(FileFullDirectoryInformation),
    Names(FileNamesInformation),
    Directory(FileDirectoryInformation),
    EndOfFile(FileEndOfFileInformation),
    Disposition(FileDispositionInformation),
    Rename(FileRenameInformation),
    Allocation(FileAllocationInformation),
}

impl FileInformationClass {
    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Basic(f) => f.encode(dst),
            Self::Standard(f) => f.encode(dst),
            Self::AttributeTag(f) => f.encode(dst),
            Self::BothDirectory(f) => f.encode(dst),
            Self::FullDirectory(f) => f.encode(dst),
            Self::Names(f) => f.encode(dst),
            Self::Directory(f) => f.encode(dst),
            _ => Err(unsupported_pdu_err!(
                "FileInformationClass::encode",
                "FileInformationClass",
                self.to_string()
            )),
        }
    }

    pub fn decode(
        file_info_class_level: FileInformationClassLevel,
        length: usize,
        src: &mut ReadCursor<'_>,
    ) -> PduResult<Self> {
        match file_info_class_level {
            FileInformationClassLevel::FILE_BASIC_INFORMATION => Ok(FileBasicInformation::decode(src)?.into()),
            FileInformationClassLevel::FILE_END_OF_FILE_INFORMATION => {
                Ok(FileEndOfFileInformation::decode(src)?.into())
            }
            FileInformationClassLevel::FILE_DISPOSITION_INFORMATION => {
                Ok(FileDispositionInformation::decode(src, length)?.into())
            }
            FileInformationClassLevel::FILE_RENAME_INFORMATION => Ok(FileRenameInformation::decode(src)?.into()),
            FileInformationClassLevel::FILE_ALLOCATION_INFORMATION => {
                Ok(FileAllocationInformation::decode(src)?.into())
            }
            _ => Err(unsupported_pdu_err!(
                "FileInformationClass::decode",
                "FileInformationClassLevel",
                file_info_class_level.to_string()
            )),
        }
    }

    pub fn size(&self) -> usize {
        match self {
            Self::Basic(_) => FileBasicInformation::size(),
            Self::Standard(_) => FileStandardInformation::size(),
            Self::AttributeTag(_) => FileAttributeTagInformation::size(),
            Self::BothDirectory(f) => f.size(),
            Self::FullDirectory(f) => f.size(),
            Self::Names(f) => f.size(),
            Self::Directory(f) => f.size(),
            Self::EndOfFile(_) => FileEndOfFileInformation::size(),
            Self::Disposition(_) => FileDispositionInformation::size(),
            Self::Rename(f) => f.size(),
            Self::Allocation(_) => FileAllocationInformation::size(),
        }
    }
}

impl Display for FileInformationClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Basic(_) => write!(f, "FileBasicInformation"),
            Self::Standard(_) => write!(f, "FileStandardInformation"),
            Self::AttributeTag(_) => write!(f, "FileAttributeTagInformation"),
            Self::BothDirectory(_) => write!(f, "FileBothDirectoryInformation"),
            Self::FullDirectory(_) => write!(f, "FileFullDirectoryInformation"),
            Self::Names(_) => write!(f, "FileNamesInformation"),
            Self::Directory(_) => write!(f, "FileDirectoryInformation"),
            Self::EndOfFile(_) => write!(f, "FileEndOfFileInformation"),
            Self::Disposition(_) => write!(f, "FileDispositionInformation"),
            Self::Rename(_) => write!(f, "FileRenameInformation"),
            Self::Allocation(_) => write!(f, "FileAllocationInformation"),
        }
    }
}

impl From<FileBasicInformation> for FileInformationClass {
    fn from(f: FileBasicInformation) -> Self {
        Self::Basic(f)
    }
}

impl From<FileStandardInformation> for FileInformationClass {
    fn from(f: FileStandardInformation) -> Self {
        Self::Standard(f)
    }
}

impl From<FileAttributeTagInformation> for FileInformationClass {
    fn from(f: FileAttributeTagInformation) -> Self {
        Self::AttributeTag(f)
    }
}

impl From<FileBothDirectoryInformation> for FileInformationClass {
    fn from(f: FileBothDirectoryInformation) -> Self {
        Self::BothDirectory(f)
    }
}

impl From<FileFullDirectoryInformation> for FileInformationClass {
    fn from(f: FileFullDirectoryInformation) -> Self {
        Self::FullDirectory(f)
    }
}

impl From<FileNamesInformation> for FileInformationClass {
    fn from(f: FileNamesInformation) -> Self {
        Self::Names(f)
    }
}

impl From<FileDirectoryInformation> for FileInformationClass {
    fn from(f: FileDirectoryInformation) -> Self {
        Self::Directory(f)
    }
}

impl From<FileEndOfFileInformation> for FileInformationClass {
    fn from(f: FileEndOfFileInformation) -> Self {
        Self::EndOfFile(f)
    }
}

impl From<FileDispositionInformation> for FileInformationClass {
    fn from(f: FileDispositionInformation) -> Self {
        Self::Disposition(f)
    }
}

impl From<FileRenameInformation> for FileInformationClass {
    fn from(f: FileRenameInformation) -> Self {
        Self::Rename(f)
    }
}

impl From<FileAllocationInformation> for FileInformationClass {
    fn from(f: FileAllocationInformation) -> Self {
        Self::Allocation(f)
    }
}

/// [2.4.7] FileBasicInformation \[MS-FSCC\]
///
/// [2.4.7]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/16023025-8a78-492f-8b96-c873b042ac50
#[derive(Debug, Clone)]
pub struct FileBasicInformation {
    pub creation_time: i64,
    pub last_access_time: i64,
    pub last_write_time: i64,
    pub change_time: i64,
    pub file_attributes: FileAttributes,
    // NOTE: The `reserved` field in the spec MUST not be serialized and sent over RDP, or it will break the server implementation.
    // FreeRDP does the same: https://github.com/FreeRDP/FreeRDP/blob/1adb263813ca2e76a893ef729a04db8f94b5d757/channels/drive/client/drive_file.c#L508
}

impl FileBasicInformation {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(ctx: "FileBasicInformation", in: src, size: Self::size());
        let creation_time = src.read_i64();
        let last_access_time = src.read_i64();
        let last_write_time = src.read_i64();
        let change_time = src.read_i64();
        let file_attributes = FileAttributes::from_bits_retain(src.read_u32());
        Ok(Self {
            creation_time,
            last_access_time,
            last_write_time,
            change_time,
            file_attributes,
        })
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_i64(self.creation_time);
        dst.write_i64(self.last_access_time);
        dst.write_i64(self.last_write_time);
        dst.write_i64(self.change_time);
        dst.write_u32(self.file_attributes.bits());
        Ok(())
    }

    pub fn size() -> usize {
        8 // CreationTime
        + 8 // LastAccessTime
        + 8 // LastWriteTime
        + 8 // ChangeTime
        + 4 // FileAttributes
    }
}

/// [2.4.41] FileStandardInformation \[MS-FSCC\]
///
/// [2.4.41]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/5afa7f66-619c-48f3-955f-68c4ece704ae
#[derive(Debug, Clone)]
pub struct FileStandardInformation {
    pub allocation_size: i64,
    pub end_of_file: i64,
    pub number_of_links: u32,
    /// Set to TRUE to indicate that a file deletion has been requested; set to FALSE
    /// otherwise.
    pub delete_pending: Boolean,
    /// Set to TRUE to indicate that the file is a directory; set to FALSE otherwise.
    pub directory: Boolean,
    // NOTE: `reserved` field omitted.
}

impl FileStandardInformation {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_i64(self.allocation_size);
        dst.write_i64(self.end_of_file);
        dst.write_u32(self.number_of_links);
        dst.write_u8(self.delete_pending.into());
        dst.write_u8(self.directory.into());
        Ok(())
    }

    pub fn size() -> usize {
        8 // AllocationSize
        + 8 // EndOfFile
        + 4 // NumberOfLinks
        + 1 // DeletePending
        + 1 // Directory
    }
}

/// [2.1.8] Boolean
///
/// [2.1.8]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/8ce7b38c-d3cc-415d-ab39-944000ea77ff
#[derive(Debug, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum Boolean {
    True = 1,
    False = 0,
}

impl From<Boolean> for u8 {
    fn from(boolean: Boolean) -> Self {
        match boolean {
            Boolean::True => 1,
            Boolean::False => 0,
        }
    }
}

impl From<u8> for Boolean {
    fn from(value: u8) -> Self {
        match value {
            1 => Boolean::True,
            _ => Boolean::False,
        }
    }
}

/// [2.4.6] FileAttributeTagInformation \[MS-FSCC\]
///
/// [2.4.6]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/d295752f-ce89-4b98-8553-266d37c84f0e?redirectedfrom=MSDN
#[derive(Debug, Clone)]
pub struct FileAttributeTagInformation {
    pub file_attributes: FileAttributes,
    pub reparse_tag: u32,
}

impl FileAttributeTagInformation {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_u32(self.file_attributes.bits());
        dst.write_u32(self.reparse_tag);
        Ok(())
    }

    fn size() -> usize {
        4 // FileAttributes
        + 4 // ReparseTag
    }
}

/// [2.4.8] FileBothDirectoryInformation \[MS-FSCC\]
///
/// [2.4.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/270df317-9ba5-4ccb-ba00-8d22be139bc5
#[derive(Debug, Clone)]
pub struct FileBothDirectoryInformation {
    pub next_entry_offset: u32,
    pub file_index: u32,
    pub creation_time: i64,
    pub last_access_time: i64,
    pub last_write_time: i64,
    pub change_time: i64,
    pub end_of_file: i64,
    pub allocation_size: i64,
    pub file_attributes: FileAttributes,
    pub ea_size: u32,
    pub short_name_length: i8,
    // reserved: u8: MUST NOT be added,
    // see https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_file.c#L907
    pub short_name: [u8; 24], // 24 bytes
    pub file_name: String,
}

impl FileBothDirectoryInformation {
    pub fn new(
        creation_time: i64,
        last_access_time: i64,
        last_write_time: i64,
        change_time: i64,
        file_size: i64,
        file_attributes: FileAttributes,
        file_name: String,
    ) -> Self {
        // Default field values taken from
        // https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_file.c#L871
        Self {
            next_entry_offset: 0,
            file_index: 0,
            creation_time,
            last_access_time,
            last_write_time,
            change_time,
            end_of_file: file_size,
            allocation_size: file_size,
            file_attributes,
            ea_size: 0,
            short_name_length: 0,
            short_name: [0; 24],
            file_name,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.next_entry_offset);
        dst.write_u32(self.file_index);
        dst.write_i64(self.creation_time);
        dst.write_i64(self.last_access_time);
        dst.write_i64(self.last_write_time);
        dst.write_i64(self.change_time);
        dst.write_i64(self.end_of_file);
        dst.write_i64(self.allocation_size);
        dst.write_u32(self.file_attributes.bits());
        dst.write_u32(cast_length!(
            "FileBothDirectoryInformation::encode",
            "file_name_length",
            encoded_str_len(&self.file_name, CharacterSet::Unicode, false)
        )?);
        dst.write_u32(self.ea_size);
        dst.write_i8(self.short_name_length);
        // reserved u8 MUST NOT be added,
        // see https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_file.c#L907
        dst.write_slice(&self.short_name);
        write_string_to_cursor(dst, &self.file_name, CharacterSet::Unicode, false)?;
        Ok(())
    }

    fn size(&self) -> usize {
        4 // NextEntryOffset
        + 4 // FileIndex
        + 8 // CreationTime
        + 8 // LastAccessTime
        + 8 // LastWriteTime
        + 8 // ChangeTime
        + 8 // EndOfFile
        + 8 // AllocationSize
        + 4 // FileAttributes
        + 4 // FileNameLength
        + 4 // EaSize
        + 1 // ShortNameLength
        + 24 // ShortName
        + encoded_str_len(&self.file_name, CharacterSet::Unicode, false)
    }
}

/// [2.4.14] FileFullDirectoryInformation \[MS-FSCC\]
///
/// [2.4.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/e8d926d1-3a22-4654-be9c-58317a85540b
#[derive(Debug, Clone)]
pub struct FileFullDirectoryInformation {
    pub next_entry_offset: u32,
    pub file_index: u32,
    pub creation_time: i64,
    pub last_access_time: i64,
    pub last_write_time: i64,
    pub change_time: i64,
    pub end_of_file: i64,
    pub allocation_size: i64,
    pub file_attributes: FileAttributes,
    pub ea_size: u32,
    pub file_name: String,
}

impl FileFullDirectoryInformation {
    pub fn new(
        creation_time: i64,
        last_access_time: i64,
        last_write_time: i64,
        change_time: i64,
        file_size: i64,
        file_attributes: FileAttributes,
        file_name: String,
    ) -> Self {
        // Default field values taken from
        // https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_file.c#L871
        Self {
            next_entry_offset: 0,
            file_index: 0,
            creation_time,
            last_access_time,
            last_write_time,
            change_time,
            end_of_file: file_size,
            allocation_size: file_size,
            file_attributes,
            ea_size: 0,
            file_name,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.next_entry_offset);
        dst.write_u32(self.file_index);
        dst.write_i64(self.creation_time);
        dst.write_i64(self.last_access_time);
        dst.write_i64(self.last_write_time);
        dst.write_i64(self.change_time);
        dst.write_i64(self.end_of_file);
        dst.write_i64(self.allocation_size);
        dst.write_u32(self.file_attributes.bits());
        dst.write_u32(cast_length!(
            "FileFullDirectoryInformation::encode",
            "file_name_length",
            encoded_str_len(&self.file_name, CharacterSet::Unicode, false)
        )?);
        dst.write_u32(self.ea_size);
        write_string_to_cursor(dst, &self.file_name, CharacterSet::Unicode, false)?;
        Ok(())
    }

    fn size(&self) -> usize {
        4 // NextEntryOffset
        + 4 // FileIndex
        + 8 // CreationTime
        + 8 // LastAccessTime
        + 8 // LastWriteTime
        + 8 // ChangeTime
        + 8 // EndOfFile
        + 8 // AllocationSize
        + 4 // FileAttributes
        + 4 // FileNameLength
        + 4 // EaSize
        + encoded_str_len(&self.file_name, CharacterSet::Unicode, false)
    }
}

/// [2.4.28] FileNamesInformation \[MS-FSCC\]
///
/// [2.4.28]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/a289f7a8-83d2-4927-8c88-b2d328dde5a5?redirectedfrom=MSDN
#[derive(Debug, Clone)]
pub struct FileNamesInformation {
    pub next_entry_offset: u32,
    pub file_index: u32,
    pub file_name: String,
}

impl FileNamesInformation {
    pub fn new(file_name: String) -> Self {
        // Default field values taken from
        // https://github.com/FreeRDP/FreeRDP/blob/dfa231c0a55b005af775b833f92f6bcd30363d77/channels/drive/client/drive_file.c#L912
        Self {
            next_entry_offset: 0,
            file_index: 0,
            file_name,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.next_entry_offset);
        dst.write_u32(self.file_index);
        dst.write_u32(cast_length!(
            "FileNamesInformation::encode",
            "file_name_length",
            encoded_str_len(&self.file_name, CharacterSet::Unicode, false)
        )?);
        write_string_to_cursor(dst, &self.file_name, CharacterSet::Unicode, false)?;
        Ok(())
    }

    fn size(&self) -> usize {
        4 // NextEntryOffset
        + 4 // FileIndex
        + 4 // FileNameLength
        + encoded_str_len(&self.file_name, CharacterSet::Unicode, false)
    }
}

/// [2.4.10] FileDirectoryInformation \[MS-FSCC\]
///
/// [2.4.10]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/b38bf518-9057-4c88-9ddd-5e2d3976a64b
#[derive(Debug, Clone)]
pub struct FileDirectoryInformation {
    pub next_entry_offset: u32,
    pub file_index: u32,
    pub creation_time: i64,
    pub last_access_time: i64,
    pub last_write_time: i64,
    pub change_time: i64,
    pub end_of_file: i64,
    pub allocation_size: i64,
    pub file_attributes: FileAttributes,
    pub file_name: String,
}

impl FileDirectoryInformation {
    pub fn new(
        creation_time: i64,
        last_access_time: i64,
        last_write_time: i64,
        change_time: i64,
        file_size: i64,
        file_attributes: FileAttributes,
        file_name: String,
    ) -> Self {
        // Default field values taken from
        // https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_file.c#L796
        Self {
            next_entry_offset: 0,
            file_index: 0,
            creation_time,
            last_access_time,
            last_write_time,
            change_time,
            end_of_file: file_size,
            allocation_size: file_size,
            file_attributes,
            file_name,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.next_entry_offset);
        dst.write_u32(self.file_index);
        dst.write_i64(self.creation_time);
        dst.write_i64(self.last_access_time);
        dst.write_i64(self.last_write_time);
        dst.write_i64(self.change_time);
        dst.write_i64(self.end_of_file);
        dst.write_i64(self.allocation_size);
        dst.write_u32(self.file_attributes.bits());
        dst.write_u32(cast_length!(
            "FileDirectoryInformation::encode",
            "file_name_length",
            encoded_str_len(&self.file_name, CharacterSet::Unicode, false)
        )?);
        write_string_to_cursor(dst, &self.file_name, CharacterSet::Unicode, false)?;
        Ok(())
    }

    fn size(&self) -> usize {
        4 // NextEntryOffset
        + 4 // FileIndex
        + 8 // CreationTime
        + 8 // LastAccessTime
        + 8 // LastWriteTime
        + 8 // ChangeTime
        + 8 // EndOfFile
        + 8 // AllocationSize
        + 4 // FileAttributes
        + 4 // FileNameLength
        + encoded_str_len(&self.file_name, CharacterSet::Unicode, false)
    }
}

/// [2.2.1.4.2] Device Close Request (DR_CLOSE_REQ)
///
/// [2.2.1.4.2]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/3ec6627f-9e0f-4941-a828-3fc6ed63d9e7
#[derive(Debug)]
pub struct DeviceCloseRequest {
    pub device_io_request: DeviceIoRequest,
    // Padding (32 bytes): ignored as per FreeRDP:
    // https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_main.c#L236
}

impl DeviceCloseRequest {
    pub fn decode(dev_io_req: DeviceIoRequest) -> Self {
        Self {
            device_io_request: dev_io_req,
        }
    }
}

/// [2.2.1.5.2] Device Close Response (DR_CLOSE_RSP)
///
/// [2.2.1.5.2]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/0dae7031-cfd8-4f14-908c-ec06e14997b5
#[derive(Debug)]
pub struct DeviceCloseResponse {
    pub device_io_response: DeviceIoResponse,
    // Padding (4 bytes):  An array of 4 bytes. Reserved. This field can be set to any value and MUST be ignored.
}

impl DeviceCloseResponse {
    const NAME: &'static str = "DR_CLOSE_RSP";

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.device_io_response.encode(dst)?;
        dst.write_u32(0); // Padding
        Ok(())
    }

    pub fn size(&self) -> usize {
        self.device_io_response.size() // DeviceIoResponse
        + 4 // Padding
    }
}

/// [2.2.3.3.10] Server Drive Query Directory Request (DR_DRIVE_QUERY_DIRECTORY_REQ)
///
/// [2.2.3.3.10]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/458019d2-5d5a-4fd4-92ef-8c05f8d7acb1
#[derive(Debug)]
pub struct ServerDriveQueryDirectoryRequest {
    pub device_io_request: DeviceIoRequest,
    pub file_info_class_lvl: FileInformationClassLevel,
    pub initial_query: u8,
    pub path: String,
}

impl ServerDriveQueryDirectoryRequest {
    const FIXED_PART_SIZE: usize = 4 /* FsInformationClass */ + 1 /* InitialQuery */ + 4 /* PathLength */ + 23 /* Padding */;

    fn decode(device_io_request: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let file_info_class_lvl = FileInformationClassLevel::from(src.read_u32());

        // This field MUST contain one of the following values
        match file_info_class_lvl {
            FileInformationClassLevel::FILE_DIRECTORY_INFORMATION
            | FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION
            | FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION
            | FileInformationClassLevel::FILE_NAMES_INFORMATION => {}
            _ => {
                return Err(invalid_message_err!(
                    "ServerDriveQueryDirectoryRequest::decode",
                    "file_info_class_lvl",
                    "received invalid level"
                ))
            }
        }

        let initial_query = src.read_u8();
        let path_length = cast_length!("ServerDriveQueryDirectoryRequest", "path_length", src.read_u32())?;
        // Padding (23 bytes): An array of 23 bytes. This field is unused and MUST be ignored.
        read_padding!(src, 23);

        ensure_size!(in: src, size: path_length);
        let path = decode_string(src.read_slice(path_length), CharacterSet::Unicode, true)?;

        Ok(Self {
            device_io_request,
            file_info_class_lvl,
            initial_query,
            path,
        })
    }
}

/// 2.2.3.3.11 Server Drive NotifyChange Directory Request (DR_DRIVE_NOTIFY_CHANGE_DIRECTORY_REQ)
///
/// [2.2.3.3.11]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/ed05e73d-e53e-4261-a1e1-365a70ba6512
#[derive(Debug)]
pub struct ServerDriveNotifyChangeDirectoryRequest {
    pub device_io_request: DeviceIoRequest,
    pub watch_tree: u8,
    pub completion_filter: u32,
}

impl ServerDriveNotifyChangeDirectoryRequest {
    const FIXED_PART_SIZE: usize = 1 /* WatchTree */ + 4 /* CompletionFilter */ + 27 /* Padding */;

    fn decode(device_io_request: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let watch_tree = src.read_u8();
        let completion_filter = src.read_u32();
        // Padding (27 bytes): An array of 27 bytes. This field is unused and MUST be ignored.
        read_padding!(src, 27);

        Ok(Self {
            device_io_request,
            watch_tree,
            completion_filter,
        })
    }
}

/// [2.2.3.4.10] Client Drive Query Directory Response (DR_DRIVE_QUERY_DIRECTORY_RSP)
///
/// [2.2.3.4.10]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/9c929407-a833-4893-8f20-90c984756140
#[derive(Debug)]
pub struct ClientDriveQueryDirectoryResponse {
    pub device_io_reply: DeviceIoResponse,
    pub buffer: Option<FileInformationClass>,
}

impl ClientDriveQueryDirectoryResponse {
    const NAME: &'static str = "DR_DRIVE_QUERY_DIRECTORY_RSP";

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.device_io_reply.encode(dst)?;
        dst.write_u32(cast_length!(
            "ClientDriveQueryDirectoryResponse",
            "length",
            if self.buffer.is_some() {
                self.buffer.as_ref().unwrap().size()
            } else {
                0
            }
        )?);
        if let Some(buffer) = &self.buffer {
            buffer.encode(dst)?;
        } else {
            write_padding!(dst, 1) // Padding: https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_file.c#L937
        }
        Ok(())
    }

    pub fn size(&self) -> usize {
        self.device_io_reply.size() // DeviceIoResponse
        + 4 // Length
        + if let Some(buffer) = &self.buffer {
            buffer.size() // Buffer
        } else {
            1 // Padding: https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_file.c#L937
        }
    }
}

/// [2.2.3.3.6] Server Drive Query Volume Information Request
///
/// We only need to read the buffer up to the FileInformationClass to get the job done, so the rest of the fields in
/// this structure are discarded. See FreeRDP:
/// https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_main.c#L464
///
/// [2.2.3.3.6]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/484e622d-0e2b-423c-8461-7de38878effb
#[derive(Debug)]
pub struct ServerDriveQueryVolumeInformationRequest {
    pub device_io_request: DeviceIoRequest,
    pub fs_info_class_lvl: FileSystemInformationClassLevel,
}

impl ServerDriveQueryVolumeInformationRequest {
    const FIXED_PART_SIZE: usize = 4 /* FsInformationClass */ + 4 /* Length */ + 24 /* Padding */;

    pub fn decode(dev_io_req: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let fs_info_class_lvl = FileSystemInformationClassLevel::from(src.read_u32());

        // This field MUST contain one of the following values.
        match fs_info_class_lvl {
            FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION
            | FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION
            | FileSystemInformationClassLevel::FILE_FS_ATTRIBUTE_INFORMATION
            | FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION
            | FileSystemInformationClassLevel::FILE_FS_DEVICE_INFORMATION => {}
            _ => {
                return Err(invalid_message_err!(
                    "ServerDriveQueryVolumeInformationRequest::decode",
                    "fs_info_class_lvl",
                    "received invalid level"
                ))
            }
        }

        // We only need to read the buffer up to the FileInformationClass to get the job done, so the rest of the fields in
        // this structure are discarded. See FreeRDP:
        // https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_main.c#L464
        let length = cast_length!("ServerDriveQueryVolumeInformationRequest", "length", src.read_u32())?; // Length
        read_padding!(src, 24); // Padding
        ensure_size!(in: src, size: length);
        read_padding!(src, length); // QueryVolumeBuffer

        Ok(Self {
            device_io_request: dev_io_req,
            fs_info_class_lvl,
        })
    }
}

/// [2.5] File System Information Classes [MS-FSCC]
///
/// [2.5] https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/ee12042a-9352-46e3-9f67-c094b75fe6c3
#[derive(Debug, PartialEq, Eq)]
pub struct FileSystemInformationClassLevel(u32);

impl FileSystemInformationClassLevel {
    /// FileFsVolumeInformation
    pub const FILE_FS_VOLUME_INFORMATION: Self = Self(1);
    /// FileFsLabelInformation
    pub const FILE_FS_LABEL_INFORMATION: Self = Self(2);
    /// FileFsSizeInformation
    pub const FILE_FS_SIZE_INFORMATION: Self = Self(3);
    /// FileFsDeviceInformation
    pub const FILE_FS_DEVICE_INFORMATION: Self = Self(4);
    /// FileFsAttributeInformation
    pub const FILE_FS_ATTRIBUTE_INFORMATION: Self = Self(5);
    /// FileFsControlInformation
    pub const FILE_FS_CONTROL_INFORMATION: Self = Self(6);
    /// FileFsFullSizeInformation
    pub const FILE_FS_FULL_SIZE_INFORMATION: Self = Self(7);
    /// FileFsObjectIdInformation
    pub const FILE_FS_OBJECT_ID_INFORMATION: Self = Self(8);
    /// FileFsDriverPathInformation
    pub const FILE_FS_DRIVER_PATH_INFORMATION: Self = Self(9);
    /// FileFsVolumeFlagsInformation
    pub const FILE_FS_VOLUME_FLAGS_INFORMATION: Self = Self(10);
    /// FileFsSectorSizeInformation
    pub const FILE_FS_SECTOR_SIZE_INFORMATION: Self = Self(11);
}

impl From<u32> for FileSystemInformationClassLevel {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

/// [2.5] File System Information Classes
///
/// [2.5]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/ee12042a-9352-46e3-9f67-c094b75fe6c3
#[derive(Debug)]
pub enum FileSystemInformationClass {
    FileFsVolumeInformation(FileFsVolumeInformation),
    FileFsSizeInformation(FileFsSizeInformation),
    FileFsAttributeInformation(FileFsAttributeInformation),
    FileFsFullSizeInformation(FileFsFullSizeInformation),
    FileFsDeviceInformation(FileFsDeviceInformation),
}

impl FileSystemInformationClass {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::FileFsVolumeInformation(f) => f.encode(dst),
            Self::FileFsSizeInformation(f) => f.encode(dst),
            Self::FileFsAttributeInformation(f) => f.encode(dst),
            Self::FileFsFullSizeInformation(f) => f.encode(dst),
            Self::FileFsDeviceInformation(f) => f.encode(dst),
        }
    }

    fn size(&self) -> usize {
        match self {
            Self::FileFsVolumeInformation(f) => f.size(),
            Self::FileFsSizeInformation(f) => f.size(),
            Self::FileFsAttributeInformation(f) => f.size(),
            Self::FileFsFullSizeInformation(_) => FileFsFullSizeInformation::size(),
            Self::FileFsDeviceInformation(_) => FileFsDeviceInformation::size(),
        }
    }
}

impl From<FileFsVolumeInformation> for FileSystemInformationClass {
    fn from(file_fs_vol_info: FileFsVolumeInformation) -> Self {
        Self::FileFsVolumeInformation(file_fs_vol_info)
    }
}

impl From<FileFsSizeInformation> for FileSystemInformationClass {
    fn from(file_fs_vol_info: FileFsSizeInformation) -> Self {
        Self::FileFsSizeInformation(file_fs_vol_info)
    }
}

impl From<FileFsAttributeInformation> for FileSystemInformationClass {
    fn from(file_fs_vol_info: FileFsAttributeInformation) -> Self {
        Self::FileFsAttributeInformation(file_fs_vol_info)
    }
}

impl From<FileFsFullSizeInformation> for FileSystemInformationClass {
    fn from(file_fs_vol_info: FileFsFullSizeInformation) -> Self {
        Self::FileFsFullSizeInformation(file_fs_vol_info)
    }
}

impl From<FileFsDeviceInformation> for FileSystemInformationClass {
    fn from(file_fs_vol_info: FileFsDeviceInformation) -> Self {
        Self::FileFsDeviceInformation(file_fs_vol_info)
    }
}

/// [2.5.9] FileFsVolumeInformation
///
/// [2.5.9]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/bf691378-c34e-4a13-976e-404ea1a87738
#[derive(Debug)]
pub struct FileFsVolumeInformation {
    pub volume_creation_time: i64,
    pub volume_serial_number: u32,
    pub supports_objects: Boolean,
    // reserved is omitted
    // https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_main.c#L495
    pub volume_label: String,
}

impl FileFsVolumeInformation {
    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_i64(self.volume_creation_time);
        dst.write_u32(self.volume_serial_number);
        dst.write_u32(cast_length!(
            "FileFsVolumeInformation::encode",
            "volume_label_length",
            encoded_str_len(&self.volume_label, CharacterSet::Unicode, true)
        )?);
        dst.write_u8(self.supports_objects.into());
        write_string_to_cursor(dst, &self.volume_label, CharacterSet::Unicode, true)?;
        Ok(())
    }

    pub fn size(&self) -> usize {
        8 // VolumeCreationTime
        + 4 // VolumeSerialNumber
        + 4 // VolumeLabelLength
        + 1 // SupportsObjects
        + encoded_str_len(&self.volume_label, CharacterSet::Unicode, true)
    }
}

/// [2.5.8] FileFsSizeInformation
///
/// [2.5.8]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/e13e068c-e3a7-4dd4-94fd-3892b492e6e7
#[derive(Debug)]
pub struct FileFsSizeInformation {
    pub total_alloc_units: i64,
    pub available_alloc_units: i64,
    pub sectors_per_alloc_unit: u32,
    pub bytes_per_sector: u32,
}

impl FileFsSizeInformation {
    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_i64(self.total_alloc_units);
        dst.write_i64(self.available_alloc_units);
        dst.write_u32(self.sectors_per_alloc_unit);
        dst.write_u32(self.bytes_per_sector);
        Ok(())
    }

    pub fn size(&self) -> usize {
        8 // TotalAllocationUnits
        + 8 // AvailableAllocationUnits
        + 4 // SectorsPerAllocationUnit
        + 4 // BytesPerSector
    }
}

/// [2.5.1] FileFsAttributeInformation
///
/// [2.5.1]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/ebc7e6e5-4650-4e54-b17c-cf60f6fbeeaa
#[derive(Debug)]
pub struct FileFsAttributeInformation {
    pub file_system_attributes: FileSystemAttributes,
    pub max_component_name_len: u32,
    pub file_system_name: String,
}

impl FileFsAttributeInformation {
    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.file_system_attributes.bits());
        dst.write_u32(self.max_component_name_len);
        dst.write_u32(cast_length!(
            "FileFsAttributeInformation::encode",
            "file_system_name_length",
            encoded_str_len(&self.file_system_name, CharacterSet::Unicode, true)
        )?);
        write_string_to_cursor(dst, &self.file_system_name, CharacterSet::Unicode, true)?;
        Ok(())
    }

    pub fn size(&self) -> usize {
        4 // FileSystemAttributes
        + 4 // MaximumComponentNameLength
        + 4 // FileSystemNameLength
        + encoded_str_len(&self.file_system_name, CharacterSet::Unicode, true)
    }
}

/// [2.5.4] FileFsFullSizeInformation
///
/// [2.5.4]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/63768db7-9012-4209-8cca-00781e7322f5
#[derive(Debug)]
pub struct FileFsFullSizeInformation {
    pub total_alloc_units: i64,
    pub caller_available_alloc_units: i64,
    pub actual_available_alloc_units: i64,
    pub sectors_per_alloc_unit: u32,
    pub bytes_per_sector: u32,
}

impl FileFsFullSizeInformation {
    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_i64(self.total_alloc_units);
        dst.write_i64(self.caller_available_alloc_units);
        dst.write_i64(self.actual_available_alloc_units);
        dst.write_u32(self.sectors_per_alloc_unit);
        dst.write_u32(self.bytes_per_sector);
        Ok(())
    }

    pub fn size() -> usize {
        8 // TotalAllocationUnits
        + 8 // CallerAvailableAllocationUnits
        + 8 // ActualAvailableAllocationUnits
        + 4 // SectorsPerAllocationUnit
        + 4 // BytesPerSector
    }
}

/// [2.5.10] FileFsDeviceInformation
///
/// [2.5.10]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/616b66d5-b335-4e1c-8f87-b4a55e8d3e4a
#[derive(Debug)]
pub struct FileFsDeviceInformation {
    pub device_type: u32,
    pub characteristics: Characteristics,
}

impl FileFsDeviceInformation {
    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_u32(self.device_type);
        dst.write_u32(self.characteristics.bits());
        Ok(())
    }

    pub fn size() -> usize {
        4 // DeviceType
        + 4 // Characteristics
    }
}

bitflags! {
    /// See [2.5.1] FileFsAttributeInformation.
    ///
    /// [2.5.1]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/ebc7e6e5-4650-4e54-b17c-cf60f6fbeeaa
    #[derive(Debug)]
    pub struct FileSystemAttributes: u32 {
        const FILE_SUPPORTS_USN_JOURNAL = 0x02000000;
        const FILE_SUPPORTS_OPEN_BY_FILE_ID = 0x01000000;
        const FILE_SUPPORTS_EXTENDED_ATTRIBUTES = 0x00800000;
        const FILE_SUPPORTS_HARD_LINKS = 0x00400000;
        const FILE_SUPPORTS_TRANSACTIONS = 0x00200000;
        const FILE_SEQUENTIAL_WRITE_ONCE = 0x00100000;
        const FILE_READ_ONLY_VOLUME = 0x00080000;
        const FILE_NAMED_STREAMS = 0x00040000;
        const FILE_SUPPORTS_ENCRYPTION = 0x00020000;
        const FILE_SUPPORTS_OBJECT_IDS = 0x00010000;
        const FILE_VOLUME_IS_COMPRESSED = 0x00008000;
        const FILE_SUPPORTS_REMOTE_STORAGE = 0x00000100;
        const FILE_SUPPORTS_REPARSE_POINTS = 0x00000080;
        const FILE_SUPPORTS_SPARSE_FILES = 0x00000040;
        const FILE_VOLUME_QUOTAS = 0x00000020;
        const FILE_FILE_COMPRESSION = 0x00000010;
        const FILE_PERSISTENT_ACLS = 0x00000008;
        const FILE_UNICODE_ON_DISK = 0x00000004;
        const FILE_CASE_PRESERVED_NAMES = 0x00000002;
        const FILE_CASE_SENSITIVE_SEARCH = 0x00000001;
        const FILE_SUPPORT_INTEGRITY_STREAMS = 0x04000000;
        const FILE_SUPPORTS_BLOCK_REFCOUNTING = 0x08000000;
        const FILE_SUPPORTS_SPARSE_VDL = 0x10000000;
    }
}

bitflags! {
    /// See [2.5.10] FileFsDeviceInformation.
    ///
    /// [2.5.10]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/616b66d5-b335-4e1c-8f87-b4a55e8d3e4a
    #[derive(Debug)]
    pub struct Characteristics: u32 {
        const FILE_REMOVABLE_MEDIA = 0x00000001;
        const FILE_READ_ONLY_DEVICE = 0x00000002;
        const FILE_FLOPPY_DISKETTE = 0x00000004;
        const FILE_WRITE_ONCE_MEDIA = 0x00000008;
        const FILE_REMOTE_DEVICE = 0x00000010;
        const FILE_DEVICE_IS_MOUNTED = 0x00000020;
        const FILE_VIRTUAL_VOLUME = 0x00000040;
        const FILE_DEVICE_SECURE_OPEN = 0x00000100;
        const FILE_CHARACTERISTIC_TS_DEVICE = 0x00001000;
        const FILE_CHARACTERISTIC_WEBDAV_DEVICE = 0x00002000;
        const FILE_DEVICE_ALLOW_APPCONTAINER_TRAVERSAL = 0x00020000;
        const FILE_PORTABLE_DEVICE = 0x0004000;
    }
}

/// [2.2.3.4.6] Client Drive Query Volume Information Response (DR_DRIVE_QUERY_VOLUME_INFORMATION_RSP)
///
/// [2.2.3.4.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/fbdc7db8-a268-4420-8b5e-ce689ad1d4ac
#[derive(Debug)]
pub struct ClientDriveQueryVolumeInformationResponse {
    pub device_io_reply: DeviceIoResponse,
    pub buffer: Option<FileSystemInformationClass>,
}

impl ClientDriveQueryVolumeInformationResponse {
    const NAME: &'static str = "DR_DRIVE_QUERY_VOLUME_INFORMATION_RSP";

    pub fn new(
        device_io_request: DeviceIoRequest,
        io_status: NtStatus,
        buffer: Option<FileSystemInformationClass>,
    ) -> Self {
        Self {
            device_io_reply: DeviceIoResponse::new(device_io_request, io_status),
            buffer,
        }
    }

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.device_io_reply.encode(dst)?;
        dst.write_u32(cast_length!(
            "ClientDriveQueryVolumeInformationResponse",
            "length",
            if self.buffer.is_some() {
                self.buffer.as_ref().unwrap().size()
            } else {
                0
            }
        )?);
        if let Some(buffer) = &self.buffer {
            buffer.encode(dst)?;
        }

        Ok(())
    }

    pub fn size(&self) -> usize {
        self.device_io_reply.size() // DeviceIoResponse
        + 4 // Length
        + if let Some(buffer) = &self.buffer {
            buffer.size() // Buffer
        } else {
            0
        }
    }
}

/// [2.2.1.4.3] Device Read Request (DR_READ_REQ)
///
/// [2.2.1.4.3]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/3192516d-36a6-47c5-987a-55c214aa0441
#[derive(Debug, Clone)]
pub struct DeviceReadRequest {
    pub device_io_request: DeviceIoRequest,
    pub length: u32,
    pub offset: u64,
}

impl DeviceReadRequest {
    const FIXED_PART_SIZE: usize = 4 /* Length */ + 8 /* Offset */ + 20 /* Padding */;

    pub fn decode(dev_io_req: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let length = src.read_u32();
        let offset = src.read_u64();
        // Padding (20 bytes):  An array of 20 bytes. Reserved. This field can be set to any value and MUST be ignored.
        read_padding!(src, 20);

        Ok(Self {
            device_io_request: dev_io_req,
            length,
            offset,
        })
    }
}

/// [2.2.1.5.3] Device Read Response (DR_READ_RSP)
///
/// [2.2.1.5.3]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/d35d3f91-fc5b-492b-80be-47f483ad1dc9
pub struct DeviceReadResponse {
    pub device_io_reply: DeviceIoResponse,
    pub read_data: Vec<u8>,
}

impl DeviceReadResponse {
    const NAME: &'static str = "DR_READ_RSP";

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.device_io_reply.encode(dst)?;
        dst.write_u32(cast_length!("DeviceReadResponse", "length", self.read_data.len())?);
        dst.write_slice(&self.read_data);
        Ok(())
    }

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn size(&self) -> usize {
        self.device_io_reply.size() // DeviceIoResponse
        + 4 // Length
        + self.read_data.len() // ReadData
    }
}

impl Debug for DeviceReadResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceReadResponse")
            .field("device_io_reply", &self.device_io_reply)
            .field("read_data", &format!("Vec<u8> of length {}", self.read_data.len()))
            .finish()
    }
}

/// [2.2.1.4.4] Device Write Request (DR_WRITE_REQ)
///
/// [2.2.1.4.4]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/2e25f0aa-a4ce-4ff3-ad62-ab6098280a3a
pub struct DeviceWriteRequest {
    pub device_io_request: DeviceIoRequest,
    pub offset: u64,
    pub write_data: Vec<u8>,
}

impl DeviceWriteRequest {
    const FIXED_PART_SIZE: usize = 4 /* Length */ + 8 /* Offset */ + 20 /* Padding */;

    pub fn decode(dev_io_req: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let length = cast_length!("DeviceWriteRequest", "length", src.read_u32())?;
        let offset = src.read_u64();
        // Padding (20 bytes):  An array of 20 bytes. Reserved. This field can be set to any value and MUST be ignored.
        read_padding!(src, 20);

        ensure_size!(in: src, size: length);
        let write_data = src.read_slice(length).to_vec();

        Ok(Self {
            device_io_request: dev_io_req,
            offset,
            write_data,
        })
    }
}

impl Debug for DeviceWriteRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceWriteRequest")
            .field("device_io_request", &self.device_io_request)
            .field("offset", &self.offset)
            .field("write_data", &format!("Vec<u8> of length {}", self.write_data.len()))
            .finish()
    }
}

/// [2.2.1.5.4] Device Write Response (DR_WRITE_RSP)
///
/// [2.2.1.5.4]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/58160a47-2379-4c4a-a99d-24a1a666c02a
#[derive(Debug)]
pub struct DeviceWriteResponse {
    pub device_io_reply: DeviceIoResponse,
    pub length: u32,
}

impl DeviceWriteResponse {
    const NAME: &'static str = "DR_WRITE_RSP";

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.device_io_reply.encode(dst)?;
        dst.write_u32(self.length);
        write_padding!(dst, 1); // Padding
        Ok(())
    }

    pub fn size(&self) -> usize {
        self.device_io_reply.size() // DeviceIoResponse
        + 4 // Length
        + 1 // Padding
    }
}

/// [2.2.3.3.9] Server Drive Set Information Request (DR_DRIVE_SET_INFORMATION_REQ)
///
/// [2.2.3.3.9]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/b5d3104b-0e42-4cf8-9059-e9fe86615e5c
#[derive(Debug, Clone)]
pub struct ServerDriveSetInformationRequest {
    pub device_io_request: DeviceIoRequest,
    pub set_buffer: FileInformationClass,
}

impl ServerDriveSetInformationRequest {
    const FIXED_PART_SIZE: usize = 4 /* FileInformationClass */ + 4 /* Length */ + 24 /* Padding */;

    fn decode(dev_io_req: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let file_information_class_level = FileInformationClassLevel::from(src.read_u32());

        // This field MUST contain one of the following values.
        match file_information_class_level {
            FileInformationClassLevel::FILE_BASIC_INFORMATION
            | FileInformationClassLevel::FILE_END_OF_FILE_INFORMATION
            | FileInformationClassLevel::FILE_DISPOSITION_INFORMATION
            | FileInformationClassLevel::FILE_RENAME_INFORMATION
            | FileInformationClassLevel::FILE_ALLOCATION_INFORMATION => {}
            _ => {
                return Err(invalid_message_err!(
                    "ServerDriveSetInformationRequest::decode",
                    "file_information_class_level",
                    "received invalid level"
                ))
            }
        };

        let length = cast_length!("ServerDriveSetInformationRequest", "length", src.read_u32())?;

        read_padding!(src, 24); // Padding

        let set_buffer = FileInformationClass::decode(file_information_class_level, length, src)?;

        Ok(Self {
            device_io_request: dev_io_req,
            set_buffer,
        })
    }
}

/// 2.4.13 FileEndOfFileInformation
///
/// [2.4.13]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/75241cca-3167-472f-8058-a52d77c6bb17
#[derive(Debug, Clone)]
pub struct FileEndOfFileInformation {
    pub end_of_file: i64,
}

impl FileEndOfFileInformation {
    const FIXED_PART_SIZE: usize = 8; // EndOfFile

    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let end_of_file = src.read_i64();
        Ok(Self { end_of_file })
    }

    fn size() -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [2.4.11] FileDispositionInformation
///
/// [2.4.11]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/12c3dd1c-14f6-4229-9d29-75fb2cb392f6
#[derive(Debug, Clone)]
pub struct FileDispositionInformation {
    pub delete_pending: u8,
}

impl FileDispositionInformation {
    const FIXED_PART_SIZE: usize = 1; // DeletePending

    fn decode(src: &mut ReadCursor<'_>, length: usize) -> PduResult<Self> {
        // https://github.com/FreeRDP/FreeRDP/blob/dfa231c0a55b005af775b833f92f6bcd30363d77/channels/drive/client/drive_file.c#L684-L692
        let delete_pending = if length != 0 {
            ensure_fixed_part_size!(in: src);
            src.read_u8()
        } else {
            1
        };
        Ok(Self { delete_pending })
    }

    fn size() -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [2.4.37] FileRenameInformation
///
/// [2.4.37]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/1d2673a8-8fb9-4868-920a-775ccaa30cf8
#[derive(Debug, Clone)]
pub struct FileRenameInformation {
    pub replace_if_exists: Boolean,
    /// `file_name` is the relative path to the new location of the file
    pub file_name: String,
}

impl FileRenameInformation {
    const FIXED_PART_SIZE: usize = 1 /* ReplaceIfExists */ + 1 /* RootDirectory */ + 4 /* FileNameLength */;

    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let replace_if_exists = Boolean::from(src.read_u8());
        let _ = src.read_u8(); // RootDirectory
        let file_name_length = cast_length!("FileRenameInformation", "file_name_length", src.read_u32())?;

        ensure_size!(in: src, size: file_name_length);
        let file_name = decode_string(src.read_slice(file_name_length), CharacterSet::Unicode, true)?;

        Ok(Self {
            replace_if_exists,
            file_name,
        })
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + encoded_str_len(&self.file_name, CharacterSet::Unicode, true)
    }
}

/// [2.4.4] FileAllocationInformation
///
/// [2.4.4]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/0201c69b-50db-412d-bab3-dd97aeede13b
#[derive(Debug, Clone)]
pub struct FileAllocationInformation {
    pub allocation_size: i64,
}

impl FileAllocationInformation {
    const FIXED_PART_SIZE: usize = 8; // AllocationSize

    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let allocation_size = src.read_i64();
        Ok(Self { allocation_size })
    }

    fn size() -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [2.2.3.4.9] Client Drive Set Information Response (DR_DRIVE_SET_INFORMATION_RSP)
///
/// [2.2.3.4.9]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/16b893d5-5d8b-49d1-8dcb-ee21e7612970
#[derive(Debug)]
pub struct ClientDriveSetInformationResponse {
    device_io_reply: DeviceIoResponse,
    /// This field MUST be equal to the Length field in the Server Drive Set Information Request (section 2.2.3.3.9).
    length: u32,
}

impl ClientDriveSetInformationResponse {
    const NAME: &'static str = "DR_DRIVE_SET_INFORMATION_RSP";

    pub fn new(req: &ServerDriveSetInformationRequest, io_status: NtStatus) -> PduResult<Self> {
        Ok(Self {
            device_io_reply: DeviceIoResponse::new(req.device_io_request.clone(), io_status),
            length: cast_length!("ClientDriveSetInformationResponse", "length", req.set_buffer.size())?,
        })
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.device_io_reply.encode(dst)?;
        dst.write_u32(self.length);
        Ok(())
    }

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn size(&self) -> usize {
        self.device_io_reply.size() // DeviceIoResponse
        + 4 // Length
    }
}

/// 2.2.3.3.12 Server Drive Lock Control Request (DR_DRIVE_LOCK_REQ)
///
/// [2.2.3.3.12]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/a96fe85c-620c-40ce-8858-a6bc38609b0a
#[derive(Debug, Clone)]
pub struct ServerDriveLockControlRequest {
    pub device_io_request: DeviceIoRequest,
}

impl ServerDriveLockControlRequest {
    fn decode(dev_io_req: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        // It's not quite clear why this is done this way, but it's what FreeRDP does:
        // https://github.com/FreeRDP/FreeRDP/blob/dfa231c0a55b005af775b833f92f6bcd30363d77/channels/drive/client/drive_main.c#L600
        ensure_size!(in: src, size: 4);
        let _ = src.read_u32();
        Ok(Self {
            device_io_request: dev_io_req,
        })
    }
}
